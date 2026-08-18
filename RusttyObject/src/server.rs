use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

const GITHUB_API: &str = "https://api.github.com";

#[derive(Clone)]
pub struct AppState {
    client: Client,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    oauth_states: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone)]
struct Session {
    token: String,
    user: GithubUser,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "Sign in with GitHub to continue".into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn upstream_network(message: String) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: format!("Could not reach GitHub: {message}"),
        }
    }

    fn upstream(response: reqwest::Response) -> impl std::future::Future<Output = Self> {
        async move {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let message = match response.text().await {
                Ok(body) => serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string)
                            .or_else(|| {
                                value.get("errors").and_then(|errors| {
                                    errors.as_array().map(|items| {
                                        items
                                            .iter()
                                            .filter_map(|item| {
                                                item.get("message")
                                                    .and_then(serde_json::Value::as_str)
                                            })
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                })
                            })
                    })
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| {
                        if body.trim().is_empty() {
                            "GitHub returned an error without details".to_string()
                        } else {
                            format!("GitHub returned an error: {}", body.trim())
                        }
                    }),
                Err(_) => "GitHub returned an error without details".to_string(),
            };
            Self { status, message }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubUser {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubProfile {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: String,
    pub html_url: String,
    pub bio: Option<String>,
    pub public_repos: u64,
    pub followers: u64,
    pub following: u64,
}

#[derive(Debug, Serialize)]
struct ProfileResponse {
    user: GithubProfile,
    stats: ProfileStats,
    activities: Vec<ActivityResponse>,
}

#[derive(Debug, Serialize)]
struct ProfileStats {
    total_contributions: u64,
    repositories: u64,
    public_repositories: u64,
    followers: u64,
    following: u64,
}

#[derive(Debug, Deserialize)]
struct GithubEvent {
    id: String,
    #[serde(rename = "type")]
    event_type: Option<String>,
    repo: Option<GithubEventRepository>,
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubEventRepository {
    name: String,
}

#[derive(Debug, Serialize)]
struct ActivityResponse {
    id: String,
    kind: String,
    repository: Option<String>,
    created_at: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubNotification {
    id: String,
    reason: Option<String>,
    unread: bool,
    updated_at: String,
    subject: GithubNotificationSubject,
    repository: GithubNotificationRepository,
}

#[derive(Debug, Deserialize)]
struct GithubNotificationSubject {
    title: String,
    #[serde(rename = "type")]
    subject_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubNotificationRepository {
    full_name: String,
    html_url: String,
}

#[derive(Debug, Serialize)]
struct NotificationResponse {
    id: String,
    title: String,
    kind: Option<String>,
    reason: Option<String>,
    repository: String,
    url: String,
    updated_at: String,
    unread: bool,
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse {
    data: Option<GraphqlData>,
}

#[derive(Debug, Deserialize)]
struct GraphqlData {
    user: Option<GraphqlUser>,
}

#[derive(Debug, Deserialize)]
struct GraphqlUser {
    #[serde(rename = "contributionsCollection")]
    contributions_collection: Option<GraphqlContributions>,
    repositories: Option<GraphqlCount>,
}

#[derive(Debug, Deserialize)]
struct GraphqlContributions {
    #[serde(rename = "contributionCalendar")]
    contribution_calendar: GraphqlContributionCalendar,
}

#[derive(Debug, Deserialize)]
struct GraphqlContributionCalendar {
    #[serde(rename = "totalContributions")]
    total_contributions: u64,
}

#[derive(Debug, Deserialize)]
struct GraphqlCount {
    #[serde(rename = "totalCount")]
    total_count: u64,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubRepository {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub html_url: String,
    pub updated_at: Option<String>,
    pub owner: GithubOwner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubOwner {
    pub login: String,
}

#[derive(Debug, Deserialize)]
struct GithubTreeResponse {
    tree: Vec<GithubTreeEntry>,
}

#[derive(Debug, Deserialize)]
struct GithubTreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    size: Option<u64>,
    sha: Option<String>,
}

#[derive(Debug, Serialize)]
struct ObjectResponse {
    path: String,
    name: String,
    bucket: String,
    content_type: String,
    size: u64,
    sha: String,
    updated: String,
    raw_url: String,
    download_url: String,
}

#[derive(Debug, Serialize)]
struct ObjectsResponse {
    repository: GithubRepository,
    branch: String,
    objects: Vec<ObjectResponse>,
    buckets: Vec<BucketResponse>,
}

#[derive(Debug, Serialize)]
struct BucketResponse {
    name: String,
    objects: usize,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct BranchQuery {
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileQuery {
    path: String,
    branch: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionResponse {
    authenticated: bool,
    user: Option<GithubUser>,
}

#[derive(Debug, Deserialize)]
struct GithubContentResponse {
    sha: Option<String>,
}

pub async fn run(bind: &str) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        client: Client::builder().user_agent("RusttyObject/0.1").build()?,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        oauth_states: Arc::new(Mutex::new(HashSet::new())),
    };

    let frontend_url =
        env::var("FRONTEND_URL").unwrap_or_else(|_| "http://localhost:5173".to_string());
    let allowed_origin = HeaderValue::from_str(&frontend_url)?;
    let app = Router::new()
        .route("/health", get(health))
        .route("/auth/github", get(begin_github_auth))
        .route("/auth/github/callback", get(github_callback))
        .route("/api/session", get(session))
        .route("/api/auth/logout", post(logout))
        .route("/api/repositories", get(repositories))
        .route("/api/profile", get(profile))
        .route("/api/notifications", get(notifications))
        .route("/api/repositories/{owner}/{repo}/objects", get(objects))
        .route(
            "/api/repositories/{owner}/{repo}/buckets",
            post(create_bucket),
        )
        .route("/api/repositories/{owner}/{repo}/file", get(file_preview))
        .route("/api/repositories/{owner}/{repo}/files", post(upload_file))
        .layer(
            CorsLayer::new()
                .allow_origin(allowed_origin)
                .allow_credentials(true)
                .allow_headers([header::CONTENT_TYPE])
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST]),
        )
        .with_state(state);

    let listener = TcpListener::bind(bind).await?;
    println!("RusttyObject API listening on http://{bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "rusttyobject-api" }))
}

async fn begin_github_auth(State(state): State<AppState>) -> Result<Redirect, ApiError> {
    let client_id = env::var("GITHUB_CLIENT_ID")
        .map_err(|_| ApiError::bad_request("GITHUB_CLIENT_ID is not configured"))?;
    let state_token = Uuid::new_v4().to_string();
    state.oauth_states.lock().await.insert(state_token.clone());
    let redirect_uri = callback_url();
    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope=repo%20read:user%20user:email%20notifications&state={}",
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&state_token)
    );
    Ok(Redirect::temporary(&url))
}

async fn github_callback(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let state_token = params.get("state").ok_or_else(ApiError::unauthorized)?;
    if !state.oauth_states.lock().await.remove(state_token) {
        return Err(ApiError::unauthorized());
    }
    let code = params
        .get("code")
        .ok_or_else(|| ApiError::bad_request("GitHub did not return an authorization code"))?;
    let client_id = env::var("GITHUB_CLIENT_ID")
        .map_err(|_| ApiError::bad_request("GITHUB_CLIENT_ID is not configured"))?;
    let client_secret = env::var("GITHUB_CLIENT_SECRET")
        .map_err(|_| ApiError::bad_request("GITHUB_CLIENT_SECRET is not configured"))?;

    let token_response = state
        .client
        .post("https://github.com/login/oauth/access_token")
        .header(header::ACCEPT, "application/json")
        .json(&json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": code,
            "redirect_uri": callback_url(),
        }))
        .send()
        .await
        .map_err(|error| ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
    if !token_response.status().is_success() {
        return Err(ApiError::upstream(token_response).await);
    }
    let token: OAuthTokenResponse = token_response.json().await.map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    let access_token = token.access_token.ok_or_else(|| {
        ApiError::bad_request(
            token
                .error_description
                .or(token.error)
                .unwrap_or_else(|| "GitHub did not issue an access token".into()),
        )
    })?;
    let user = github_get::<GithubUser>(&state.client, &access_token, "/user").await?;
    let session_id = Uuid::new_v4().to_string();
    state.sessions.lock().await.insert(
        session_id.clone(),
        Session {
            token: access_token,
            user,
        },
    );

    let frontend_url =
        env::var("FRONTEND_URL").unwrap_or_else(|_| "http://localhost:5173".to_string());
    let mut response = Redirect::temporary(&frontend_url).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "rusty_session={session_id}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800"
        ))
        .map_err(|error| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        })?,
    );
    Ok(response)
}

