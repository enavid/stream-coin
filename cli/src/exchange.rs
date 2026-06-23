use colored::Colorize;

use crate::client::ApiClient;

pub async fn handle_seed(client: &ApiClient, exchange: &str, top: u32) -> Result<(), String> {
    let resp = client
        .exchange_seed_top_pairs(exchange, top)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{} Seeded {} top pairs for {}",
        "✓".green(),
        resp.data.pairs_seeded.to_string().cyan(),
        exchange.cyan()
    );
    Ok(())
}
