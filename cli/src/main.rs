use clap::{Parser, Subcommand};
use colored::Colorize;

mod auth;
mod candle;
mod client;
mod config;
mod exchange;
mod response;
mod ticker;

use client::ApiClient;
use config::Config;

#[derive(Parser)]
#[command(
    name = "sc",
    about = "stream-coin CLI — control your arbitrage engine",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage authentication
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    /// Control price tickers
    Ticker {
        #[command(subcommand)]
        command: TickerCommands,
    },
    /// Configure CLI settings
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Manage historical candle data
    Candle {
        #[command(subcommand)]
        command: CandleCommands,
    },
    /// Manage exchange registry
    Exchange {
        #[command(subcommand)]
        command: ExchangeCommands,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Open browser and save API token
    Login,
    /// Clear stored token
    Logout,
    /// Show current authentication status
    Status,
}

#[derive(Subcommand)]
enum TickerCommands {
    /// Start streaming a trading pair
    Start { exchange: String, symbol: String },
    /// Stop streaming a trading pair
    Stop { exchange: String, symbol: String },
    /// List all active tickers
    List,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set the API server URL
    SetUrl { url: String },
    /// Show current configuration
    Show,
}

#[derive(Subcommand)]
enum CandleCommands {
    /// Fetch historical klines from the exchange and persist them
    Backfill {
        exchange: String,
        /// Trading pair in "BASE/QUOTE" form, e.g. "BTC/USDT"
        pair: String,
        /// Candle interval: 1m, 5m, 15m, or 1h
        interval: String,
        /// Start of the range, RFC3339 (e.g. "2026-01-01T00:00:00Z")
        #[arg(long)]
        from: String,
        /// End of the range, RFC3339 (e.g. "2026-01-02T00:00:00Z")
        #[arg(long)]
        to: String,
    },
}

#[derive(Subcommand)]
enum ExchangeCommands {
    /// Seed trading pairs for an exchange from the canonical asset catalog
    Seed {
        exchange: String,
        /// Comma-separated quote currencies to pair every asset against
        #[arg(long = "quotes", default_value = "USDT")]
        quotes: String,
    },
}

async fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let mut config = Config::load();

    match cli.command {
        Commands::Auth { command } => match command {
            AuthCommands::Login => auth::handle_login(&mut config),
            AuthCommands::Logout => auth::handle_logout(&mut config),
            AuthCommands::Status => {
                auth::handle_status(&config);
                Ok(())
            }
        },

        Commands::Ticker { command } => {
            let client = ApiClient::new(&config);
            match command {
                TickerCommands::Start { exchange, symbol } => {
                    ticker::handle_start(&client, &exchange, &symbol).await
                }
                TickerCommands::Stop { exchange, symbol } => {
                    ticker::handle_stop(&client, &exchange, &symbol).await
                }
                TickerCommands::List => ticker::handle_list(&client).await,
            }
        }

        Commands::Candle { command } => {
            let client = ApiClient::new(&config);
            match command {
                CandleCommands::Backfill {
                    exchange,
                    pair,
                    interval,
                    from,
                    to,
                } => {
                    candle::handle_backfill(&client, &exchange, &pair, &interval, &from, &to).await
                }
            }
        }

        Commands::Exchange { command } => {
            let client = ApiClient::new(&config);
            match command {
                ExchangeCommands::Seed { exchange, quotes } => {
                    exchange::handle_seed(&client, &exchange, &quotes).await
                }
            }
        }

        Commands::Config { command } => match command {
            ConfigCommands::SetUrl { url } => {
                config.set_url(&url);
                config.save()?;
                println!("{} Server URL: {}", "✓".green(), url.cyan());
                Ok(())
            }
            ConfigCommands::Show => {
                println!("Server:  {}", config.server.url.cyan());
                if config.is_authenticated() {
                    println!("Auth:    {}", "authenticated".green());
                } else {
                    println!("Auth:    {}", "not authenticated".yellow());
                }
                Ok(())
            }
        },
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}
