use colored::Colorize;

use crate::config::Config;

pub fn handle_login(config: &mut Config) -> Result<(), String> {
    let url = format!("{}/swagger-ui/", config.server.url);
    println!("{} Opening: {}", "→".cyan(), url.yellow());
    open::that(&url).map_err(|e| format!("Cannot open browser: {e}"))?;

    println!(
        "{}",
        "Paste your API token (or press Enter to skip):".dimmed()
    );
    let mut token = String::new();
    std::io::stdin()
        .read_line(&mut token)
        .map_err(|e| e.to_string())?;
    let token = token.trim().to_string();

    if token.is_empty() {
        println!("{}", "No token saved. Unauthenticated mode.".yellow());
    } else {
        config.set_token(Some(token));
        config.save()?;
        println!("{}", "✓ Token saved".green());
    }
    Ok(())
}

pub fn handle_logout(config: &mut Config) -> Result<(), String> {
    config.set_token(None);
    config.save()?;
    println!("{}", "✓ Logged out".green());
    Ok(())
}

pub fn handle_status(config: &Config) {
    if config.is_authenticated() {
        println!("{} Authenticated", "✓".green());
    } else {
        println!("{} Not authenticated", "✗".red());
    }
    println!("  Server: {}", config.server.url.cyan());
}
