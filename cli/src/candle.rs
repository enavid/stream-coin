use colored::Colorize;

use crate::client::ApiClient;

pub async fn handle_backfill(
    client: &ApiClient,
    exchange: &str,
    pair: &str,
    interval: &str,
    from: &str,
    to: &str,
) -> Result<(), String> {
    let resp = client
        .candle_backfill(exchange, pair, interval, from, to)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{} Backfilled {} candles for {}:{} ({})",
        "✓".green(),
        resp.data.candles_written.to_string().cyan(),
        exchange.cyan(),
        pair.cyan(),
        interval
    );
    Ok(())
}
