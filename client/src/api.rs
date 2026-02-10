//! HTTP/REST API client for ticketing, room management, and server metadata.
//!
//! Uses ehttp for cross-platform HTTP (native + WASM).

use crate::ClientError;
use crate::error::{ConnectionError, ReceiveError};
use multiplayer_kit_protocol::{QuickplayResponse, RoomId, RoomInfo};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
use wasm_bindgen::prelude::*;

// ============================================================================
// Response types
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
// HTTP helpers (wrapping ehttp's callback API into async)
// ============================================================================

async fn fetch(request: ehttp::Request) -> Result<ehttp::Response, ClientError> {
    let (tx, rx) = futures::channel::oneshot::channel();

    ehttp::fetch(request, move |result| {
        let _ = tx.send(result);
    });

    rx.await
        .map_err(|_| ClientError::Connection(ConnectionError::Transport("Request cancelled".to_string())))?
        .map_err(|e| ClientError::Connection(ConnectionError::Transport(e)))
}

fn parse_response<T: DeserializeOwned>(response: ehttp::Response) -> Result<T, ClientError> {
    if !response.ok {
        let status = response.status;
        let body = response.text().unwrap_or_default();
        return Err(ClientError::Connection(ConnectionError::ServerRejected(
            format!("{}: {}", status, body),
        )));
    }

    let bytes = response.bytes;
    serde_json::from_slice(&bytes)
        .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))
}

fn parse_optional_response<T: DeserializeOwned>(
    response: ehttp::Response,
) -> Result<Option<T>, ClientError> {
    if response.status == 404 {
        return Ok(None);
    }

    if !response.ok {
        let status = response.status;
        let body = response.text().unwrap_or_default();
        return Err(ClientError::Connection(ConnectionError::ServerRejected(
            format!("{}: {}", status, body),
        )));
    }

    let bytes = response.bytes;
    let value: T = serde_json::from_slice(&bytes)
        .map_err(|e| ClientError::Receive(ReceiveError::MalformedMessage(e.to_string())))?;
    Ok(Some(value))
}

fn post_json(url: String, body: Vec<u8>, auth: Option<&str>) -> ehttp::Request {
    let mut req = ehttp::Request::post(url, body);
    req.headers.insert("Content-Type".to_string(), "application/json".to_string());
    if let Some(token) = auth {
        req.headers.insert("Authorization".to_string(), format!("Bearer {}", token));
    }
    req
}

fn get_with_auth(url: String, auth: Option<&str>) -> ehttp::Request {
    let mut req = ehttp::Request::get(url);
    if let Some(token) = auth {
        req.headers.insert("Authorization".to_string(), format!("Bearer {}", token));
    }
    req
}

