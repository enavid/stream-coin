use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct User{
    pub id: i32,
    pub username: String,
    pub email: String,
}

impl User{
    pub fn new(id:i32, username: String, email: String) -> Self {
        Self{
            id,
            username,
            email
        }
    }
}