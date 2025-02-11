use std::sync::Arc;
use sea_orm::{Database, DatabaseConnection};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DatabaseConnection>,
}

pub async fn establish_connection(database_url: &str) -> DatabaseConnection {
    Database::connect(database_url).await.expect("Failed to connect to database")
}
