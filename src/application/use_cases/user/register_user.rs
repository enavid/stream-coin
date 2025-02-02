use crate::domain::entities::User;
use crate::domain::repositories::UserRepository;

pub struct RegisterUser<'a> {
    user_repo: &'a dyn UserRepository,
}

impl<'a> RegisterUser<'a> {
    pub fn new(user_repo: &'a dyn UserRepository) -> Self {
        RegisterUser { user_repo }
    }

    pub fn execute(&self, username: String, email: String) -> User {
        let user = User::new(0, username, email);
        self.user_repo.save(user.clone());
        user
    }
}
