use reprodeck_lib::list_sessions_service;

fn main() {
    match list_sessions_service() {
        Ok(sessions) => println!(
            "list_sessions_service ok: {}",
            serde_json::to_string_pretty(&sessions).expect("serialize sessions")
        ),
        Err(error) => {
            eprintln!("list_sessions_service error: {error}");
            std::process::exit(1);
        }
    }
}
