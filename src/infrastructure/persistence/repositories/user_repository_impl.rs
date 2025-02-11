use sea_orm::*;
use std::sync::Arc;
use actix_web::web;
use sea_orm::EntityTrait;
use crate::infrastructure::persistence::models::user::ActiveModel;
use crate::infrastructure::persistence::database::maria_db::AppState;
use crate::infrastructure::persistence::models::user::{Entity as User, Model as UserModel};


#[derive(Clone)]
pub struct UserRepository {
    pub db: Arc<DatabaseConnection>,
}

impl UserRepository {
    pub async fn new(app_state: web::Data<AppState>) -> Self {
        Self { db: Arc::clone(&app_state.db), }
    }

    pub async fn find_by_id(&self, id: i32) -> Result<Option<UserModel>, DbErr> {
        User::find_by_id(id).one(self.db.as_ref()).await
    }

    pub async fn create_user(&self, username: String, email: String) -> Result<UserModel, DbErr> {
        let new_user = ActiveModel {
            username: Set(username),
            email: Set(email),
            created_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        new_user.insert(self.db.as_ref()).await
    }
}
