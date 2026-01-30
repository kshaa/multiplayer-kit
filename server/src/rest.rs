//! REST API handlers for actix-web.

use crate::lobby::Lobby;
use crate::room::RoomManager;
use crate::ticket::TicketManager;
use crate::{AuthFuture, AuthRequest};
use actix_web::{web, HttpRequest, HttpResponse};
use multiplayer_kit_protocol::{RoomId, UserContext};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared application state for REST handlers.
pub struct AppState<T: UserContext> {
    pub room_manager: Arc<RoomManager<T>>,
    pub ticket_manager: Arc<TicketManager>,
    pub lobby: Arc<Lobby>,
    pub auth_handler: Arc<dyn Fn(AuthRequest) -> AuthFuture<T> + Send + Sync>,
    pub cert_hash: Arc<tokio::sync::RwLock<Option<String>>>,
}

#[derive(Deserialize)]
pub struct CreateRoomRequest {
    pub metadata: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct CreateRoomResponse {
    pub room_id: u64,
}

#[derive(Serialize)]
pub struct TicketResponse {
    pub ticket: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// POST /ticket - Issue a new JWT ticket.
pub async fn issue_ticket<T: UserContext>(
    req: HttpRequest,
    body: web::Bytes,
    state: web::Data<AppState<T>>,
) -> HttpResponse {
    // Extract headers
    let headers = req
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    let auth_request = AuthRequest {
        headers,
        body: if body.is_empty() {
            None
        } else {
            Some(body.to_vec())
        },
    };

    // Call user's auth handler
    let user = match (state.auth_handler)(auth_request).await {
        Ok(user) => user,
        Err(reason) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: format!("{:?}", reason),
            });
        }
    };

    // Issue ticket
    match state.ticket_manager.issue(user) {
        Ok(ticket) => HttpResponse::Ok().json(TicketResponse { ticket }),
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: e.to_string(),
        }),
    }
}

/// POST /rooms - Create a new room.
pub async fn create_room<T: UserContext>(
    req: HttpRequest,
    body: web::Json<CreateRoomRequest>,
    state: web::Data<AppState<T>>,
) -> HttpResponse {
    // Extract ticket from Authorization header
    let ticket = match req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(t) => t,
        None => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "Missing Authorization header".to_string(),
            });
        }
    };

    // Validate ticket
    let user: T = match state.ticket_manager.validate(ticket) {
        Ok(u) => u,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "Invalid ticket".to_string(),
            });
        }
    };

    // Create room
    let room_id = state.room_manager.create_room(&user, body.metadata.clone());

    // Notify lobby
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_created(info);
    }

    HttpResponse::Ok().json(CreateRoomResponse {
        room_id: room_id.0,
    })
}

/// DELETE /rooms/{id} - Delete a room (creator only).
pub async fn delete_room<T: UserContext>(
    req: HttpRequest,
    path: web::Path<u64>,
    state: web::Data<AppState<T>>,
) -> HttpResponse {
    let room_id = RoomId(*path);

    // Extract ticket from Authorization header
    let ticket = match req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(t) => t,
        None => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "Missing Authorization header".to_string(),
            });
        }
    };

    // Validate ticket
    let user: T = match state.ticket_manager.validate(ticket) {
        Ok(u) => u,
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "Invalid ticket".to_string(),
            });
        }
    };

    // Delete room
    match state.room_manager.delete_room(room_id, &user.id()) {
        Ok(()) => {
            state.lobby.notify_room_deleted(room_id);
            HttpResponse::Ok().json(serde_json::json!({"deleted": true}))
        }
        Err(e) => HttpResponse::Forbidden().json(ErrorResponse {
            error: e.to_string(),
        }),
    }
}

/// GET /rooms - List all rooms (optional, lobby QUIC does this live).
pub async fn list_rooms<T: UserContext>(state: web::Data<AppState<T>>) -> HttpResponse {
    let rooms = state.room_manager.get_all_rooms();
    HttpResponse::Ok().json(rooms)
}

/// GET /cert-hash - Get the server's certificate hash for WebTransport.
pub async fn get_cert_hash<T: UserContext>(state: web::Data<AppState<T>>) -> HttpResponse {
    let hash = state.cert_hash.read().await;
    match hash.as_ref() {
        Some(h) => HttpResponse::Ok().json(serde_json::json!({ "hash": h })),
        None => HttpResponse::ServiceUnavailable().json(ErrorResponse {
            error: "Certificate hash not yet available".to_string(),
        }),
    }
}