async fn session(headers: HeaderMap, State(state): State<AppState>) -> Json<SessionResponse> {
    let user = session_from_headers(&headers, &state)
        .await
        .ok()
        .map(|session| session.user);
    Json(SessionResponse {
        authenticated: user.is_some(),
        user,
    })
}

async fn logout(headers: HeaderMap, State(state): State<AppState>) -> impl IntoResponse {
    if let Some(session_id) = cookie_value(&headers, "rusty_session") {
        state.sessions.lock().await.remove(&session_id);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_static("rusty_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"),
    );
    response
}

async fn repositories(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<GithubRepository>>, ApiError> {
    let session = session_from_headers(&headers, &state).await?;
    let repos = github_get::<Vec<GithubRepository>>(
        &state.client,
        &session.token,
        "/user/repos?per_page=100&sort=updated",
    )
    .await?;
    Ok(Json(repos))
}

async fn profile(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<ProfileResponse>, ApiError> {
    let session = session_from_headers(&headers, &state).await?;
    let user = github_get::<GithubProfile>(&state.client, &session.token, "/user").await?;
    let total_contributions =
        github_contributions(&state.client, &session.token, &user.login).await?;
    let events = github_get::<Vec<GithubEvent>>(
        &state.client,
        &session.token,
        &format!(
            "/users/{}/events?per_page=20",
            urlencoding::encode(&user.login)
        ),
    )
    .await
    .unwrap_or_default();
    let activities = events
        .into_iter()
        .map(|event| {
            let repository = event.repo.map(|repo| repo.name);
            ActivityResponse {
                id: event.id,
                kind: event
                    .event_type
                    .unwrap_or_else(|| "GitHub activity".to_string())
                    .trim_end_matches("Event")
                    .to_string(),
                url: repository
                    .as_ref()
                    .map(|repo| format!("https://github.com/{repo}")),
                repository,
                created_at: event.created_at,
            }
        })
        .collect();
    let stats = ProfileStats {
        total_contributions,
        repositories: github_repository_count(&state.client, &session.token, &user.login).await?,
        public_repositories: user.public_repos,
        followers: user.followers,
        following: user.following,
    };
    Ok(Json(ProfileResponse {
        user,
        stats,
        activities,
    }))
}

async fn notifications(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Json<Vec<NotificationResponse>>, ApiError> {
    let session = session_from_headers(&headers, &state).await?;
    let github_notifications = github_get::<Vec<GithubNotification>>(
        &state.client,
        &session.token,
        "/notifications?all=false&participating=false&per_page=20",
    )
    .await?;
    Ok(Json(
        github_notifications
            .into_iter()
            .map(|notification| NotificationResponse {
                id: notification.id,
                title: notification.subject.title,
                kind: notification.subject.subject_type,
                reason: notification.reason,
                repository: notification.repository.full_name,
                url: notification.repository.html_url,
                updated_at: notification.updated_at,
                unread: notification.unread,
            })
            .collect(),
    ))
}

async fn objects(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<BranchQuery>,
) -> Result<Json<ObjectsResponse>, ApiError> {
    let session = session_from_headers(&headers, &state).await?;
    let repository = github_get::<GithubRepository>(
        &state.client,
        &session.token,
        &format!("/repos/{owner}/{repo}"),
    )
    .await?;
    let branch = query
        .branch
        .unwrap_or_else(|| repository.default_branch.clone());
    let tree_path = format!(
        "/repos/{owner}/{repo}/git/trees/{}?recursive=1",
        urlencoding::encode(&branch)
    );
    let tree = github_get::<GithubTreeResponse>(&state.client, &session.token, &tree_path).await?;
    let mut objects = Vec::new();
    let mut buckets: HashMap<String, BucketResponse> = HashMap::new();

    for entry in tree
        .tree
        .into_iter()
        .filter(|entry| entry.entry_type == "blob")
    {
        if entry.path.ends_with("/.rustyobject") {
            if let Some(bucket) = entry.path.strip_suffix("/.rustyobject") {
                buckets.entry(bucket.to_string()).or_insert(BucketResponse {
                    name: bucket.to_string(),
                    objects: 0,
                    size: 0,
                });
            }
            continue;
        }
        if entry.path == ".rustyobject" || entry.path == "config.rustyobject" {
            continue;
        }
        let bucket = entry.path.split('/').next().unwrap_or("root").to_string();
        let name = entry
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&entry.path)
            .to_string();
        let size = entry.size.unwrap_or(0);
        let bucket_entry = buckets.entry(bucket.clone()).or_insert(BucketResponse {
            name: bucket.clone(),
            objects: 0,
            size: 0,
        });
        bucket_entry.objects += 1;
        bucket_entry.size += size;
        objects.push(ObjectResponse {
            content_type: mime_guess::from_path(&entry.path)
                .first_or_octet_stream()
                .essence_str()
                .to_string(),
            raw_url: format!(
                "https://raw.githubusercontent.com/{owner}/{repo}/{}/{}",
                urlencoding::encode(&branch),
                entry
                    .path
                    .split('/')
                    .map(|part| urlencoding::encode(part).into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            ),
            download_url: format!(
                "https://raw.githubusercontent.com/{owner}/{repo}/{}/{}",
                urlencoding::encode(&branch),
                entry
                    .path
                    .split('/')
                    .map(|part| urlencoding::encode(part).into_owned())
                    .collect::<Vec<_>>()
                    .join("/")
            ),
            path: entry.path,
            name,
            bucket,
            size,
            sha: entry.sha.unwrap_or_default(),
            updated: "synced".to_string(),
        });
    }
    objects.sort_by(|left, right| left.path.cmp(&right.path));
    let mut buckets = buckets.into_values().collect::<Vec<_>>();
    buckets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Json(ObjectsResponse {
        repository,
        branch,
        objects,
        buckets,
    }))
}

#[derive(Debug, Deserialize)]
struct CreateBucketRequest {
    name: String,
    branch: Option<String>,
    message: Option<String>,
}

async fn create_bucket(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Json(request): Json<CreateBucketRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = session_from_headers(&headers, &state).await?;
    let bucket = validate_bucket_name(request.name)?;
    let repository = github_get::<GithubRepository>(
        &state.client,
        &session.token,
        &format!("/repos/{owner}/{repo}"),
    )
    .await?;
    let branch = request
        .branch
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(repository.default_branch);
    let marker_path = format!("{bucket}/.rustyobject");
    let encoded_path = marker_path
        .split('/')
        .map(|part| urlencoding::encode(part).into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let endpoint = format!(
        "/repos/{owner}/{repo}/contents/{encoded_path}?ref={}",
        urlencoding::encode(&branch)
    );
    let existing = github_request(
        &state.client,
        &session.token,
        reqwest::Method::GET,
        &endpoint,
    )
    .send()
    .await
    .map_err(|error| ApiError::upstream_network(error.to_string()))?;
    if existing.status().is_success() {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: format!("Bucket '{bucket}' already exists"),
        });
    }
    if existing.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(ApiError::upstream(existing).await);
    }

    let payload = json!({
        "message": request.message.unwrap_or_else(|| format!("Create bucket {bucket} via RusttyObject")),
        "content": base64::engine::general_purpose::STANDARD.encode(b"RusttyObject bucket marker\n"),
        "branch": branch,
    });
    let response = github_request(
        &state.client,
        &session.token,
        reqwest::Method::PUT,
        &format!("/repos/{owner}/{repo}/contents/{encoded_path}"),
    )
    .json(&payload)
    .send()
    .await
    .map_err(|error| ApiError::upstream_network(error.to_string()))?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(response).await);
    }
    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|error| ApiError::upstream_network(error.to_string()))?;
    Ok(Json(json!({
        "ok": true,
        "name": bucket,
        "branch": branch,
        "commit": result.get("commit").and_then(|commit| commit.get("sha")).cloned(),
    })))
}

