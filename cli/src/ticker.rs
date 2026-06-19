use colored::Colorize;

use crate::client::ApiClient;

pub async fn handle_start(client: &ApiClient, exchange: &str, symbol: &str) -> Result<(), String> {
    client
        .ticker_start(exchange, symbol)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{} Ticker started: {}:{}",
        "✓".green(),
        exchange.cyan(),
        symbol.cyan()
    );
    Ok(())
}

pub async fn handle_stop(client: &ApiClient, exchange: &str, symbol: &str) -> Result<(), String> {
    client
        .ticker_stop(exchange, symbol)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "{} Ticker stopped: {}:{}",
        "✓".green(),
        exchange.cyan(),
        symbol.cyan()
    );
    Ok(())
}

pub async fn handle_list(client: &ApiClient) -> Result<(), String> {
    let resp = client.ticker_list().await.map_err(|e| e.to_string())?;
    let tickers = resp.data.tickers;

    if tickers.is_empty() {
        println!("{}", "No active tickers".dimmed());
        return Ok(());
    }

    println!("{}", "Active tickers:".bold());
    for t in &tickers {
        println!("  {} {}:{}", "•".cyan(), t.exchange, t.pair);
    }
    Ok(())
}
