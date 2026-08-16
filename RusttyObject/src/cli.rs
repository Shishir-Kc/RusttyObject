use std::io;
use std::io::Write;

pub fn get_user_github_repo_url()-> String{
  /* 
        This function will read the input provided by the user and returns it 
   */ 
    print!("Provide your Github repo url => ");
   io::stdout().flush().expect("Went wrong when Flushing"); 
   let mut url = String::new(); 
    io::stdin()
    .read_line(&mut url)
    .expect("Something went wrong :( "); 
    url.trim().to_string()
}

pub fn show_options(){
   /*
        This will show all the available options for Rustty Objects
    */

    println!("
1) Show Current Version -> v 
2) Create Index -> i
3) Validate Rustty Object -> r 
    ");
    
}

pub fn run_cli(){
    let mut user_input = String::new();
    print!(":> ");
   io::stdout().flush().expect("Flushing went wrong : ( ");
    io::stdin().read_line(&mut user_input).expect("Something went wrong");
    println!("{}",user_input);
    

    
}
