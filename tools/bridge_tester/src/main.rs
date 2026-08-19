use reprodeck_lib::*;
fn main() {
    match list_sessions() {
        Ok(v) => println!("list_sessions ok: {}", serde_json::to_string_pretty(&v).unwrap()),
        Err(e) => println!("list_sessions err: {}", e),
    }
}
