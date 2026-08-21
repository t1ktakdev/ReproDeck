use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(
    name = "reprodeck",
    version,
    about = "ReproDeck local debugging utilities"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Check local developer prerequisites without changing the system.
    Doctor,
    /// Inspect a local Git repository using the same core logic as the desktop app.
    Repo { path: PathBuf },
    /// Validate and inspect a .reprodeck capsule without importing it.
    Capsule { path: PathBuf },
}

fn probe(program: &str, args: &[&str]) -> Option<String> {
    #[cfg(windows)]
    let output = if program.eq_ignore_ascii_case("npm") {
        // npm is normally installed as npm.cmd on Windows. cmd.exe is used only
        // with these trusted static arguments; no user input is interpolated.
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c", "npm"]).args(args);
        command.output().ok()?
    } else {
        Command::new(program).args(args).output().ok()?
    };
    #[cfg(not(windows))]
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Some(
        if stdout.is_empty() { stderr } else { stdout }
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
    )
}

fn doctor() {
    for (name, program, args) in [
        ("Git", "git", &["--version"][..]),
        ("Node", "node", &["--version"][..]),
        ("npm", "npm", &["--version"][..]),
        ("Rust", "rustc", &["--version"][..]),
        ("Cargo", "cargo", &["--version"][..]),
        ("GitHub CLI", "gh", &["--version"][..]),
    ] {
        match probe(program, args) {
            Some(version) => println!("{name:<12} OK  {version}"),
            None => println!("{name:<12} --  not detected"),
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => {
            println!("ReproDeck CLI v{}", env!("CARGO_PKG_VERSION"));
            println!("Use `reprodeck --help` for local inspection commands.");
        }
        Some(Commands::Doctor) => doctor(),
        Some(Commands::Repo { path }) => {
            let info = reprodeck_core::repository::inspect_repository(&path)?;
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        Some(Commands::Capsule { path }) => {
            let summary = reprodeck_core::capsule::inspect_capsule(&path)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
    }
    Ok(())
}