fn delete_with_auth(url: String, auth: &str) -> ehttp::Request {
    #[cfg(target_arch = "wasm32")]
    {
        ehttp::Request {
            method: "DELETE".to_string(),
            url,
            body: Vec::new(),
            headers: ehttp::Headers::new(&[("Authorization", &format!("Bearer {}", auth))]),
            mode: ehttp::Mode::Cors,
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        ehttp::Request {
            method: "DELETE".to_string(),
            url,
            body: Vec::new(),
            headers: ehttp::Headers::new(&[("Authorization", &format!("Bearer {}", auth))]),
        }
    }
}

// ============================================================================
// ApiClient - single implementation for both native and WASM
// ============================================================================

/// HTTP API client for interacting with the multiplayer-kit server's REST endpoints.
///
/// Works on both native (using system HTTP) and WASM (using browser fetch).
pub struct ApiClient {
    base_url: String,
}

impl ApiClient {
    /// Create a new API client pointing to the given server base URL.
    ///
    /// Example: `ApiClient::new("http://127.0.0.1:8080")`
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Request a ticket from the server.
    ///
    /// The `auth_payload` is application-specific and will be validated by
    /// the server's configured auth handler.
    pub async fn get_ticket<T: Serialize>(
        &self,
        auth_payload: &T,
    ) -> Result<TicketResponse, ClientError> {
        let url = format!("{}/ticket", self.base_url);
        let body = serde_json::to_vec(auth_payload)
            .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

        let request = post_json(url, body, None);
        let response = fetch(request).await?;
        parse_response(response)
    }

    /// Get the server's self-signed certificate hash (base64-encoded SHA-256).
    ///
    /// Returns `None` if the server doesn't expose a cert hash (e.g., using real certs).
    pub async fn get_cert_hash(&self) -> Result<Option<String>, ClientError> {
        let url = format!("{}/cert-hash", self.base_url);
        let request = ehttp::Request::get(url);
        let response = fetch(request).await?;
        let data: Option<CertHashResponse> = parse_optional_response(response)?;
        Ok(data.map(|r| r.hash))
    }

    /// List all available rooms. Requires a valid ticket.
    pub async fn list_rooms(&self, ticket: &str) -> Result<Vec<RoomInfo>, ClientError> {
        let url = format!("{}/rooms", self.base_url);
        let request = get_with_auth(url, Some(ticket));
        let response = fetch(request).await?;
        parse_response(response)
    }

    /// Create a new room. Requires a valid ticket in the Authorization header.
    ///
    /// `config` should be a serializable room config (e.g., `{"name": "My Room"}`).
    pub async fn create_room<C: Serialize>(
        &self,
        ticket: &str,
        config: &C,
    ) -> Result<CreateRoomResponse, ClientError> {
        let url = format!("{}/rooms", self.base_url);
        let body = serde_json::to_vec(config)
            .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?;

        let request = post_json(url, body, Some(ticket));
        let response = fetch(request).await?;
        parse_response(response)
    }

    /// Delete a room. Requires a valid ticket in the Authorization header.
    pub async fn delete_room(&self, ticket: &str, room_id: RoomId) -> Result<(), ClientError> {
        let url = format!("{}/rooms/{}", self.base_url, room_id.0);
        let request = delete_with_auth(url, ticket);
        let response = fetch(request).await?;

        if !response.ok {
            let status = response.status;
            let body = response.text().unwrap_or_default();
            return Err(ClientError::Connection(ConnectionError::ServerRejected(
                format!("{}: {}", status, body),
            )));
        }

        Ok(())
    }

    /// Quickplay - find or create a room.
    ///
    /// `filter` is optional game-specific criteria for finding a room.
    pub async fn quickplay<F: Serialize>(
        &self,
        ticket: &str,
        filter: Option<&F>,
    ) -> Result<QuickplayResponse, ClientError> {
        let url = format!("{}/quickplay", self.base_url);

        let body = match filter {
            Some(f) => serde_json::to_vec(f)
                .map_err(|e| ClientError::Connection(ConnectionError::Transport(e.to_string())))?,
            None => b"{}".to_vec(),
        };

        let request = post_json(url, body, Some(ticket));
        let response = fetch(request).await?;
        parse_response(response)
    }
}

// ============================================================================
// WASM bindings
// ============================================================================

/// HTTP API client for JavaScript.
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
#[wasm_bindgen]
pub struct JsApiClient {
    inner: ApiClient,
}

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
#[wasm_bindgen]
impl JsApiClient {
    /// Create a new API client. Example: `new JsApiClient("http://127.0.0.1:8080")`
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: &str) -> JsApiClient {
        JsApiClient {
            inner: ApiClient::new(base_url),
        }
    }

    /// Request a ticket from the server. Pass your auth payload as a JS object.
    #[wasm_bindgen(js_name = getTicket)]
    pub async fn get_ticket(&self, auth_payload: JsValue) -> Result<JsValue, JsError> {
        let payload: serde_json::Value = serde_wasm_bindgen::from_value(auth_payload)
            .map_err(|e| JsError::new(&e.to_string()))?;

        let resp = self
            .inner
            .get_ticket(&payload)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&resp).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Get the server's certificate hash (base64 SHA-256) for self-signed certs.
    #[wasm_bindgen(js_name = getCertHash)]
    pub async fn get_cert_hash(&self) -> Result<Option<String>, JsError> {
        self.inner
            .get_cert_hash()
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// List all available rooms.
    #[wasm_bindgen(js_name = listRooms)]
    pub async fn list_rooms(&self, ticket: &str) -> Result<JsValue, JsError> {
        let rooms = self
            .inner
            .list_rooms(ticket)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&rooms).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Create a new room with config. Returns { room_id: number }.
    #[wasm_bindgen(js_name = createRoom)]
    pub async fn create_room(&self, ticket: &str, config: JsValue) -> Result<JsValue, JsError> {
        let config: serde_json::Value = serde_wasm_bindgen::from_value(config)
            .map_err(|e| JsError::new(&format!("Invalid config: {}", e)))?;

        let resp = self
            .inner
            .create_room(ticket, &config)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&resp).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Delete a room.
    #[wasm_bindgen(js_name = deleteRoom)]
    pub async fn delete_room(&self, ticket: &str, room_id: u32) -> Result<(), JsError> {
        self.inner
            .delete_room(ticket, RoomId(room_id as u64))
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Quickplay - find or create a room.
    /// Returns { room_id: number, created: boolean }.
    #[wasm_bindgen]
    pub async fn quickplay(&self, ticket: &str, filter: JsValue) -> Result<JsValue, JsError> {
        let filter_opt: Option<serde_json::Value> = if filter.is_null() || filter.is_undefined() {
            None
        } else {
            Some(
                serde_wasm_bindgen::from_value(filter)
                    .map_err(|e| JsError::new(&format!("Invalid filter: {}", e)))?,
            )
        };

        let resp = self
            .inner
            .quickplay(ticket, filter_opt.as_ref())
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        serde_wasm_bindgen::to_value(&resp).map_err(|e| JsError::new(&e.to_string()))
    }
}
