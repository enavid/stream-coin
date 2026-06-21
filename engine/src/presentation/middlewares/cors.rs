use actix_cors::Cors;
use actix_web::http::header;

/// Builds the CORS layer. `allowed_origins` is a comma-separated list (from
/// `CORS_ALLOWED_ORIGINS`) for production; `None`/empty falls back to
/// allowing any origin, which is what local development needs — `dx serve`
/// binds the UI to a fresh random port on every run, so no fixed origin
/// can be hardcoded. Credentials (cookies) are never used by this API
/// (auth is a Bearer token), so `allow_any_origin` carries no CSRF risk.
pub fn cors_middleware(allowed_origins: Option<&str>) -> Cors {
    let cors = Cors::default()
        .allowed_methods(["GET", "POST", "PUT", "DELETE", "OPTIONS"])
        .allowed_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
        .max_age(3600);

    match allowed_origins.map(str::trim) {
        Some(origins) if !origins.is_empty() => origins
            .split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .fold(cors, Cors::allowed_origin),
        _ => cors.allow_any_origin(),
    }
}

/// Reads `CORS_ALLOWED_ORIGINS` from the environment — the production
/// entry point (`engine/bin/http.rs`) calls this; tests call
/// [`cors_middleware`] directly with an explicit value to stay
/// independent of process-wide environment state.
pub fn configure_cors() -> Cors {
    cors_middleware(std::env::var("CORS_ALLOWED_ORIGINS").ok().as_deref())
}
