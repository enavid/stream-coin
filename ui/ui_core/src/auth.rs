//! Client-side session derived from the JWT the backend hands back on
//! login. This only *decodes* the payload to read `permissions` for UI
//! gating (which nav links/buttons to show) — it never verifies the
//! signature. Signature verification, and therefore the actual security
//! boundary, lives entirely on the server
//! (`engine/src/presentation/middlewares/jwt.rs`); a user could hand-edit
//! their own decoded permissions and it would change nothing but which
//! buttons they see — every request still gets re-checked server-side.

use std::collections::HashSet;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub token: String,
    pub user_id: String,
    pub permissions: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    MalformedToken,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    permissions: Vec<String>,
}

impl Session {
    pub fn from_token(token: String) -> Result<Self, AuthError> {
        let payload_segment = token.split('.').nth(1).ok_or(AuthError::MalformedToken)?;

        let decoded = URL_SAFE_NO_PAD
            .decode(payload_segment)
            .map_err(|_| AuthError::MalformedToken)?;

        let claims: Claims =
            serde_json::from_slice(&decoded).map_err(|_| AuthError::MalformedToken)?;

        Ok(Session {
            token,
            user_id: claims.sub,
            permissions: claims.permissions.into_iter().collect(),
        })
    }

    pub fn has(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(sub: &str, permissions: &[&str]) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#);
        let payload = serde_json::json!({ "sub": sub, "permissions": permissions });
        let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{payload}.fake-signature")
    }

    #[test]
    fn session_from_token_decodes_permissions() {
        let token = make_token("1", &["users.manage", "roles.manage"]);
        let session = Session::from_token(token).unwrap();

        assert_eq!(session.user_id, "1");
        assert!(session.has("users.manage"));
        assert!(session.has("roles.manage"));
    }

    #[test]
    fn session_from_token_defaults_permissions_when_field_absent() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#);
        let payload = URL_SAFE_NO_PAD.encode(r#"{"sub":"7"}"#);
        let token = format!("{header}.{payload}.fake-signature");

        let session = Session::from_token(token).unwrap();
        assert_eq!(session.user_id, "7");
        assert!(session.permissions.is_empty());
    }

    #[test]
    fn session_from_token_rejects_malformed_jwt() {
        assert_eq!(
            Session::from_token("not-a-jwt".to_string()),
            Err(AuthError::MalformedToken)
        );
    }

    #[test]
    fn session_from_token_rejects_non_base64_payload_segment() {
        let token = "header.not!!valid!!base64.sig".to_string();
        assert_eq!(Session::from_token(token), Err(AuthError::MalformedToken));
    }

    #[test]
    fn has_returns_false_for_unlisted_permission() {
        let token = make_token("1", &["users.manage"]);
        let session = Session::from_token(token).unwrap();
        assert!(!session.has("roles.manage"));
    }
}