async fn file_preview(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<FileQuery>,
) -> Result<Response, ApiError> {
    let session = session_from_headers(&headers, &state).await?;
    let branch = match query.branch.filter(|branch| !branch.trim().is_empty()) {
        Some(branch) => branch,
        None => {
            github_get::<GithubRepository>(
                &state.client,
                &session.token,
                &format!("/repos/{owner}/{repo}"),
            )
            .await?
            .default_branch
        }
    };
    let path = validate_path(query.path)?;
    let encoded_path = path
        .split('/')
        .map(|part| urlencoding::encode(part).into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let endpoint = format!(
        "/repos/{owner}/{repo}/contents/{encoded_path}?ref={}",
        urlencoding::encode(&branch)
    );
    let upstream = github_request(
        &state.client,
        &session.token,
        reqwest::Method::GET,
        &endpoint,
    )
    .header(header::ACCEPT, "application/vnd.github.raw+json")
    .send()
    .await
    .map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    if !upstream.status().is_success() {
        return Err(ApiError::upstream(upstream).await);
    }

    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .or_else(|| {
            HeaderValue::from_str(
                mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .essence_str(),
            )
            .ok()
        })
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
    let content = upstream.bytes().await.map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, "inline")
        .header(header::CACHE_CONTROL, "private, max-age=60")
        .header(header::CONTENT_LENGTH, content.len())
        .body(Body::from(content))
        .map_err(|error| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        })?;
    Ok(response)
}

