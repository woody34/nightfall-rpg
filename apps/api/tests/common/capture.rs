//! In-memory JSON log/span capture for asserting on what the process would emit.

#![allow(dead_code, missing_docs, unreachable_pub, clippy::unwrap_used)]

use std::io;
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
pub struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    /// Installs a thread-local subscriber (all levels, spans included) that writes JSON here.
    /// `#[tokio::test]` runs everything on the test thread, so tasks it spawns are captured.
    pub fn install(&self) -> DefaultGuard {
        self.install_with(tracing_subscriber::EnvFilter::new("trace"))
    }

    /// Like [`Self::install`], filtered like production would be.
    pub fn install_with(&self, filter: tracing_subscriber::EnvFilter) -> DefaultGuard {
        tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filter)
                .with_current_span(true)
                .with_span_list(true)
                .with_writer(self.clone())
                .finish(),
        )
    }

    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().clone()).unwrap()
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
