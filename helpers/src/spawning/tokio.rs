//! Tokio-based spawner that creates a new thread for local context.

use super::Spawner;
use crate::utils::MaybeSend;

/// Tokio-based spawner for native platforms.
///
/// Creates a new thread with its own runtime for `spawn_with_local_context`.
/// Use `LocalSetSpawner` if you're already inside a LocalSet.
#[derive(Clone, Copy)]
pub struct TokioSpawner;

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
        tokio::task::spawn_local(future);
    }

    fn yield_now(&self) -> impl std::future::Future<Output = ()> + Send {
        tokio::task::yield_now()
    }
}