async fn upload_file(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = session_from_headers(&headers, &state).await?;
    let mut bytes = None;
    let mut file_name = None;
    let mut bucket = "root".to_string();
    let mut path = None;
    let mut branch = None;
    let mut commit_message = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?
    {
        let field_name = field.name().unwrap_or_default().to_string();
        match field_name.as_str() {
            "file" => {
                file_name = field.file_name().map(ToString::to_string);
                let content = field
                    .bytes()
                    .await
                    .map_err(|error| ApiError::bad_request(error.to_string()))?;
                if content.len() > 100 * 1024 * 1024 {
                    return Err(ApiError::bad_request(
                        "GitHub Contents API uploads are limited to 100 MB",
                    ));
                }
                bytes = Some(content);
            }
            "bucket" => {
                bucket = field
                    .text()
                    .await
                    .map_err(|error| ApiError::bad_request(error.to_string()))?
            }
            "path" => {
                path = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| ApiError::bad_request(error.to_string()))?,
                )
            }
            "branch" => {
                branch = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| ApiError::bad_request(error.to_string()))?,
                )
            }
            "message" => {
                commit_message = Some(
                    field
                        .text()
                        .await
                        .map_err(|error| ApiError::bad_request(error.to_string()))?,
                )
            }
            _ => {}
        }
    }

    let bytes = bytes.ok_or_else(|| ApiError::bad_request("A file is required"))?;
    let file_name =
        file_name.ok_or_else(|| ApiError::bad_request("The uploaded file has no name"))?;
    let requested_path = path.unwrap_or_else(|| {
        if bucket == "root" {
            file_name.clone()
        } else {
            format!("{bucket}/{file_name}")
        }
    });
    let file_path = validate_path(requested_path)?;
    let repository = github_get::<GithubRepository>(
        &state.client,
        &session.token,
        &format!("/repos/{owner}/{repo}"),
    )
    .await?;
    let branch = branch.unwrap_or(repository.default_branch);
    let content_path = file_path
        .split('/')
        .map(|part| urlencoding::encode(part).into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let existing_url = format!(
        "/repos/{owner}/{repo}/contents/{content_path}?ref={}",
        urlencoding::encode(&branch)
    );
    let existing_response = github_request(
        &state.client,
        &session.token,
        reqwest::Method::GET,
        &existing_url,
    )
    .send()
    .await
    .map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    let existing_sha = if existing_response.status().is_success() {
        existing_response
            .json::<GithubContentResponse>()
            .await
            .ok()
            .and_then(|content| content.sha)
    } else if existing_response.status() == reqwest::StatusCode::NOT_FOUND {
        None
    } else {
        return Err(ApiError::upstream(existing_response).await);
    };

    let mut payload = json!({
        "message": commit_message.unwrap_or_else(|| format!("Upload {file_path} via RusttyObject")),
        "content": base64::engine::general_purpose::STANDARD.encode(bytes),
        "branch": branch,
    });
    if let Some(sha) = existing_sha {
        payload["sha"] = json!(sha);
    }
    let response = github_request(
        &state.client,
        &session.token,
        reqwest::Method::PUT,
        &format!("/repos/{owner}/{repo}/contents/{content_path}"),
    )
    .json(&payload)
    .send()
    .await
    .map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(response).await);
    }
    let result: serde_json::Value = response.json().await.map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    Ok(Json(
        json!({ "ok": true, "path": file_path, "commit": result.get("commit").and_then(|commit| commit.get("sha")).cloned() }),
    ))
}

