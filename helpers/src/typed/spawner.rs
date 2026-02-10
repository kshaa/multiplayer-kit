//! Task spawner abstraction for cross-platform async code.
//!
//! Provides the `Spawner` trait and built-in implementations for tokio and WASM.

use super::platform::{MaybeSend, MaybeSync};

/// Trait for spawning async tasks.
///
/// Implement this to provide platform-specific task spawning.
/// The library provides `TokioSpawner` (native) and `WasmSpawner` (WASM).
///
/// For Bevy integration, implement this trait using `bevy_tasks`:
///
/// ```ignore
/// struct BevySpawner;
/// impl Spawner for BevySpawner {
///     fn spawn<F>(&self, future: F)
///     where
///         F: Future<Output = ()> + 'static,
///     {
///         bevy_tasks::AsyncComputeTaskPool::get().spawn_local(future).detach();
///     }
/// }
/// ```
pub trait Spawner: Clone + MaybeSend + MaybeSync + 'static {
    /// Spawn an async task.
    fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + MaybeSend + 'static;
}

/// Tokio-based spawner for native platforms.
#[cfg(feature = "client")]
#[derive(Clone, Copy)]
pub struct TokioSpawner;

#[cfg(feature = "client")]
impl Spawner for TokioSpawner {
    fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + MaybeSend + 'static,
    {
        tokio::spawn(future);
    }
}

/// WASM spawner using wasm-bindgen-futures.
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
#[derive(Clone, Copy)]
pub struct WasmSpawner;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
impl Spawner for WasmSpawner {
    fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + MaybeSend + 'static,
    {
        wasm_bindgen_futures::spawn_local(future);
    }
}
