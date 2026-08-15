use std::io;
use std::io::Write;

pub fn get_user_github_repo_url()-> String{
   print!("Provide your Github repo url => ");
   io::stdout().flush().expect("Went wrong when Flushing"); 
   let mut url = String::new(); 
    io::stdin()
    .read_line(&mut url)
    .expect("Something went wrong :( ");
    
    url.trim().to_string()
}
