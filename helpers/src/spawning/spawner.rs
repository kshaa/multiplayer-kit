//! Task spawner abstraction for cross-platform async code.
//!
//! Provides the `Spawner` trait and built-in implementations for tokio and WASM.

use crate::utils::{MaybeSend, MaybeSync};

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
///     fn spawn_with_local_context<F>(&self, future: F)
///     where
///         F: Future<Output = ()> + 'static,
///     {
///         // Same as spawn for Bevy since it uses spawn_local
///         bevy_tasks::AsyncComputeTaskPool::get().spawn_local(future).detach();
///     }
/// }
/// ```
pub trait Spawner: Clone + MaybeSend + MaybeSync + 'static {
    /// Spawn an async task that may be moved between threads.
    fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + MaybeSend + 'static;

    /// Spawn a future that enables `spawn_local` within it.
    ///
    /// The spawned future can call `spawn_local` to spawn `!Send` futures.
    /// On native, this sets up a LocalSet context. On WASM, this is the same as spawn.
    fn spawn_with_local_context<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + MaybeSend + 'static;

    /// Spawn a `!Send` future in the current local context.
    ///
    /// **Must be called from within a future spawned by `spawn_with_local_context`.**
    /// On WASM this always works. On native this requires being in a LocalSet.
    fn spawn_local<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + 'static;
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

    fn spawn_with_local_context<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + MaybeSend + 'static,
    {
        // Run on a dedicated thread with LocalSet for !Send futures.
        // This thread runs its own single-threaded tokio runtime.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, future);
        });
    }

    fn spawn_local<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + 'static,
    {
        // Spawn on the current LocalSet (must be called from within spawn_with_local_context)
        tokio::task::spawn_local(future);
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

    fn spawn_with_local_context<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + MaybeSend + 'static,
    {
        // WASM is single-threaded, always has local context
        wasm_bindgen_futures::spawn_local(future);
    }

    fn spawn_local<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + 'static,
    {
        // WASM is single-threaded, spawn_local always works
        wasm_bindgen_futures::spawn_local(future);
    }
}
