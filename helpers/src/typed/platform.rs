//! Platform abstraction traits for cross-platform async code.
//!
//! Provides conditional Send/Sync bounds that differ between native and WASM.

// ============================================================================
// Conditional Send/Sync bounds
// ============================================================================

/// Conditional Send bound - required on native, not on WASM.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// Conditional Sync bound - required on native, not on WASM.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync> MaybeSync for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSync for T {}

// ============================================================================
// GameClientContext
// ============================================================================

/// Context trait for game-specific client services.
///
/// Implement this trait on your context type to provide game-specific
/// services (state managers, caches, etc.) to client actors.
///
/// On native, requires Send + Sync for thread safety.
/// On WASM, no Send/Sync required (single-threaded).
///
/// # Example
///
/// ```ignore
/// struct MyClientContext {
///     game_state: Arc<RwLock<GameState>>,
///     sound_player: SoundPlayer,
/// }
///
/// impl GameClientContext for MyClientContext {}
/// ```
pub trait GameClientContext: MaybeSend + MaybeSync + 'static {}

impl GameClientContext for () {}

// Blanket impls for shared pointers
#[cfg(not(target_arch = "wasm32"))]
impl<T: GameClientContext> GameClientContext for std::sync::Arc<T> {}

#[cfg(target_arch = "wasm32")]
impl<T: GameClientContext> GameClientContext for std::rc::Rc<T> {}
