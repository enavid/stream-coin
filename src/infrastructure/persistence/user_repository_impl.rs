use crate::domain::entities::User;
use crate::domain::repositories::UserRepository;

pub struct UserRepositoryImpl;

impl UserRepository for UserRepositoryImpl {
    fn save(&self, user: User) {
        println!("Saving user: {:?}", user);
    }

    fn find_by_id(&self, id: i32) -> Option<User> {
        todo!()
    }

    fn find_by_username(&self, username: &str) -> Option<User> {
        todo!()
    }
}
