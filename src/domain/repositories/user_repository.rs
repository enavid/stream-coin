use crate::domain::entities::User;

pub trait UserRepository {
    fn save(&self, user: User);
    fn find_by_id(&self, id: i32) -> Option<User>;
    fn find_by_username(&self, username: &str) -> Option<User>;
}