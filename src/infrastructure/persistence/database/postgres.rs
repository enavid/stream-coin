use sea_orm::{Database, DatabaseConnection, DbErr};

pub async fn establish_db_connection(database_url: &str) -> Result<DatabaseConnection, DbErr> {
    Database::connect(database_url).await
}

