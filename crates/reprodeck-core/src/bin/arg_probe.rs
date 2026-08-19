use std::env;
use std::io::{self, Write};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    // Print argv entries one per line prefixed by ARG:<index>:
    for (i, a) in args.iter().enumerate() {
        println!("ARG:{}:{}", i, a);
    }

    // Print CWD
    if let Ok(cwd) = env::current_dir() {
        println!("CWD={}", cwd.to_string_lossy());
    } else {
        println!("CWD=");
    }

    // PROBE_KEYS env contains comma-separated env var names to report
    if let Ok(keys) = env::var("PROBE_KEYS") {
        for k in keys.split(',') {
            let ktrim = k.trim();
            if ktrim.is_empty() {
                continue;
            }
            match env::var(ktrim) {
                Ok(v) => println!("ENV:{}={}", ktrim, v),
                Err(_) => println!("ENV:{}=<MISSING>", ktrim),
            }
        }
    }

    // flush stdout
    io::stdout().flush()?;
    Ok(())
}
