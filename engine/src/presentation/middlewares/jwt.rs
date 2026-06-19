use actix_web::body::EitherBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::header;
use actix_web::http::Method;
use actix_web::middleware::Next;
use actix_web::{web, Error, HttpResponse};
use jsonwebtoken::{decode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::presentation::shared::app_state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

/// Returns `true` when the request must bypass JWT validation.
/// Exempt:
/// - `GET /v1/check/health`
/// - `GET /v1/exchanges`  (public exchange list)
fn is_exempt(path: &str, method: &Method) -> bool {
    path == "/v1/check/health" || (path == "/v1/exchanges" && method == Method::GET)
}

/// Extracts the Bearer token from the Authorization header.
pub fn extract_bearer_token(headers: &actix_web::http::header::HeaderMap) -> Option<String> {
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    auth.strip_prefix("Bearer ").map(|t| t.to_string())
}

/// Validates an HS256 JWT against the given secret.
pub fn validate_jwt(token: &str, secret: &str) -> Result<Claims, String> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::default();
    validation.leeway = 0;
    decode::<Claims>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(|e| e.to_string())
}

/// Creates a signed HS256 token — used in tests and `sc auth login`.
/// `exp_from_now_secs` > 0 = future expiry (valid), < 0 = past expiry (expired).
pub fn mint_token(sub: &str, secret: &str, exp_from_now_secs: i64) -> String {
    let exp = (chrono::Utc::now().timestamp() + exp_from_now_secs) as usize;
    let claims = Claims {
        sub: sub.to_string(),
        exp,
    };
    jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("token encoding must not fail")
}

/// JWT middleware — applied at the `/v1/` scope level.
/// Passes exempt paths through without validation.
/// Returns 401 when the token is missing or invalid.
pub async fn jwt_middleware<B: actix_web::body::MessageBody + 'static>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    // Exempt paths bypass auth entirely.
    if is_exempt(req.path(), req.method()) {
        return next.call(req).await.map(|r| r.map_into_left_body());
    }

    let secret = req
        .app_data::<web::Data<AppState>>()
        .and_then(|s| s.jwt_secret.as_ref().map(|arc| arc.as_str().to_string()));

    // Auth disabled (jwt_secret = None) — pass through.
    let secret = match secret {
        None => return next.call(req).await.map(|r| r.map_into_left_body()),
        Some(s) => s,
    };

    let token = extract_bearer_token(req.headers());
    let token = match token {
        Some(t) => t,
        None => {
            let (req, _) = req.into_parts();
            let res = HttpResponse::Unauthorized().json(
                serde_json::json!({"success": false, "message": "Missing authorization token"}),
            );
            return Ok(ServiceResponse::new(req, res).map_into_right_body());
        }
    };

    match validate_jwt(&token, &secret) {
        Ok(_) => next.call(req).await.map(|r| r.map_into_left_body()),
        Err(reason) => {
            let (req, _) = req.into_parts();
            let res = HttpResponse::Unauthorized()
                .json(serde_json::json!({"success": false, "message": format!("Invalid token: {reason}")}));
            Ok(ServiceResponse::new(req, res).map_into_right_body())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::header::HeaderMap;

    // --- unit tests (ROADMAP 1c) ---

    #[test]
    fn jwt_valid_token_passes_middleware() {
        let secret = "test_secret";
        let token = mint_token("user", secret, 3600);
        assert!(
            validate_jwt(&token, secret).is_ok(),
            "valid token must pass validation"
        );
    }

    #[test]
    fn jwt_expired_token_returns_401() {
        let secret = "test_secret";
        let token = mint_token("user", secret, -1);
        let result = validate_jwt(&token, secret);
        assert!(result.is_err(), "expired token must fail validation");
    }

    #[test]
    fn jwt_missing_token_returns_401() {
        let headers = HeaderMap::new();
        assert!(
            extract_bearer_token(&headers).is_none(),
            "missing Authorization header must return None"
        );
    }

    #[test]
    fn jwt_wrong_secret_returns_error() {
        let token = mint_token("user", "correct_secret", 3600);
        let result = validate_jwt(&token, "wrong_secret");
        assert!(result.is_err(), "token signed with wrong secret must fail");
    }

    #[test]
    fn extract_bearer_token_strips_bearer_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer mytoken123".parse().unwrap());
        assert_eq!(
            extract_bearer_token(&headers).as_deref(),
            Some("mytoken123")
        );
    }

    #[test]
    fn extract_bearer_token_returns_none_for_non_bearer_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Basic dXNlcjpwYXNz".parse().unwrap());
        assert!(
            extract_bearer_token(&headers).is_none(),
            "non-Bearer auth scheme must return None"
        );
    }

    #[test]
    fn health_path_is_exempt() {
        assert!(is_exempt("/v1/check/health", &Method::GET));
        assert!(is_exempt("/v1/check/health", &Method::POST));
    }

    #[test]
    fn exchanges_get_is_exempt() {
        assert!(is_exempt("/v1/exchanges", &Method::GET));
    }

    #[test]
    fn exchanges_post_is_not_exempt() {
        assert!(!is_exempt("/v1/exchanges", &Method::POST));
    }

    #[test]
    fn ticker_path_is_not_exempt() {
        assert!(!is_exempt(
            "/v1/exchanges/futures/start_kline_symbol_ticker",
            &Method::POST
        ));
    }
}
