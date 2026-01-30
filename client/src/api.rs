//! HTTP/REST API client for ticketing, room management, and server metadata.

use crate::error::{ConnectionError, ReceiveError};
use crate::ClientError;
use multiplayer_kit_protocol::{RoomId, RoomInfo};
use serde::{Deserialize, Serialize};

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
use serde::de::DeserializeOwned;

// ============================================================================
// Shared types
// ============================================================================

/// Response from the ticket endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketResponse {
    pub ticket: String,
}

/// Response from the cert-hash endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertHashResponse {
    pub hash: String,
}

/// Response from the create room endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomResponse {
    pub room_id: u64,
}

// ============================================================================
// Native implementation
// ============================================================================

#[cfg(feature = "native")]
mod native {
    use super::*;

    /// HTTP API client for interacting with the multiplayer-kit server's REST endpoints.
    pub struct ApiClient {
        base_url: String,
        http_client: reqwest::Client,
    }

    impl ApiClient {
        /// Create a new API client pointing to the given server base URL.
        /// Example: `ApiClient::new("http://127.0.0.1:8080")`
        pub fn new(base_url: &str) -> Self {
            Self {
                base_url: base_url.trim_end_matches('/').to_string(),
                http_client: reqwest::Client::new(),
            }
        }

        /// Request a ticket from the server.
        /// The `auth_payload` is application-specific and will be validated by
        /// the server's configured auth handler.
        pub async fn get_ticket<T: Serialize>(
            &self,
            auth_payload: &T,
        ) -> Result<TicketResponse, ClientError> {
            let url = format!("{}/ticket", self.base_url);
            let resp = self
                .http_client
                .post(&url)
                .json(auth_payload)
                .send()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body),
                )));
            }

            resp.json()
                .await
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
        }

        /// Get the server's self-signed certificate hash (base64-encoded SHA-256).
        /// Returns `None` if the server doesn't expose a cert hash (e.g., using real certs).
        pub async fn get_cert_hash(&self) -> Result<Option<String>, ClientError> {
            let url = format!("{}/cert-hash", self.base_url);
            let resp = self
                .http_client
                .get(&url)
                .send()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            if resp.status().is_success() {
                let data: CertHashResponse = resp.json().await.map_err(|e| {
                    ClientError::Receive(ReceiveError::MalformedMessage(e.to_string()))
                })?;
                Ok(Some(data.hash))
            } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
                Ok(None)
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body),
                )))
            }
        }

        /// List all available rooms.
        pub async fn list_rooms(&self) -> Result<Vec<RoomInfo>, ClientError> {
            let url = format!("{}/rooms", self.base_url);
            let resp = self
                .http_client
                .get(&url)
                .send()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body),
                )));
            }

            resp.json()
                .await
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
        }

        /// Create a new room. Requires a valid ticket in the Authorization header.
        pub async fn create_room(&self, ticket: &str) -> Result<CreateRoomResponse, ClientError> {
            let url = format!("{}/rooms", self.base_url);
            let resp = self
                .http_client
                .post(&url)
                .header("Authorization", format!("Bearer {}", ticket))
                .json(&serde_json::json!({}))
                .send()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body),
                )));
            }

            resp.json()
                .await
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
        }

        /// Delete a room. Requires a valid ticket in the Authorization header.
        pub async fn delete_room(&self, ticket: &str, room_id: RoomId) -> Result<(), ClientError> {
            let url = format!("{}/rooms/{}", self.base_url, room_id.0);
            let resp = self
                .http_client
                .delete(&url)
                .header("Authorization", format!("Bearer {}", ticket))
                .send()
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body),
                )));
            }

            Ok(())
        }
    }
}

