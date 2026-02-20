//! Async task spawning abstractions.

mod spawner;

pub use spawner::Spawner;

#[cfg(feature = "client")]
pub use spawner::TokioSpawner;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use spawner::WasmSpawner;
