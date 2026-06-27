use actix_cors::Cors;
use actix_web::http::header;

/// Builds the CORS layer. `allowed_origins` is a comma-separated list (from
/// `CORS_ALLOWED_ORIGINS`) — set this in production to the real UI
/// domain(s). With no allowlist configured, only `localhost`/`127.0.0.1`
/// (any port) is allowed — `dx serve` binds the UI dev server to a fresh
/// random port every run, so no single fixed dev origin can be
/// hardcoded, but the API must never be reachable from arbitrary internet
/// origins just because a token leaked somewhere.
pub fn cors_middleware(allowed_origins: Option<&str>) -> Cors {
    let cors = Cors::default()
        .allowed_methods(["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
        .allowed_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
        .max_age(3600);

    match allowed_origins.map(str::trim) {
        Some(origins) if !origins.is_empty() => origins
            .split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .fold(cors, Cors::allowed_origin),
        _ => cors.allowed_origin_fn(|origin, _req_head| {
            origin.to_str().map(is_local_dev_origin).unwrap_or(false)
        }),
    }
}

/// `http://localhost[:port]` or `http://127.0.0.1[:port]`, any port,
/// nothing else — in particular not `http://localhost.evil.com` (the
/// `strip_prefix` leaves `.evil.com`, which is neither empty nor a `:`
/// followed by digits, so it's rejected).
fn is_local_dev_origin(origin: &str) -> bool {
    for host in ["http://localhost", "http://127.0.0.1"] {
        let Some(rest) = origin.strip_prefix(host) else {
            continue;
        };
        return match rest.strip_prefix(':') {
            Some(port) => !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()),
            None => rest.is_empty(),
        };
    }
    false
}

/// Reads `CORS_ALLOWED_ORIGINS` from the environment — the production
/// entry point (`engine/bin/http.rs`) calls this; tests call
/// [`cors_middleware`] directly with an explicit value to stay
/// independent of process-wide environment state.
pub fn configure_cors() -> Cors {
    cors_middleware(std::env::var("CORS_ALLOWED_ORIGINS").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_dev_origin_accepts_localhost_with_any_port() {
        assert!(is_local_dev_origin("http://localhost:38391"));
        assert!(is_local_dev_origin("http://localhost:8080"));
    }

    #[test]
    fn is_local_dev_origin_accepts_127_0_0_1_with_any_port() {
        assert!(is_local_dev_origin("http://127.0.0.1:5173"));
    }

    #[test]
    fn is_local_dev_origin_accepts_bare_localhost_without_port() {
        assert!(is_local_dev_origin("http://localhost"));
    }

    #[test]
    fn is_local_dev_origin_rejects_arbitrary_internet_origin() {
        assert!(!is_local_dev_origin("https://evil.com"));
    }

    #[test]
    fn is_local_dev_origin_rejects_lookalike_subdomain_attack() {
        assert!(!is_local_dev_origin("http://localhost.evil.com"));
    }

    #[test]
    fn is_local_dev_origin_rejects_non_numeric_port() {
        assert!(!is_local_dev_origin("http://localhost:abc"));
    }

    #[test]
    fn is_local_dev_origin_rejects_https_scheme() {
        // dx serve's dev server is plain http; reject https to keep the
        // matcher exact rather than guessing at every variant.
        assert!(!is_local_dev_origin("https://localhost:38391"));
    }
}
