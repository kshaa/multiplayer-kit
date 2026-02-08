//! JWT ticket issuance and validation.

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use multiplayer_kit_protocol::{TicketClaims, UserContext};
use std::time::{SystemTime, UNIX_EPOCH};

/// Ticket manager for JWT operations.
pub struct TicketManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    expiry_secs: u64,
}

impl TicketManager {
    pub fn new(secret: &[u8], expiry_secs: u64) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret),
            decoding_key: DecodingKey::from_secret(secret),
            expiry_secs,
        }
    }

    /// Issue a new ticket for a user.
    pub fn issue<T: UserContext>(&self, user: T) -> Result<String, jsonwebtoken::errors::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = TicketClaims {
            exp: now + self.expiry_secs,
            user,
        };

        encode(&Header::default(), &claims, &self.encoding_key)
    }

    /// Validate and decode a ticket.
    pub fn validate<T: UserContext>(&self, token: &str) -> Result<T, jsonwebtoken::errors::Error> {
        let token_data =
            decode::<TicketClaims<T>>(token, &self.decoding_key, &Validation::default())?;
        Ok(token_data.claims.user)
    }
}
