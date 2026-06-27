use std::collections::HashSet;

use actix_web::body::EitherBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::header;
use actix_web::http::Method;
use actix_web::middleware::Next;
use actix_web::{web, Error, HttpMessage, HttpResponse};
use jsonwebtoken::{decode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::presentation::shared::app_state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    /// Flattened permission set, embedded at login so authorization needs no DB
    /// round-trip per request. `#[serde(default)]` keeps old 2-field tokens decodable.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// Authenticated request context, inserted into request extensions by `jwt_middleware`
/// after a successful token decode. Handlers extract it to check permissions.
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: i32,
    pub permissions: HashSet<String>,
}

/// Wildcard permission granted only in the dev-only "auth disabled" mode
/// (see [`jwt_middleware`]). `AuthContext::has` treats it as "all permissions".
/// Production never runs in this mode — [`resolve_jwt_secret`] refuses to boot
/// without a configured `JWT_SECRET`.
pub const WILDCARD_PERMISSION: &str = "*";

impl AuthContext {
    pub fn has(&self, permission: &str) -> bool {
        self.permissions.contains(WILDCARD_PERMISSION) || self.permissions.contains(permission)
    }
}

/// Resolves the JWT secret at startup, failing closed by default.
///
/// - A non-empty secret enables authentication.
/// - An absent or empty secret is a hard error **unless** `allow_insecure` is set
///   (the `ALLOW_INSECURE_NO_AUTH` escape hatch, for local development only).
///
/// This closes the fail-open gap where a missing `JWT_SECRET` silently served the
/// entire API unauthenticated. Pure and unit-tested; the binary maps `Err` to a
/// refused boot.
pub fn resolve_jwt_secret(
    secret: Option<&str>,
    allow_insecure: bool,
) -> Result<Option<String>, String> {
    match secret {
        Some(s) if !s.is_empty() => Ok(Some(s.to_string())),
        _ if allow_insecure => Ok(None),
        _ => Err(
            "JWT_SECRET is not set — refusing to start with authentication disabled. \
             Set JWT_SECRET, or set ALLOW_INSECURE_NO_AUTH=1 for local development only."
                .to_string(),
        ),
    }
}

/// Extracts `AuthContext` from request extensions and checks for `permission`.
/// Returns the context on success, or a ready-to-send 401/403 `HttpResponse` on failure —
/// handlers call this first and `return` the `Err` value directly.
pub fn require_permission(
    req: &actix_web::HttpRequest,
    permission: &str,
) -> Result<AuthContext, HttpResponse> {
    use crate::presentation::responses::ApiError;

    match req.extensions().get::<AuthContext>().cloned() {
        Some(ctx) if ctx.has(permission) => Ok(ctx),
        Some(_) => {
            Err(ApiError::forbidden(&format!("missing permission: {permission}")).to_response())
        }
        None => Err(ApiError::unauthorized("authentication required").to_response()),
    }
}

/// Returns `true` when the request must bypass JWT validation.
/// Exempt:
/// - `GET /v1/check/health`
/// - `GET /v1/exchanges`  (public exchange list)
/// - `POST /v1/auth/token`   (login — cannot require a token to get a token)
/// - `POST /v1/auth/refresh` (refresh — validated inside the handler to allow expired tokens)
fn is_exempt(path: &str, method: &Method) -> bool {
    path == "/v1/check/health"
        || (path == "/v1/exchanges" && method == Method::GET)
        || path == "/v1/auth/token"
        || path == "/v1/auth/refresh"
}

/// Extracts the Bearer token from the Authorization header.
pub fn extract_bearer_token(headers: &actix_web::http::header::HeaderMap) -> Option<String> {
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    auth.strip_prefix("Bearer ").map(|t| t.to_string())
}

/// Extracts `token` from a raw query string (`a=1&token=xyz&b=2`). JWTs are
/// base64url (`A-Za-z0-9-_.` only), so no percent-decoding is needed —
/// every character a token can contain is already URL-safe.
///
/// Only used for `/v1/ws`: a browser's native WebSocket API cannot set an
/// `Authorization` header on the upgrade request, so the token has to
/// travel in the URL for that one endpoint. Every other route still
/// requires the header.
fn extract_query_token(query: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token" && !value.is_empty()).then(|| value.to_string())
    })
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

/// Validates an HS256 JWT, ignoring token expiry.
/// Used by the refresh endpoint so a recently-expired token can still be renewed.
pub fn validate_jwt_allow_expired(token: &str, secret: &str) -> Result<Claims, String> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::default();
    validation.validate_exp = false;
    decode::<Claims>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(|e| e.to_string())
}

/// Creates a signed HS256 token with no permissions — used by the ~existing test suite
/// (ticker/strategy/order endpoints) that only needs a validly-signed token, not
/// authorization. Real login uses `mint_token_with_permissions`.
/// `exp_from_now_secs` > 0 = future expiry (valid), < 0 = past expiry (expired).
pub fn mint_token(sub: &str, secret: &str, exp_from_now_secs: i64) -> String {
    mint_token_with_permissions(sub, secret, exp_from_now_secs, &[])
}

