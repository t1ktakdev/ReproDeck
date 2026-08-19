#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    println!("ReproDeck CLI v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
