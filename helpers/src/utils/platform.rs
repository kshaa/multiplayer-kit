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
/// Similar to `GameServerContext`, this provides a way to inject
/// game-specific extras into client actors.
///
/// On native, requires Send + Sync for thread safety.
/// On WASM, no Send/Sync required (single-threaded).
///
/// # Example
///
/// ```ignore
/// struct MyClientContext {
///     sound_player: SoundPlayer,
/// }
///
/// struct MyExtras {
///     sound_player: SoundPlayer,
/// }
///
/// impl GameClientContext for MyClientContext {
///     type Extras = MyExtras;
///
///     fn get_extras(&self) -> Self::Extras {
///         MyExtras { sound_player: self.sound_player.clone() }
///     }
/// }
/// ```
pub trait GameClientContext: MaybeSend + MaybeSync + 'static {
    /// Extra tools/services passed to actor's handle and shutdown methods.
    type Extras: MaybeSend + 'static;

    /// Get extras for the actor.
    fn get_extras(&self) -> Self::Extras;
}

impl GameClientContext for () {
    type Extras = ();

    fn get_extras(&self) -> Self::Extras {}
}

// Blanket impls for shared pointers
#[cfg(not(target_arch = "wasm32"))]
impl<T: GameClientContext> GameClientContext for std::sync::Arc<T> {
    type Extras = T::Extras;

    fn get_extras(&self) -> Self::Extras {
        (**self).get_extras()
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: GameClientContext> GameClientContext for std::rc::Rc<T> {
    type Extras = T::Extras;

    fn get_extras(&self) -> Self::Extras {
        (**self).get_extras()
    }
}
