use crate::object::{build_config, config_path, read_config, scan_workspace, write_config};
use base64::Engine;
use reqwest::Client;
use serde_json::json;
use std::env;
use std::io::{self, Write};
use std::path::Path;

pub fn init(repo: &str, branch: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = path.canonicalize()?;
    let repository = normalize_repo(repo)?;
    let config = build_config(&workspace, repository.clone(), branch.to_string())?;
    write_config(&workspace, &config)?;
    println!("Initialized RusttyObject in {}", workspace.display());
    println!("Repository: {repository} ({branch})");
    println!(
        "Indexed {} objects in {}",
        config.objects.len(),
        config_path(&workspace).display()
    );
    Ok(())
}

pub fn index(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = path.canonicalize()?;
    let mut config = read_config(&workspace)?;
    config.objects = scan_workspace(&workspace)?;
    config.generated_at = current_timestamp();
    write_config(&workspace, &config)?;
    println!(
        "Indexed {} objects in {}",
        config.objects.len(),
        config_path(&workspace).display()
    );
    Ok(())
}

pub async fn push(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = path.canonicalize()?;
    let mut config = read_config(&workspace)?;
    config.objects = scan_workspace(&workspace)?;
    config.generated_at = current_timestamp();
    write_config(&workspace, &config)?;

    let token =
        env::var("GITHUB_TOKEN").map_err(|_| "GITHUB_TOKEN is required for `rusttyobject push`")?;
    let (owner, repo) = config
        .repository
        .split_once('/')
        .ok_or("repository must be owner/name")?;
    let client = Client::builder().user_agent("RusttyObject/0.1").build()?;

    for object in &config.objects {
        let bytes = std::fs::read(workspace.join(&object.path))?;
        let encoded_path = object
            .path
            .split('/')
            .map(|part| urlencoding::encode(part).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let endpoint =
            format!("https://api.github.com/repos/{owner}/{repo}/contents/{encoded_path}");
        let existing = client
            .get(format!(
                "{endpoint}?ref={}",
                urlencoding::encode(&config.branch)
            ))
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;
        let sha = if existing.status().is_success() {
            existing
                .json::<serde_json::Value>()
                .await?
                .get("sha")
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
        } else if existing.status() == reqwest::StatusCode::NOT_FOUND {
            None
        } else {
            return Err(format!(
                "GitHub rejected lookup for {}: {}",
                object.path,
                existing.status()
            )
            .into());
        };

        let mut body = json!({
            "message": format!("Sync {} via RusttyObject", object.path),
            "content": base64::engine::general_purpose::STANDARD.encode(bytes),
            "branch": config.branch,
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        let response = client
            .put(&endpoint)
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(format!(
                "GitHub rejected {}: {}",
                object.path,
                response.text().await?
            )
            .into());
        }
        println!("Synced {}", object.path);
    }
    println!(
        "Pushed {} objects to {}",
        config.objects.len(),
        config.repository
    );
    Ok(())
}

pub fn show_options() {
    println!(
        "\nCommands:\n  rusttyobject server       Start the API\n  rusttyobject init -r ... Create config.rustyobject\n  rusttyobject index        Rebuild the index\n  rusttyobject push         Sync files to GitHub\n"
    );
}

pub fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    print!(":> ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    match input.trim() {
        "v" => println!("RusttyObject {}", env!("CARGO_PKG_VERSION")),
        "i" => println!("Use `rusttyobject init --repo owner/name` to create an index."),
        "r" => println!("Validation is performed when `config.rustyobject` is read."),
        _ => println!("Unknown command. Try `rusttyobject --help`."),
    }
    Ok(())
}

fn normalize_repo(repo: &str) -> Result<String, Box<dyn std::error::Error>> {
    let value = repo.trim().trim_end_matches('/').trim_end_matches(".git");
    let value = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("http://github.com/"))
        .unwrap_or(value);
    let parts = value
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err("repository must be owner/name or a GitHub repository URL".into());
    }
    Ok(format!("{}/{}", parts[0], parts[1]))
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
