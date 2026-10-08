//! Secrets must never reach spans or logs (plan Revision 1, item 7): the play ticket travels in
//! an `Authorization` header and possibly a query string, and neither may be recorded.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use std::io;
use std::sync::Arc;

use common::TestApp;
use parking_lot::Mutex;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

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

#[tokio::test]
async fn query_string_and_authorization_header_never_appear_in_spans_or_logs() {
    const QUERY_SECRET: &str = "SENTINEL-QUERY-4f1c9a";
    const HEADER_SECRET: &str = "SENTINEL-BEARER-77d2e0";

    let capture = Capture::default();
    // Thread-local default: #[tokio::test] runs the servers on this thread.
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .json()
            .with_max_level(tracing::Level::TRACE)
            .with_current_span(true)
            .with_span_list(true)
            .with_writer(capture.clone())
            .finish(),
    );

    let app = TestApp::spawn().await;
    let res = reqwest::Client::new()
        .get(format!("{}/health?ticket={QUERY_SECRET}", app.http_base))
        .header("authorization", format!("Bearer {HEADER_SECRET}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let logged = String::from_utf8(capture.0.lock().clone()).unwrap();
    assert!(
        logged.contains("\"http.path\":\"/health\""),
        "request span not captured, test is vacuous:\n{logged}"
    );
    assert!(!logged.contains(QUERY_SECRET), "query string leaked:\n{logged}");
    assert!(!logged.contains(HEADER_SECRET), "header leaked:\n{logged}");
}