async fn session_from_headers(headers: &HeaderMap, state: &AppState) -> Result<Session, ApiError> {
    let session_id = cookie_value(headers, "rusty_session").ok_or_else(ApiError::unauthorized)?;
    state
        .sessions
        .lock()
        .await
        .get(&session_id)
        .cloned()
        .ok_or_else(ApiError::unauthorized)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|cookie| {
            let (key, value) = cookie.trim().split_once('=')?;
            (key == name).then(|| value.to_string())
        })
}

fn callback_url() -> String {
    env::var("GITHUB_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:8787/auth/github/callback".to_string())
}

fn validate_path(path: String) -> Result<String, ApiError> {
    let normalized = path.trim().trim_matches('/').replace('\\', "/");
    if normalized.is_empty()
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ApiError::bad_request(
            "File paths must be relative and cannot contain . or .. segments",
        ));
    }
    Ok(normalized)
}

fn validate_bucket_name(name: String) -> Result<String, ApiError> {
    let normalized = name.trim().trim_matches('/').replace('\\', "/");
    if normalized.is_empty()
        || normalized.len() > 100
        || normalized.contains('/')
        || normalized == "."
        || normalized == ".."
        || normalized == "root"
        || normalized.starts_with('.')
    {
        return Err(ApiError::bad_request(
            "Bucket names must be one non-empty path segment, up to 100 characters, and cannot start with a dot or use 'root'",
        ));
    }
    if !normalized
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ' '))
    {
        return Err(ApiError::bad_request(
            "Bucket names may contain only letters, numbers, spaces, hyphens, and underscores",
        ));
    }
    Ok(normalized)
}