// ============================================================================
// WASM implementation
// ============================================================================

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response};

    /// HTTP API client for interacting with the multiplayer-kit server's REST endpoints.
    pub struct ApiClient {
        base_url: String,
    }

    impl ApiClient {
        /// Create a new API client pointing to the given server base URL.
        pub fn new(base_url: &str) -> Self {
            Self {
                base_url: base_url.trim_end_matches('/').to_string(),
            }
        }

        async fn fetch<T: DeserializeOwned>(
            &self,
            method: &str,
            path: &str,
            body: Option<String>,
            auth_token: Option<&str>,
        ) -> Result<T, ClientError> {
            let url = format!("{}{}", self.base_url, path);

            let opts = RequestInit::new();
            opts.set_method(method);
            opts.set_mode(RequestMode::Cors);

            if let Some(b) = body {
                opts.set_body(&JsValue::from_str(&b));
            }

            let request = Request::new_with_str_and_init(&url, &opts)
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            request
                .headers()
                .set("Content-Type", "application/json")
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            if let Some(token) = auth_token {
                request
                    .headers()
                    .set("Authorization", &format!("Bearer {}", token))
                    .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;
            }

            let window = web_sys::window()
                .ok_or_else(|| ClientError::Connection(ConnectionError::Transport("No window".to_string())))?;

            let resp_value = JsFuture::from(window.fetch_with_request(&request))
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            let resp: Response = resp_value.unchecked_into();

            if !resp.ok() {
                let status = resp.status();
                let body_text = JsFuture::from(resp.text().unwrap())
                    .await
                    .map(|v| v.as_string().unwrap_or_default())
                    .unwrap_or_default();
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body_text),
                )));
            }

            let json = JsFuture::from(resp.json().unwrap())
                .await
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(format!("{:?}", e))))?;

            serde_wasm_bindgen::from_value(json)
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
        }

        async fn fetch_optional<T: DeserializeOwned>(
            &self,
            method: &str,
            path: &str,
        ) -> Result<Option<T>, ClientError> {
            let url = format!("{}{}", self.base_url, path);

            let opts = RequestInit::new();
            opts.set_method(method);
            opts.set_mode(RequestMode::Cors);

            let request = Request::new_with_str_and_init(&url, &opts)
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            let window = web_sys::window()
                .ok_or_else(|| ClientError::Connection(ConnectionError::Transport("No window".to_string())))?;

            let resp_value = JsFuture::from(window.fetch_with_request(&request))
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            let resp: Response = resp_value.unchecked_into();

            if resp.status() == 404 {
                return Ok(None);
            }

            if !resp.ok() {
                let status = resp.status();
                let body_text = JsFuture::from(resp.text().unwrap())
                    .await
                    .map(|v| v.as_string().unwrap_or_default())
                    .unwrap_or_default();
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body_text),
                )));
            }

            let json = JsFuture::from(resp.json().unwrap())
                .await
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(format!("{:?}", e))))?;

            let value: T = serde_wasm_bindgen::from_value(json)
                .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))?;

            Ok(Some(value))
        }

        /// Request a ticket from the server.
        pub async fn get_ticket<T: Serialize>(
            &self,
            auth_payload: &T,
        ) -> Result<TicketResponse, ClientError> {
            let body = serde_json::to_string(auth_payload)
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;
            self.fetch("POST", "/ticket", Some(body), None).await
        }

        /// Get the server's self-signed certificate hash.
        pub async fn get_cert_hash(&self) -> Result<Option<String>, ClientError> {
            let resp: Option<CertHashResponse> = self.fetch_optional("GET", "/cert-hash").await?;
            Ok(resp.map(|r| r.hash))
        }

        /// List all available rooms.
        pub async fn list_rooms(&self) -> Result<Vec<RoomInfo>, ClientError> {
            self.fetch("GET", "/rooms", None, None).await
        }

        /// Create a new room.
        pub async fn create_room(&self, ticket: &str) -> Result<CreateRoomResponse, ClientError> {
            self.fetch("POST", "/rooms", Some("{}".to_string()), Some(ticket))
                .await
        }

        /// Delete a room.
        pub async fn delete_room(&self, ticket: &str, room_id: RoomId) -> Result<(), ClientError> {
            let url = format!("{}/rooms/{}", self.base_url, room_id.0);

            let opts = RequestInit::new();
            opts.set_method("DELETE");
            opts.set_mode(RequestMode::Cors);

            let request = Request::new_with_str_and_init(&url, &opts)
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            request
                .headers()
                .set("Authorization", &format!("Bearer {}", ticket))
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            let window = web_sys::window()
                .ok_or_else(|| ClientError::Connection(ConnectionError::Transport("No window".to_string())))?;

            let resp_value = JsFuture::from(window.fetch_with_request(&request))
                .await
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(format!("{:?}", e))))?;

            let resp: Response = resp_value.unchecked_into();

            if !resp.ok() {
                let status = resp.status();
                let body_text = JsFuture::from(resp.text().unwrap())
                    .await
                    .map(|v| v.as_string().unwrap_or_default())
                    .unwrap_or_default();
                return Err(ClientError::Connection(ConnectionError::ServerRejected(
                    format!("{}: {}", status, body_text),
                )));
            }

            Ok(())
        }
    }
}

// ============================================================================
// Fallback
// ============================================================================

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
mod fallback {
    use super::*;

    pub struct ApiClient {
        _base_url: String,
    }

    impl ApiClient {
        pub fn new(base_url: &str) -> Self {
            Self {
                _base_url: base_url.to_string(),
            }
        }

        pub async fn get_ticket<T: Serialize>(
            &self,
            _auth_payload: &T,
        ) -> Result<TicketResponse, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No HTTP feature enabled".to_string(),
            )))
        }

        pub async fn get_cert_hash(&self) -> Result<Option<String>, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No HTTP feature enabled".to_string(),
            )))
        }

        pub async fn list_rooms(&self) -> Result<Vec<RoomInfo>, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No HTTP feature enabled".to_string(),
            )))
        }

        pub async fn create_room(&self, _ticket: &str) -> Result<CreateRoomResponse, ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No HTTP feature enabled".to_string(),
            )))
        }

        pub async fn delete_room(&self, _ticket: &str, _room_id: RoomId) -> Result<(), ClientError> {
            Err(ClientError::Connection(ConnectionError::Transport(
                "No HTTP feature enabled".to_string(),
            )))
        }
    }
}

// ============================================================================
// Re-export
// ============================================================================

#[cfg(feature = "native")]
pub use native::ApiClient;

#[cfg(all(feature = "wasm", target_arch = "wasm32", not(feature = "native")))]
pub use wasm::ApiClient;

#[cfg(not(any(feature = "native", all(feature = "wasm", target_arch = "wasm32"))))]
pub use fallback::ApiClient;
