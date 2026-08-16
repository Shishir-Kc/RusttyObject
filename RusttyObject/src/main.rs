mod brand;
mod cli;
fn main() {
    brand::brand();
    // println!("{}",cli::get_user_github_repo_url());
    println!("Home Dir => {}", std::env::var("HOME").unwrap());
    cli::show_options();
    cli::run_cli();
}
