use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// Hashes a plaintext password with Argon2 (random salt per call).
pub fn hash_password(plain: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .expect("argon2 hashing must not fail")
        .to_string()
}

/// Verifies a plaintext password against a stored Argon2 hash.
pub fn verify_password(plain: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_password_accepts_correct_password() {
        let hash = hash_password("correct-horse-battery-staple");
        assert!(verify_password("correct-horse-battery-staple", &hash));
    }

    #[test]
    fn verify_password_rejects_wrong_password() {
        let hash = hash_password("correct-horse-battery-staple");
        assert!(!verify_password("wrong-password", &hash));
    }

    #[test]
    fn hash_password_produces_different_hash_each_call() {
        let hash1 = hash_password("same-password");
        let hash2 = hash_password("same-password");
        assert_ne!(hash1, hash2, "salts must differ between calls");
    }

    #[test]
    fn verify_password_rejects_malformed_hash() {
        assert!(!verify_password("anything", "not-a-valid-argon2-hash"));
    }
}
