use colored::Colorize;

use crate::cli::client::ApiClient;

pub async fn handle_start(client: &ApiClient, exchange: &str, symbol: &str) -> Result<(), String> {
    let resp = client.ticker_start(exchange, symbol).await?;
    if resp["success"] == true {
        println!(
            "{} Ticker started: {}:{}",
            "✓".green(),
            exchange.cyan(),
            symbol.cyan()
        );
    } else {
        let msg = resp["message"].as_str().unwrap_or("Unknown error");
        return Err(msg.to_string());
    }
    Ok(())
}

pub async fn handle_stop(client: &ApiClient, exchange: &str, symbol: &str) -> Result<(), String> {
    let resp = client.ticker_stop(exchange, symbol).await?;
    if resp["success"] == true {
        println!(
            "{} Ticker stopped: {}:{}",
            "✓".green(),
            exchange.cyan(),
            symbol.cyan()
        );
    } else {
        let msg = resp["message"].as_str().unwrap_or("Unknown error");
        return Err(msg.to_string());
    }
    Ok(())
}

pub async fn handle_list(client: &ApiClient) -> Result<(), String> {
    let resp = client.ticker_list().await?;
    let tickers = resp["data"]["tickers"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    if tickers.is_empty() {
        println!("{}", "No active tickers".dimmed());
        return Ok(());
    }

    println!("{}", "Active tickers:".bold());
    for t in &tickers {
        println!(
            "  {} {}:{}",
            "•".cyan(),
            t["exchange"].as_str().unwrap_or("?"),
            t["symbol"].as_str().unwrap_or("?"),
        );
    }
    Ok(())
}
