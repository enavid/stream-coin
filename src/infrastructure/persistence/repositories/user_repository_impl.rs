use sea_orm::*;
use crate::infrastructure::persistence::models::user::ActiveModel;
use crate::infrastructure::persistence::models::user::{Entity as User, Model as UserModel};

#[derive(Clone)]
pub struct UserRepository {
    pub db: DatabaseConnection,
}

impl UserRepository {
    pub async fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn find_by_id(&self, id: i32) -> Result<Option<UserModel>, DbErr> {
        User::find_by_id(id).one(&self.db).await
    }

    pub async fn create_user(&self, username: String, email: String) -> Result<UserModel, DbErr> {
        let new_user = ActiveModel {
            username: Set(username),
            email: Set(email),
            created_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        new_user.insert(&self.db).await
    }
}
