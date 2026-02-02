pub mod api;
pub mod error;
pub mod lobby;
pub mod room;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub mod wasm_exports;

pub use api::{ApiClient, CertHashResponse, CreateRoomResponse, TicketResponse};
pub use error::{
    ClientError, ConnectionError, ConnectionState, DisconnectReason, ReceiveError, SendError,
};
pub use lobby::LobbyClient;
pub use room::{Channel, ConnectionConfig, RoomConnection, Transport};

// Deprecated alias
#[allow(deprecated)]
pub use room::RoomClient;
