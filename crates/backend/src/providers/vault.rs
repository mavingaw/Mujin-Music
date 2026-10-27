//! Seals per-user streaming credentials and tokens at rest.
//!
//! Values are encrypted with AES-256-GCM under a key kept in
//! `Secrets/streaming.key`. The associated data binds each ciphertext to its
//! user, provider and purpose, so a sealed value copied into another row (or
//! another user's row) fails to open instead of being silently accepted.

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::ProviderId;

const SEAL_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealPurpose {
    Credentials,
    Session,
    PendingAuthorization,
    /// Server-wide credentials, sealed under user id 0.
    ServerCredentials,
}

impl SealPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Credentials => "credentials",
            Self::Session => "session",
            Self::PendingAuthorization => "pending",
            Self::ServerCredentials => "server-credentials",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VaultError {
    Encode,
    Seal,
    Open,
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Encode => "could not encode a sealed value",
            Self::Seal => "could not seal a value",
            Self::Open => "a sealed value could not be opened",
        })
    }
}

impl std::error::Error for VaultError {}

pub struct TokenVault {
    key: LessSafeKey,
}

impl TokenVault {
    pub fn new(key: [u8; 32]) -> Self {
        let unbound = UnboundKey::new(&AES_256_GCM, &key).expect("a 32-byte AES-256-GCM key");
        Self {
            key: LessSafeKey::new(unbound),
        }
    }

    fn associated_data(user_id: i32, provider: ProviderId, purpose: SealPurpose) -> String {
        format!(
            "parson-streaming:v{SEAL_VERSION}:{user_id}:{}:{}",
            provider.as_str(),
            purpose.as_str()
        )
    }

    pub fn seal<T: Serialize>(
        &self,
        user_id: i32,
        provider: ProviderId,
        purpose: SealPurpose,
        value: &T,
    ) -> Result<Vec<u8>, VaultError> {
        self.seal_in_context(&Self::associated_data(user_id, provider, purpose), value)
    }

    pub fn open<T: DeserializeOwned>(
        &self,
        user_id: i32,
        provider: ProviderId,
        purpose: SealPurpose,
        sealed: &[u8],
    ) -> Result<T, VaultError> {
        self.open_in_context(&Self::associated_data(user_id, provider, purpose), sealed)
    }

    /// Seals `value` bound to `context` (the associated data). Other
    /// subsystems (scrobbling) use their own context namespace, so a sealed
    /// value never opens under another subsystem, user or purpose.
    pub fn seal_in_context<T: Serialize>(
        &self,
        context: &str,
        value: &T,
    ) -> Result<Vec<u8>, VaultError> {
        let mut payload = serde_json::to_vec(value).map_err(|_| VaultError::Encode)?;
        let mut nonce = [0_u8; NONCE_LEN];
        rand::fill(&mut nonce);
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(context.as_bytes()),
                &mut payload,
            )
            .map_err(|_| VaultError::Seal)?;
        let mut sealed = Vec::with_capacity(1 + NONCE_LEN + payload.len());
        sealed.push(SEAL_VERSION);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&payload);
        Ok(sealed)
    }

    pub fn open_in_context<T: DeserializeOwned>(
        &self,
        context: &str,
        sealed: &[u8],
    ) -> Result<T, VaultError> {
        if sealed.len() <= 1 + NONCE_LEN || sealed[0] != SEAL_VERSION {
            return Err(VaultError::Open);
        }
        let mut nonce = [0_u8; NONCE_LEN];
        nonce.copy_from_slice(&sealed[1..1 + NONCE_LEN]);
        let mut payload = sealed[1 + NONCE_LEN..].to_vec();
        let plain = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(context.as_bytes()),
                &mut payload,
            )
            .map_err(|_| VaultError::Open)?;
        serde_json::from_slice(plain).map_err(|_| VaultError::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::{SealPurpose, TokenVault, VaultError};
    use crate::providers::ProviderId;

    #[test]
    fn sealed_tokens_round_trip_without_plaintext_on_disk() {
        let vault = TokenVault::new([3; 32]);
        let secret = "refresh-token-that-must-not-leak".to_string();
        let sealed = vault
            .seal(4, ProviderId::Tidal, SealPurpose::Session, &secret)
            .expect("seal");
        assert!(
            !sealed
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        );
        let opened: String = vault
            .open(4, ProviderId::Tidal, SealPurpose::Session, &sealed)
            .expect("open");
        assert_eq!(opened, secret);

        let again = vault
            .seal(4, ProviderId::Tidal, SealPurpose::Session, &secret)
            .expect("seal again");
        assert_ne!(sealed, again, "every seal uses a fresh nonce");
    }

    #[test]
    fn sealed_values_are_bound_to_user_provider_purpose_and_key() {
        let vault = TokenVault::new([3; 32]);
        let sealed = vault
            .seal(4, ProviderId::Qobuz, SealPurpose::Credentials, &"app")
            .expect("seal");
        for (user, provider, purpose) in [
            (5, ProviderId::Qobuz, SealPurpose::Credentials),
            (4, ProviderId::Tidal, SealPurpose::Credentials),
            (4, ProviderId::Qobuz, SealPurpose::Session),
        ] {
            assert_eq!(
                vault.open::<String>(user, provider, purpose, &sealed),
                Err(VaultError::Open)
            );
        }
        assert_eq!(
            TokenVault::new([9; 32]).open::<String>(
                4,
                ProviderId::Qobuz,
                SealPurpose::Credentials,
                &sealed
            ),
            Err(VaultError::Open)
        );
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert_eq!(
            vault.open::<String>(4, ProviderId::Qobuz, SealPurpose::Credentials, &tampered),
            Err(VaultError::Open)
        );
        assert_eq!(
            vault.open::<String>(4, ProviderId::Qobuz, SealPurpose::Credentials, &[1, 2]),
            Err(VaultError::Open)
        );
    }
}