/// Creates a signed HS256 token carrying the user's flattened permission set.
/// `sub` must be the user's numeric id (as a string) for `AuthContext` extraction to work.
pub fn mint_token_with_permissions(
    sub: &str,
    secret: &str,
    exp_from_now_secs: i64,
    permissions: &[String],
) -> String {
    let exp = (chrono::Utc::now().timestamp() + exp_from_now_secs) as usize;
    let claims = Claims {
        sub: sub.to_string(),
        exp,
        permissions: permissions.to_vec(),
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

    // Auth disabled (jwt_secret = None) — dev/test only; the binary refuses to boot
    // in this mode (see `resolve_jwt_secret`). Insert a wildcard superuser context so
    // single-tenant local runs and the legacy no-auth test harness keep working.
    let secret = match secret {
        None => {
            req.extensions_mut().insert(AuthContext {
                user_id: 0,
                permissions: HashSet::from([WILDCARD_PERMISSION.to_string()]),
            });
            return next.call(req).await.map(|r| r.map_into_left_body());
        }
        Some(s) => s,
    };

    let token = extract_bearer_token(req.headers()).or_else(|| {
        (req.path() == "/v1/ws")
            .then(|| extract_query_token(req.query_string()))
            .flatten()
    });
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
        Ok(claims) => {
            if let Ok(user_id) = claims.sub.parse::<i32>() {
                req.extensions_mut().insert(AuthContext {
                    user_id,
                    permissions: claims.permissions.into_iter().collect(),
                });
            }
            next.call(req).await.map(|r| r.map_into_left_body())
        }
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
    fn extract_query_token_finds_token_param() {
        assert_eq!(
            extract_query_token("token=abc.def.ghi"),
            Some("abc.def.ghi".to_string())
        );
    }

    #[test]
    fn extract_query_token_finds_token_among_other_params() {
        assert_eq!(
            extract_query_token("foo=1&token=abc.def.ghi&bar=2"),
            Some("abc.def.ghi".to_string())
        );
    }

    #[test]
    fn extract_query_token_returns_none_when_absent() {
        assert_eq!(extract_query_token("foo=1&bar=2"), None);
    }

    #[test]
    fn extract_query_token_returns_none_for_empty_value() {
        assert_eq!(extract_query_token("token="), None);
    }

    #[test]
    fn extract_query_token_returns_none_for_empty_query_string() {
        assert_eq!(extract_query_token(""), None);
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

    #[test]
    fn auth_token_path_is_exempt() {
        assert!(is_exempt("/v1/auth/token", &Method::POST));
    }

    #[test]
    fn auth_refresh_path_is_exempt() {
        assert!(is_exempt("/v1/auth/refresh", &Method::POST));
    }

    #[test]
    fn validate_jwt_allow_expired_accepts_past_expiry() {
        let secret = "test_secret";
        let token = mint_token("user", secret, -1);
        assert!(
            validate_jwt_allow_expired(&token, secret).is_ok(),
            "expired token must pass when expiry check is disabled"
        );
    }

    #[test]
    fn validate_jwt_allow_expired_still_rejects_wrong_secret() {
        let token = mint_token("user", "correct", 3600);
        assert!(
            validate_jwt_allow_expired(&token, "wrong").is_err(),
            "wrong secret must still fail even with expiry check disabled"
        );
    }

    #[test]
    fn mint_token_with_permissions_round_trips_permissions() {
        let secret = "test_secret";
        let perms = vec!["users.manage".to_string(), "roles.manage".to_string()];
        let token = mint_token_with_permissions("1", secret, 3600, &perms);
        let claims = validate_jwt(&token, secret).unwrap();
        assert_eq!(claims.permissions, perms);
    }

    #[test]
    fn mint_token_produces_empty_permissions() {
        let secret = "test_secret";
        let token = mint_token("user", secret, 3600);
        let claims = validate_jwt(&token, secret).unwrap();
        assert!(claims.permissions.is_empty());
    }

    #[test]
    fn claims_decode_old_two_field_token_with_empty_permissions() {
        // Simulates a token minted before the `permissions` field existed.
        #[derive(Serialize)]
        struct OldClaims {
            sub: String,
            exp: usize,
        }
        let secret = "test_secret";
        let old = OldClaims {
            sub: "1".to_string(),
            exp: (chrono::Utc::now().timestamp() + 3600) as usize,
        };
        let token = jsonwebtoken::encode(
            &Header::default(),
            &old,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        let claims = validate_jwt(&token, secret).unwrap();
        assert!(claims.permissions.is_empty());
    }

    #[test]
    fn auth_context_has_returns_true_for_granted_permission() {
        let ctx = AuthContext {
            user_id: 1,
            permissions: HashSet::from(["users.manage".to_string()]),
        };
        assert!(ctx.has("users.manage"));
        assert!(!ctx.has("roles.manage"));
    }

    // --- C3: fail-closed JWT secret resolution + wildcard dev context ---

    #[test]
    fn auth_context_wildcard_grants_any_permission() {
        let ctx = AuthContext {
            user_id: 0,
            permissions: HashSet::from([WILDCARD_PERMISSION.to_string()]),
        };
        assert!(ctx.has("orders.manage"));
        assert!(ctx.has("anything.at.all"));
    }

    #[test]
    fn resolve_jwt_secret_returns_secret_when_present() {
        assert_eq!(
            resolve_jwt_secret(Some("s3cr3t"), false).unwrap(),
            Some("s3cr3t".to_string())
        );
    }

    #[test]
    fn resolve_jwt_secret_refuses_boot_when_absent_and_not_insecure() {
        assert!(
            resolve_jwt_secret(None, false).is_err(),
            "missing secret must fail closed (refuse to boot)"
        );
    }

    #[test]
    fn resolve_jwt_secret_treats_empty_as_absent() {
        assert!(
            resolve_jwt_secret(Some(""), false).is_err(),
            "empty secret must be treated as not configured"
        );
    }

    #[test]
    fn resolve_jwt_secret_allows_none_when_insecure_opt_in() {
        assert_eq!(
            resolve_jwt_secret(None, true).unwrap(),
            None,
            "explicit insecure opt-in disables auth (dev only)"
        );
    }

    #[test]
    fn resolve_jwt_secret_prefers_real_secret_even_with_insecure_flag() {
        assert_eq!(
            resolve_jwt_secret(Some("real"), true).unwrap(),
            Some("real".to_string())
        );
    }
}
