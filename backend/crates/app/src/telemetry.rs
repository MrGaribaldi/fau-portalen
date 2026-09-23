//! Structured JSON logging (design section 10), installed once at startup.
//!
//! JSON to stdout is the only log transport this crate has: every line carries a
//! conventional top-level `level` (`TRACE`/`DEBUG`/`INFO`/`WARN`/`ERROR`), which is
//! exactly what Alloy's `stage.json` promotes to a Loki label with no configuration
//! on either side. Logs are never exported over OTLP -- Alloy's OTLP receiver would
//! then collect each line twice, and OTLP stays reserved for traces (spec section
//! 10) -- so this module adds no OTLP log appender and never will; a
//! `no_log_is_exported_over_otlp` test enforces that by scanning this crate's own
//! `Cargo.toml`.
//!
//! # Why a hand-rolled `Layer`, not `tracing_subscriber::fmt`'s built-in JSON
//!
//! The global constraints require `service_version` on *every* line, with no
//! reliance on a call site remembering to pass it, and `request_id` on every line
//! logged while handling a request -- including ones (like `readiness`'s probe
//! warning) that have no idea a request triggered them and no `AppState` to read a
//! version from. `tracing_subscriber::fmt`'s built-in `Json` formatter cannot do
//! this: with `flatten_event(true)` it flattens only the *event's own* fields to the
//! top level, and puts a span's fields in a nested `"span"`/`"spans"` object instead
//! (confirmed by reading `tracing-subscriber`'s own `fmt::format::json` source) --
//! so a value that lives on a span, not the event, can never land as a top-level
//! field the way this line needs it to.
//!
//! [`JsonLineLayer`] instead writes its own flat JSON object per event: it always
//! inserts `service_version` from its own field (constant for the process, set once
//! here), and merges every field recorded on every span currently in scope (root to
//! leaf) before the event's own fields. `http::request_context::middleware` enters a
//! `request_id`-carrying span around each request, so any log line emitted while
//! that span is active -- the access log itself, a readiness warning, a caught
//! handler panic -- picks it up automatically, with nothing at the call site to
//! forget.

use std::io::Write;

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer as TimeWriter;
use tracing_subscriber::fmt::time::{FormatTime, SystemTime as FmtSystemTime};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// `sqlx` logs every query it runs -- parameters included -- at `INFO`/`DEBUG`
/// through `tracing` (design section 10's "never logged: ... SQL parameters"). This
/// keeps `sqlx` at `WARN` regardless of `LOG_LEVEL`, while still letting an operator
/// override it explicitly (an `EnvFilter` directive naming a target more
/// specifically always wins over a less specific one for the same target, so a
/// `LOG_LEVEL` that itself named `sqlx=...` would still take effect over this
/// default).
const SQLX_DEFAULT_DIRECTIVE: &str = "sqlx=warn";

/// The dedicated `tracing` target `http::request_context::middleware` creates its
/// per-request span under, given its own always-on directive below, independent of
/// `LOG_LEVEL`. Fix round 2, item 1: `EnvFilter` disables *span creation itself*
/// when a span's own level does not pass the filter -- at `LOG_LEVEL=warn` or
/// `error`, an ordinary `info_span!` is never created at all, so no event nested
/// inside it, however loud, could ever pick up its `request_id` field (confirmed
/// against the live binary: a WARN logged during a request lost `request_id`
/// whenever `LOG_LEVEL` was `warn` or stricter). Pinning this target to `trace`
/// keeps the span itself always enabled; each log line inside it is still filtered
/// normally by its own level -- this only stops the *span* from disappearing.
pub(crate) const REQUEST_SPAN_TARGET: &str = "fau_request_span";
const REQUEST_SPAN_DIRECTIVE: &str = "fau_request_span=trace";

/// Installs the process-wide JSON subscriber and a panic hook that reports through
/// it. `log_level` feeds the `EnvFilter` alongside [`SQLX_DEFAULT_DIRECTIVE`] and
/// [`REQUEST_SPAN_DIRECTIVE`]; it is assumed already validated *and trimmed* --
/// `config::ServeConfig::from_env` parses it as a `LevelFilter` and fails startup by
/// variable name before any subscriber exists to log through, and stores the
/// trimmed value, not the raw one, so a value like `"info "` cannot reach
/// `EnvFilter` untrimmed here (an untrimmed bare word does not fail to parse -- it is
/// read as a *target name* enabling only that literal, bogus target, which silently
/// turns off ordinary application logging; see `config.rs`'s doc comment on that
/// field for the full explanation).
pub fn init(log_level: &str, service_version: &'static str) {
    let filter = EnvFilter::try_new(format!(
        "{SQLX_DEFAULT_DIRECTIVE},{REQUEST_SPAN_DIRECTIVE},{log_level}"
    ))
    .expect("LOG_LEVEL was already validated and trimmed by config::ServeConfig::from_env");

    tracing_subscriber::registry()
        .with(filter)
        .with(JsonLineLayer::new(service_version))
        .init();

    install_panic_hook();
}

