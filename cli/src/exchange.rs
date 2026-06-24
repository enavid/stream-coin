use colored::Colorize;

use crate::client::ApiClient;

pub async fn handle_seed(client: &ApiClient, exchange: &str, quotes: &str) -> Result<(), String> {
    let resp = client
        .exchange_seed_from_assets(exchange, quotes)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{} Seeded {} pairs for {} from the asset catalog",
        "✓".green(),
        resp.data.pairs_seeded.to_string().cyan(),
        exchange.cyan()
    );
    Ok(())
}
