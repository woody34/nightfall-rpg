//! Trace context across queues.
//!
//! A `MoveTo` crosses three tasks: the socket reader, the zone actor, the broadcaster. Spans do
//! not follow a message through a channel on their own. Capture a [`TraceCarrier`] where the
//! message is created, put it in the message, and parent the consumer's span on it:
//!
//! ```ignore
//! // socket task
//! let cmd = ZoneCommand { intent, trace: TraceCarrier::current() };
//! tx.send(cmd).await?;
//!
//! // zone actor
//! let span = tracing::info_span!(parent: cmd.trace.span(), "zone.apply_move");
//! async { /* ... */ }.instrument(span).await;
//! ```
//!
//! The carrier holds a `tracing::Span`, so the whole chain lands in one trace with the original
//! request as root. It is in-process only; it is not serialised into the event log.
//! Holding the carrier keeps the span open, so drop commands promptly.

use tracing::Span;

/// The span active where a message was created. Cheap to clone.
#[derive(Debug, Clone)]
pub struct TraceCarrier {
    span: Span,
}

impl TraceCarrier {
    /// Captures the current span (a disabled span if none is active).
    #[must_use]
    pub fn current() -> Self {
        Self {
            span: Span::current(),
        }
    }

    /// The captured span, for `info_span!(parent: carrier.span(), ...)`.
    #[must_use]
    pub fn span(&self) -> &Span {
        &self.span
    }

    /// Runs `f` with the captured span as the current span.
    pub fn in_scope<R>(&self, f: impl FnOnce() -> R) -> R {
        self.span.in_scope(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_span_is_parented_on_the_carried_span() {
        let _guard = tracing::subscriber::set_default(tracing_subscriber::registry());
        let root = tracing::info_span!("socket");
        let carrier = root.in_scope(TraceCarrier::current);
        // The consumer runs outside the root span, as an actor task would.
        let child = tracing::info_span!(parent: carrier.span(), "actor");
        assert_eq!(child.metadata().map(tracing::Metadata::name), Some("actor"));
        assert_eq!(carrier.span().id(), root.id());
        carrier.in_scope(|| assert_eq!(Span::current().id(), root.id()));
    }
}
