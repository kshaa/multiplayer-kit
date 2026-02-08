//! REST API handlers for actix-web.

use crate::lobby::Lobby;
use crate::room::RoomManager;
use crate::ticket::TicketManager;
use crate::{AuthFuture, AuthRequest};
use actix_web::{HttpRequest, HttpResponse, web};
use multiplayer_kit_protocol::{QuickplayResponse, RoomConfig, RoomId, UserContext};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared application state for REST handlers.
pub struct AppState<T: UserContext, C: RoomConfig> {
    pub room_manager: Arc<RoomManager<T, C>>,
    pub ticket_manager: Arc<TicketManager>,
    pub lobby: Arc<Lobby>,
    pub auth_handler: Arc<dyn Fn(AuthRequest) -> AuthFuture<T> + Send + Sync>,
    pub cert_hash: Arc<tokio::sync::RwLock<Option<String>>>,
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

/// Extract ticket from Authorization header.
fn extract_ticket(req: &HttpRequest) -> Option<&str> {
    req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// POST /ticket - Issue a new JWT ticket.
pub async fn issue_ticket<T: UserContext, C: RoomConfig>(
    req: HttpRequest,
    body: web::Bytes,
    state: web::Data<AppState<T, C>>,
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
                error: reason.to_string(),
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
/// Body should be JSON matching the room config type C.
/// If C: Default and body is empty/`{}`, uses default config.
pub async fn create_room<T: UserContext, C: RoomConfig>(
    req: HttpRequest,
    body: web::Bytes,
    state: web::Data<AppState<T, C>>,
) -> HttpResponse {
    let ticket = match extract_ticket(&req) {
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

    // Parse config from body
    let config: C = if body.is_empty() {
        // Try default if body is empty
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Room config required in request body".to_string(),
        });
    } else {
        match serde_json::from_slice(&body) {
            Ok(c) => c,
            Err(e) => {
                return HttpResponse::BadRequest().json(ErrorResponse {
                    error: format!("Invalid room config: {}", e),
                });
            }
        }
    };

    // Create room (this also validates config)
    let room_id = match state.room_manager.create_room(&user, config) {
        Ok(id) => id,
        Err(e) => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: format!("Room creation failed: {}", e),
            });
        }
    };

    // Notify lobby
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_created(info);
    }

    HttpResponse::Ok().json(CreateRoomResponse { room_id: room_id.0 })
}

/// DELETE /rooms/{id} - Delete a room (creator only).
pub async fn delete_room<T: UserContext, C: RoomConfig>(
    req: HttpRequest,
    path: web::Path<u64>,
    state: web::Data<AppState<T, C>>,
) -> HttpResponse {
    let room_id = RoomId(*path);

    let ticket = match extract_ticket(&req) {
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
    match state.room_manager.delete_room(room_id, &user.id()).await {
        Ok(()) => {
            state.lobby.notify_room_deleted(room_id);
            HttpResponse::Ok().json(serde_json::json!({"deleted": true}))
        }
        Err(e) => HttpResponse::Forbidden().json(ErrorResponse {
            error: e.to_string(),
        }),
    }
}

/// GET /rooms - List all rooms (requires authentication).
pub async fn list_rooms<T: UserContext, C: RoomConfig>(
    req: HttpRequest,
    state: web::Data<AppState<T, C>>,
) -> HttpResponse {
    let ticket = match extract_ticket(&req) {
        Some(t) => t,
        None => {
            return HttpResponse::Unauthorized().json(ErrorResponse {
                error: "Missing Authorization header".to_string(),
            });
        }
    };

    // Validate ticket (we don't need the user, just verify it's valid)
    if state.ticket_manager.validate::<T>(ticket).is_err() {
        return HttpResponse::Unauthorized().json(ErrorResponse {
            error: "Invalid ticket".to_string(),
        });
    }

    let rooms = state.room_manager.get_all_rooms();
    HttpResponse::Ok().json(rooms)
}

/// POST /quickplay - Find or create a room for quickplay.
/// Body is optional game-specific filter criteria (JSON).
#[derive(Deserialize)]
pub struct QuickplayRequest {
    #[serde(flatten)]
    pub filter: serde_json::Value,
}

pub async fn quickplay<T: UserContext, C: RoomConfig>(
    req: HttpRequest,
    body: web::Bytes,
    state: web::Data<AppState<T, C>>,
) -> HttpResponse {
    let ticket = match extract_ticket(&req) {
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

    // Parse filter criteria (optional)
    let filter: serde_json::Value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)
    };

    // Try to find existing room
    if let Some(room_id) = state.room_manager.find_quickplay_room(&filter) {
        return HttpResponse::Ok().json(QuickplayResponse {
            room_id,
            created: false,
        });
    }

    // No room found - try to create one
    let config = match C::quickplay_default(&filter) {
        Some(c) => c,
        None => {
            return HttpResponse::BadRequest().json(ErrorResponse {
                error: "Quickplay not enabled for this game".to_string(),
            });
        }
    };

    let room_id = match state.room_manager.create_room(&user, config) {
        Ok(id) => id,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to create quickplay room: {}", e),
            });
        }
    };

    // Notify lobby
    if let Some(info) = state.room_manager.get_room_info(room_id) {
        state.lobby.notify_room_created(info);
    }

    HttpResponse::Ok().json(QuickplayResponse {
        room_id,
        created: true,
    })
}

/// GET /cert-hash - Get the server's certificate hash for WebTransport.
pub async fn get_cert_hash<T: UserContext, C: RoomConfig>(
    state: web::Data<AppState<T, C>>,
) -> HttpResponse {
    let hash = state.cert_hash.read().await;
    match hash.as_ref() {
        Some(h) => HttpResponse::Ok().json(serde_json::json!({ "hash": h })),
        None => HttpResponse::ServiceUnavailable().json(ErrorResponse {
            error: "Certificate hash not yet available".to_string(),
        }),
    }
}
