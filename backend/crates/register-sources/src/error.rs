//! Errors carry the source and a fixed kind, never a response body or an address.

/// Which public source an error came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Nsr,
    Kartverket,
    Ssb,
    Brreg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceErrorKind {
    /// The response did not match the expected shape, e.g. a required field was missing.
    /// `detail` names the missing field or the error class, plus a position, never a value.
    Parse { detail: String },
    /// A value was present but not one we accept, e.g. an unknown Kartverket language.
    UnexpectedValue { field: &'static str },
    /// A non-success HTTP status.
    Status { status: u16 },
    /// Transport failure or timeout.
    Transport,
    /// NSR paging did not end within the hard limit.
    TooManyPages,
    /// NSR paging ended without covering every unit: an empty page arrived before the claimed
    /// page count, or the deduped count did not match the claimed total. Never returned as a
    /// partial list (§5.3).
    IncompletePaging,
    /// Reading or writing the local Brreg download failed.
    Io,
}

/// Display and Error are implemented by hand: thiserror would treat a field named `source`
/// as the error's cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    pub source: Source,
    pub kind: SourceErrorKind,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {:?}", self.source, self.kind)
    }
}

impl std::error::Error for SourceError {}

impl SourceError {
    pub(crate) fn new(source: Source, kind: SourceErrorKind) -> Self {
        Self { source, kind }
    }

    /// Never serde_json's full message: an invalid-type error quotes the input's value. Keep
    /// the missing field's name, or else the error class, plus the position.
    pub(crate) fn parse(source: Source, err: &serde_json::Error) -> Self {
        if err.classify() == serde_json::error::Category::Io {
            // A read failure underneath serde_json, e.g. a corrupt gzip stream: this is not a
            // shape mismatch in the JSON, so it must not be reported as a parse error.
            return Self::new(source, SourceErrorKind::Io);
        }
        let msg = err.to_string();
        let missing = msg
            .strip_prefix("missing field `")
            .and_then(|rest| rest.split('`').next());
        let detail = match missing {
            Some(field) => format!(
                "missing field {field} at line {} column {}",
                err.line(),
                err.column()
            ),
            None => format!(
                "{:?} error at line {} column {}",
                err.classify(),
                err.line(),
                err.column()
            ),
        };
        Self::new(source, SourceErrorKind::Parse { detail })
    }
}
