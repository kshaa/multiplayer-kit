//! Async task spawning abstractions.

mod spawner;

#[cfg(feature = "client")]
mod tokio;

#[cfg(feature = "client")]
mod local_set;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
mod wasm;

pub use spawner::Spawner;

#[cfg(feature = "client")]
pub use self::tokio::TokioSpawner;

#[cfg(feature = "client")]
pub use local_set::LocalSetSpawner;

#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub use wasm::WasmSpawner;
