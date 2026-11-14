//! The one-off unlock. A licence key is the buyer's email signed with
//! Ed25519 by the licence server; it is checked here offline against the
//! embedded public key, so the app never phones home.
//!
//! Key format: `MAC1.<base64url email>.<base64url signature>`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};

/// Where "Buy" sends people. While this is empty the app has no paid tier
/// at all: the limit is off, so a build can never ship a paywall that
/// nobody can pay.
pub const BUY_URL: &str = "https://mp3.leatham.net/buy";

/// Files converted per run without a licence.
pub const FREE_BATCH: usize = 3;

/// Public half of the licence server's signing key (base64url, 32 bytes).
const PUBLIC_KEY: &str = "WUCBcqf36s-BDPFF1mw5Y8bZlUVZ_bggBqvFg9SkK_Q";

const PREFIX: &str = "MAC1";

pub fn paid_tier_enabled() -> bool {
    !BUY_URL.is_empty()
}

/// Returns the licensed email if `key` is genuine.
pub fn verify(key: &str) -> Option<String> {
    let public = URL_SAFE_NO_PAD.decode(PUBLIC_KEY).ok()?;
    verify_with(public.as_slice().try_into().ok()?, key)
}

fn verify_with(public_key: &[u8; 32], key: &str) -> Option<String> {
    // Keys get pasted from emails and web pages: tolerate stray whitespace.
    let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    let mut parts = key.split('.');
    let (prefix, email, signature) = (parts.next()?, parts.next()?, parts.next()?);
    if prefix != PREFIX || parts.next().is_some() {
        return None;
    }
    let email = URL_SAFE_NO_PAD.decode(email).ok()?;
    let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
    let signature = Signature::from_slice(&signature).ok()?;
    let verifying_key = VerifyingKey::from_bytes(public_key).ok()?;
    verifying_key.verify_strict(&email, &signature).ok()?;
    String::from_utf8(email).ok()
}

/// How many of `pending` files a run may convert.
pub fn run_allowance(pending: usize, licensed: bool) -> usize {
    if licensed || !paid_tier_enabled() {
        pending
    } else {
        pending.min(FREE_BATCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn issue(signing_key: &SigningKey, email: &str) -> String {
        let signature = signing_key.sign(email.as_bytes());
        format!(
            "{PREFIX}.{}.{}",
            URL_SAFE_NO_PAD.encode(email),
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        )
    }

    #[test]
    fn accepts_a_genuine_key_and_returns_the_email() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let public = signing_key.verifying_key().to_bytes();
        let key = issue(&signing_key, "buyer@example.com");
        assert_eq!(
            verify_with(&public, &key).as_deref(),
            Some("buyer@example.com")
        );
        // Pasted with line breaks and spaces.
        let messy = format!("  {}\n{} ", &key[..20], &key[20..]);
        assert_eq!(
            verify_with(&public, &messy).as_deref(),
            Some("buyer@example.com")
        );
    }

    #[test]
    fn rejects_forged_altered_and_malformed_keys() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let public = signing_key.verifying_key().to_bytes();
        let other = SigningKey::from_bytes(&[8u8; 32]);
        assert_eq!(verify_with(&public, &issue(&other, "a@b.c")), None);

        let key = issue(&signing_key, "a@b.c");
        let signature = key.rsplit('.').next().unwrap();
        let swapped = format!(
            "{PREFIX}.{}.{signature}",
            URL_SAFE_NO_PAD.encode("z@b.c")
        );
        assert_eq!(verify_with(&public, &swapped), None);

        for bad in ["", "MAC1", "MAC1..", "HWC2.YQ.YQ", "not a key", &format!("{key}.x")] {
            assert_eq!(verify_with(&public, bad), None, "{bad}");
        }
    }

    #[test]
    fn embedded_public_key_is_well_formed() {
        let bytes = URL_SAFE_NO_PAD.decode(PUBLIC_KEY).unwrap();
        let bytes: [u8; 32] = bytes.as_slice().try_into().unwrap();
        assert!(VerifyingKey::from_bytes(&bytes).is_ok());
        assert_eq!(verify("MAC1.YQ.YQ"), None);
    }

    /// Set HWC_LICENSE_KEY to a key issued by the real licence server to
    /// prove the server's signing key matches the embedded public key.
    #[test]
    fn accepts_a_key_from_the_real_server_when_provided() {
        if let Ok(key) = std::env::var("HWC_LICENSE_KEY") {
            assert_eq!(verify(&key).as_deref(), Some("buyer@example.com"));
        }
    }

    #[test]
    fn allowance_only_limits_when_a_paid_tier_exists() {
        assert_eq!(run_allowance(10, true), 10);
        let expected = if paid_tier_enabled() { FREE_BATCH } else { 10 };
        assert_eq!(run_allowance(10, false), expected);
        assert_eq!(run_allowance(2, false), 2);
    }
}
