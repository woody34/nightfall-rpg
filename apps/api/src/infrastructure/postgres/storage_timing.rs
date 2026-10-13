//! Opt-in storage timings with fixed phase/subphase labels and no SQL or identities.
use std::future::Future;
use std::time::Instant;

pub(super) async fn timed<T>(
    phase: &'static str,
    subphase: &'static str,
    future: impl Future<Output = T>,
) -> T {
    let started = Instant::now();
    let result = future.await;
    tracing::debug!(target: "nightfall_api::runtime_timing", phase, subphase, micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX), "storage subphase");
    result
}
