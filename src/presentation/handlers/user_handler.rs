use actix_web::Responder;
use actix_web::{web, HttpResponse};
use serde_json::json;
use crate::domain::repositories::UserRepository;
use crate::domain::entities::User;


// user_repo: web::Data<dyn UserRepository>
pub async fn get_user(user_id: web::Path<i32>) -> impl Responder {
    HttpResponse::Ok().json(json!({
        "fn":"get_user",
    }))

    // match user_repo.find_by_id(user_id.into_inner()) {
    //     Some(user) => HttpResponse::Ok().json(user),
    //     None => HttpResponse::NotFound().body("User not found"),
    // }
}


// new_user: web::Json<User>, user_repo: web::Data<dyn UserRepository>
pub async fn create_user() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "fn":"create_user"
    }))
    // user_repo.save(new_user.into_inner());
    // HttpResponse::Created().body("User created")
}
