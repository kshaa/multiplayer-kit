//! Spawner trait definition.

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

    /// Yield control to let other tasks run.
    ///
    /// On native, this yields the current tokio task. On WASM, this is a no-op
    /// since the single-threaded runtime handles scheduling automatically.
    fn yield_now(&self) -> impl std::future::Future<Output = ()> + Send;
}