/// Fix round 2, item 2: `tower_http::CatchPanicLayer` (`http::router`) stops a
/// panic from crashing the process and answers the request with our own JSON error
/// contract, but it does not touch Rust's *panic hook* -- the default hook still
/// runs first, before any unwinding starts, and prints
/// `thread '...' panicked at ...:<payload>` as plain text straight to stderr, which
/// both breaks "JSON to stdout is the only log transport" and -- since `<payload>`
/// can be arbitrary data a handler was working with -- is exactly the kind of leak
/// design section 15 tests for. Replacing the hook entirely (never chaining to the
/// default one) is what stops that text from ever being printed. `location` (file
/// and line, never the payload) is safe to log: it identifies a place in *our own
/// source*, not any data a request carried. `request_id` is not passed explicitly --
/// this event is logged from inside the same call stack the panic occurred on,
/// before any unwinding, so the ambient per-request span (if any) is still active
/// and `JsonLineLayer` merges it in the same way it would for any other line.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".to_owned());
        tracing::error!(location, "a thread panicked");
    }));
}

/// One span's own recorded fields (from the span's creation attributes and any later
/// `Span::record`), stored in the span's extensions by [`JsonLineLayer::on_new_span`]
/// / [`JsonLineLayer::on_record`] and read back by [`JsonLineLayer::on_event`] for
/// every event while that span is an ancestor of the current one.
#[derive(Default)]
struct SpanFields(Map<String, Value>);

/// Collects one `tracing` field set into a flat JSON object -- shared by span
/// creation/update and by event recording, so there is exactly one place that
/// decides how a `tracing` value becomes a `serde_json::Value`.
struct JsonVisitor<'a>(&'a mut Map<String, Value>);

impl Visit for JsonVisitor<'_> {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), Value::from(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_owned(), Value::from(format!("{value:?}")));
    }
}

/// Writes one JSON object per event, directly to stdout. See the module doc comment
/// for why this exists instead of `tracing_subscriber::fmt`'s built-in JSON
/// formatter.
struct JsonLineLayer {
    service_version: &'static str,
}

impl JsonLineLayer {
    fn new(service_version: &'static str) -> Self {
        Self { service_version }
    }
}

impl<S> Layer<S> for JsonLineLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let mut fields = Map::new();
        attrs.record(&mut JsonVisitor(&mut fields));
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(fields));
        }
    }

    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut extensions = span.extensions_mut();
        if let Some(SpanFields(fields)) = extensions.get_mut::<SpanFields>() {
            values.record(&mut JsonVisitor(fields));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // Root to leaf, so a more specific (inner) span's field wins over an outer
        // one sharing the same name, and the event's own fields (merged last, just
        // below) win over both -- the least surprising precedence, though nothing
        // in this application currently relies on overriding a span field.
        let mut fields = Map::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(SpanFields(span_fields)) = span.extensions().get::<SpanFields>() {
                    for (key, value) in span_fields {
                        fields.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        event.record(&mut JsonVisitor(&mut fields));

        // `fields` (span fields, then the event's own) is extended into `line`
        // *first*, and `timestamp`/`level`/`service_version` are inserted
        // *afterwards* -- `Map::insert` overwrites an existing key, so this order is
        // load-bearing: it is what makes these three fields impossible for any span
        // or event to spoof by recording a field of the same name. The reverse order
        // (insert these three, then extend with `fields`) would let a same-named
        // event field silently win instead, which is exactly backwards from what
        // "always this layer's own field, never the event's" requires.
        let mut line = Map::new();
        line.extend(fields);
        line.insert("timestamp".to_owned(), Value::from(now()));
        line.insert(
            "level".to_owned(),
            Value::from(event.metadata().level().to_string()),
        );
        line.insert(
            "service_version".to_owned(),
            Value::from(self.service_version),
        );

        if let Ok(text) = serde_json::to_string(&Value::Object(line)) {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{text}");
        }
    }
}

/// An RFC 3339 / ISO 8601 UTC timestamp, reusing `tracing-subscriber`'s own
/// `SystemTime` timer (the same one its built-in formatters use) rather than adding
/// a date/time dependency this crate does not otherwise need.
fn now() -> String {
    let mut buf = String::new();
    let mut writer = TimeWriter::new(&mut buf);
    let _ = FmtSystemTime.format_time(&mut writer);
    buf
}