async fn github_repository_count(
    client: &Client,
    token: &str,
    login: &str,
) -> Result<u64, ApiError> {
    let query = r#"query($login: String!) { user(login: $login) { repositories(first: 1, ownerAffiliations: OWNER) { totalCount } } }"#;
    let response = client
        .post("https://api.github.com/graphql")
        .bearer_auth(token)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&json!({ "query": query, "variables": { "login": login } }))
        .send()
        .await
        .map_err(|error| ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(response).await);
    }
    let payload: GraphqlResponse = response.json().await.map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    Ok(payload
        .data
        .and_then(|data| data.user)
        .and_then(|user| user.repositories)
        .map(|repositories| repositories.total_count)
        .unwrap_or_default())
}

async fn github_contributions(client: &Client, token: &str, login: &str) -> Result<u64, ApiError> {
    let query = r#"query($login: String!) { user(login: $login) { contributionsCollection { contributionCalendar { totalContributions } } } }"#;
    let response = client
        .post("https://api.github.com/graphql")
        .bearer_auth(token)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&json!({ "query": query, "variables": { "login": login } }))
        .send()
        .await
        .map_err(|error| ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(response).await);
    }
    let payload: GraphqlResponse = response.json().await.map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })?;
    Ok(payload
        .data
        .and_then(|data| data.user)
        .and_then(|user| user.contributions_collection)
        .map(|contributions| contributions.contribution_calendar.total_contributions)
        .unwrap_or_default())
}

fn github_request(
    client: &Client,
    token: &str,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    client
        .request(method, format!("{GITHUB_API}{path}"))
        .bearer_auth(token)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
}

#[cfg(test)]
mod tests {
    use super::{validate_bucket_name, validate_path};

    #[test]
    fn bucket_names_accept_safe_single_segments() {
        assert_eq!(
            validate_bucket_name(" product-images ".into()).unwrap(),
            "product-images"
        );
        assert_eq!(
            validate_bucket_name("user_uploads".into()).unwrap(),
            "user_uploads"
        );
    }

    #[test]
    fn bucket_names_reject_paths_and_reserved_names() {
        for name in ["", "root", ".hidden", "one/two", "../bucket", "a?b"] {
            assert!(
                validate_bucket_name(name.into()).is_err(),
                "accepted invalid bucket: {name}"
            );
        }
    }

    #[test]
    fn paths_reject_traversal() {
        assert!(validate_path("bucket/../secret".into()).is_err());
        assert_eq!(
            validate_path("bucket/file.txt".into()).unwrap(),
            "bucket/file.txt"
        );
    }
}

async fn github_get<T: for<'de> Deserialize<'de>>(
    client: &Client,
    token: &str,
    path: &str,
) -> Result<T, ApiError> {
    let response = github_request(client, token, reqwest::Method::GET, path)
        .send()
        .await
        .map_err(|error| ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        })?;
    if !response.status().is_success() {
        return Err(ApiError::upstream(response).await);
    }
    response.json().await.map_err(|error| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: error.to_string(),
    })
}
