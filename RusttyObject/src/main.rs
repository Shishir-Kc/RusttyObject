mod brand;
mod cli;
fn main() {
    brand::brand();
    println!("{}",cli::get_user_github_repo_url());
}
