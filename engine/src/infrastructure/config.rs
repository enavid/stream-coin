//! Startup configuration guards.
//!
//! Pure, unit-testable validators the binary (`engine/bin/http.rs`) calls
//! before wiring up state, so a misconfiguration fails the boot loudly instead
//! of silently running on an insecure default.

/// The placeholder marker shared by every secret default in the compose files
/// and `.env.example`. A real deployment must replace these; a value that still
/// contains it means the secret was never rotated.
const PLACEHOLDER_MARKER: &str = "change-me";

/// Rejects a secret/credential whose value still carries the shipped
/// `change-me` placeholder (case-insensitive, matched anywhere so it also
/// catches the password embedded in a `postgresql://user:change-me@host` URL).
///
/// This is a fail-closed guard for a financial service: a default JWT signing
/// key, database password, or credential-encryption key left at its placeholder
/// is a critical hole (forgeable tokens, trivially reachable DB, decryptable
/// exchange API keys). We refuse to start rather than run on a known value.
pub fn reject_placeholder_secret(name: &str, value: &str) -> Result<(), String> {
    if value.to_ascii_lowercase().contains(PLACEHOLDER_MARKER) {
        return Err(format!(
            "{name} still contains the placeholder '{PLACEHOLDER_MARKER}' — refusing to start. \
             Set a real, rotated secret before deploying."
        ));
    }
    Ok(())
}

/// Resolves a non-secret deployment knob, falling back to `default` when the
/// environment variable was absent or empty/whitespace-only.
///
/// Used for values that must be overridable per environment without a code
/// change — broker REST base URLs (so staging points at a sandbox) and the
/// default market type stamped on order broadcasts — but that have a safe,
/// production-correct default baked in.
pub fn resolve_or_default(env_value: Option<&str>, default: &str) -> String {
    match env_value.map(str::trim) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => default.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_or_default_uses_env_value_when_present() {
        assert_eq!(
            resolve_or_default(Some("https://sandbox.example.com"), "https://prod"),
            "https://sandbox.example.com"
        );
    }

    #[test]
    fn resolve_or_default_falls_back_when_absent() {
        assert_eq!(resolve_or_default(None, "https://prod"), "https://prod");
    }

    #[test]
    fn resolve_or_default_falls_back_when_blank() {
        assert_eq!(resolve_or_default(Some("   "), "spot"), "spot");
    }

    #[test]
    fn resolve_or_default_trims_surrounding_whitespace() {
        assert_eq!(resolve_or_default(Some("  https://x  "), "d"), "https://x");
    }

    #[test]
    fn accepts_a_real_secret() {
        assert!(reject_placeholder_secret("JWT_SECRET", "k7f3p9q2-real-secret").is_ok());
    }

    #[test]
    fn rejects_an_exact_placeholder() {
        assert!(reject_placeholder_secret("JWT_SECRET", "change-me").is_err());
    }

    #[test]
    fn rejects_a_placeholder_suffix() {
        assert!(
            reject_placeholder_secret("CREDENTIALS_ENCRYPTION_KEY", "change-me-please").is_err()
        );
    }

    #[test]
    fn rejects_a_placeholder_embedded_in_a_database_url() {
        let url = "postgresql://stream_coin:change-me@localhost:5432/stream_coin";
        let err = reject_placeholder_secret("DATABASE_URL", url).unwrap_err();
        assert!(
            err.contains("DATABASE_URL"),
            "error must name the offending var"
        );
    }

    #[test]
    fn rejection_is_case_insensitive() {
        assert!(reject_placeholder_secret("JWT_SECRET", "CHANGE-ME-NOW").is_err());
    }

    #[test]
    fn error_message_names_the_variable() {
        let err = reject_placeholder_secret("JWT_SECRET", "change-me").unwrap_err();
        assert!(err.contains("JWT_SECRET"));
    }
}
