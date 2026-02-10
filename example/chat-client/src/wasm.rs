//! WASM platform - re-exports + JS bindings only.
//!
//! All core types and functions are defined in actor.rs (platform-agnostic).
//! This file only contains WASM-specific JS interop code.

pub use crate::actor::{start_chat_actor, ChatClientContext, ChatHandle};
pub use multiplayer_kit_client::JsApiClient;

use crate::ChatClientAdapter;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

// ============================================================================
// JS bindings - wraps JS callbacks and calls the Rust actor
// ============================================================================

/// Adapter that wraps JS callbacks.
struct JsAdapter {
    on_connected: js_sys::Function,
    on_disconnected: js_sys::Function,
    on_text_message: js_sys::Function,
    on_system_message: js_sys::Function,
}

// SAFETY: JsAdapter is only used in single-threaded WASM context
unsafe impl Send for JsAdapter {}
unsafe impl Sync for JsAdapter {}

// Required because ChatClientAdapter extends GameClientContext
impl multiplayer_kit_helpers::GameClientContext for JsAdapter {}

impl ChatClientAdapter for JsAdapter {
    fn on_connected(&self) {
        let _ = self.on_connected.call0(&JsValue::NULL);
    }

    fn on_disconnected(&self) {
        let _ = self.on_disconnected.call0(&JsValue::NULL);
    }

    fn on_text_message(&self, username: &str, content: &str) {
        let _ = self.on_text_message.call2(
            &JsValue::NULL,
            &JsValue::from_str(username),
            &JsValue::from_str(content),
        );
    }

    fn on_system_message(&self, text: &str) {
        let _ = self
            .on_system_message
            .call1(&JsValue::NULL, &JsValue::from_str(text));
    }
}

// ============================================================================
// JsChatHandle - JS wrapper for ChatHandle
// ============================================================================

/// JS wrapper for ChatHandle.
///
/// Returned from `runChatActor`. Use this to send messages.
#[wasm_bindgen]
pub struct JsChatHandle {
    inner: ChatHandle,
}

#[wasm_bindgen]
impl JsChatHandle {
    /// Send a text message to the chat room.
    #[wasm_bindgen(js_name = sendText)]
    pub fn send_text(&self, content: &str) -> Result<(), JsError> {
        self.inner
            .send_text(content)
            .map_err(|e| JsError::new(&e.to_string()))
    }
}

// ============================================================================
// runChatActor - JS entry point, returns handle directly
// ============================================================================

/// Run the chat actor with JavaScript callbacks.
///
/// Returns a handle that can be used to send messages.
///
/// ```javascript
/// const handle = runChatActor(conn, {
///     onConnected: () => console.log("Connected!"),
///     onDisconnected: () => console.log("Disconnected"),
///     onTextMessage: (username, content) => console.log(`${username}: ${content}`),
///     onSystemMessage: (text) => console.log(`[system] ${text}`),
/// });
///
/// // Send a message:
/// handle.sendText("Hello!");
/// ```
#[wasm_bindgen(js_name = runChatActor)]
pub fn js_run_chat_actor(
    conn: multiplayer_kit_client::JsRoomConnection,
    adapter: JsValue,
) -> Result<JsChatHandle, JsError> {
    let rust_adapter = JsAdapter {
        on_connected: get_callback(&adapter, "onConnected")?,
        on_disconnected: get_callback(&adapter, "onDisconnected")?,
        on_text_message: get_callback(&adapter, "onTextMessage")?,
        on_system_message: get_callback(&adapter, "onSystemMessage")?,
    };

    // Extract the inner RoomConnection from the JS wrapper
    let room_conn = conn.into_inner();
    let handle = start_chat_actor(room_conn, rust_adapter);

    Ok(JsChatHandle { inner: handle })
}

fn get_callback(obj: &JsValue, name: &str) -> Result<js_sys::Function, JsError> {
    let func = js_sys::Reflect::get(obj, &JsValue::from_str(name))
        .map_err(|_| JsError::new(&format!("Missing callback: {}", name)))?;

    if func.is_undefined() {
        return Err(JsError::new(&format!("Missing callback: {}", name)));
    }

    func.dyn_into::<js_sys::Function>()
        .map_err(|_| JsError::new(&format!("{} is not a function", name)))
}
