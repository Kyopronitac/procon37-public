#[tokio::main]
async fn main() -> anyhow::Result<()> {
    procon37_common::cli::run(procon37_common::runner::Mode::Game).await
}
