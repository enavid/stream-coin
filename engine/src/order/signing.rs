//! Request signing primitives shared by the live order adapters (C10).
//!
//! All the exchanges integrated here authenticate write requests with an
//! HMAC-SHA256 over a canonical request string, keyed by the user's `api_secret`.
//! The exact canonical string differs per exchange (Binance signs the query
//! string; HollaEx/Exir signs `VERB + path + expires`), so each adapter builds
//! its own string and calls [`hmac_sha256_hex`] here. Keeping the cryptographic
//! primitive in one tested place means no adapter hand-rolls HMAC.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Lower-case hex HMAC-SHA256 of `message` keyed by `secret`.
///
/// This is the signature format every integrated exchange expects. HMAC accepts
/// a key of any length, so construction is infallible.
pub fn hmac_sha256_hex(secret: &str, message: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(message.as_bytes());
    to_hex(&mac.finalize().into_bytes())
}

/// Joins ordered params into a `k=v&k=v` string with RFC 3986 percent-encoding of
/// each value — the canonical form a Binance-style signature is computed over and
/// then sent as the request body/query (the signed string must equal the sent one).
pub fn encode_query(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Current Unix time in milliseconds — the `timestamp` value signed requests
/// require. Saturates to 0 before the Unix epoch (unreachable in practice).
pub fn current_timestamp_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_openssl_reference() {
        // Oracle: `printf '%s' "<msg>" | openssl dgst -sha256 -hmac "<secret>"`.
        // Pinning this proves our hex HMAC-SHA256 is byte-identical to the
        // reference implementation every exchange validates against.
        let secret = "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0";
        let query = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        assert_eq!(
            hmac_sha256_hex(secret, query),
            "b89008e7051ffbf2242be7dc5ae67fd146e6430688627b802c0cbec146e46aef"
        );
    }

    #[test]
    fn hmac_sha256_is_lowercase_hex_of_fixed_length() {
        let sig = hmac_sha256_hex("secret", "message");
        assert_eq!(sig.len(), 64, "SHA-256 is 32 bytes = 64 hex chars");
        assert!(sig
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn hmac_sha256_differs_for_different_secret() {
        let a = hmac_sha256_hex("secret-a", "message");
        let b = hmac_sha256_hex("secret-b", "message");
        assert_ne!(a, b, "the signature must depend on the secret");
    }

    #[test]
    fn hmac_sha256_empty_secret_does_not_panic() {
        let sig = hmac_sha256_hex("", "message");
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn encode_query_preserves_order_and_joins_with_ampersand() {
        let params = vec![
            ("symbol", "USDTIRT".to_string()),
            ("side", "BUY".to_string()),
            ("quantity", "100".to_string()),
        ];
        assert_eq!(
            encode_query(&params),
            "symbol=USDTIRT&side=BUY&quantity=100"
        );
    }

    #[test]
    fn encode_query_percent_encodes_reserved_characters() {
        let params = vec![("clientOrderId", "a b&c=d/e".to_string())];
        // space->%20, &->%26, =->%3D, /->%2F
        assert_eq!(encode_query(&params), "clientOrderId=a%20b%26c%3Dd%2Fe");
    }

    #[test]
    fn encode_query_then_sign_is_stable() {
        // The exact string we sign must be reproducible byte-for-byte.
        let params = vec![("a", "1".to_string()), ("b", "2".to_string())];
        let encoded = encode_query(&params);
        assert_eq!(encoded, "a=1&b=2");
        assert_eq!(
            hmac_sha256_hex("k", &encoded),
            hmac_sha256_hex("k", "a=1&b=2")
        );
    }
}
