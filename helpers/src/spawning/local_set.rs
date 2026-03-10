//! Tokio spawner that stays on the current LocalSet.

use super::Spawner;
use crate::utils::MaybeSend;

/// Tokio spawner that stays on the current LocalSet.
///
/// Use this when you're already running inside a LocalSet and want all tasks
/// to stay there. Important for QUIC streams which are tied to their runtime.
#[derive(Clone, Copy)]
pub struct LocalSetSpawner;

impl Spawner for LocalSetSpawner {
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
        // Stay on the current LocalSet instead of creating a new thread.
        tokio::task::spawn_local(future);
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
