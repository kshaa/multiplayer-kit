//! WASM spawner using wasm-bindgen-futures.

use super::Spawner;
use crate::utils::MaybeSend;

/// WASM spawner using wasm-bindgen-futures.
#[derive(Clone, Copy)]
pub struct WasmSpawner;

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
        wasm_bindgen_futures::spawn_local(future);
    }

    fn yield_now(&self) -> impl std::future::Future<Output = ()> + Send {
        // WASM is single-threaded, no need to yield
        std::future::ready(())
    }
}
