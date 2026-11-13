//! Errors for Go Max.

/// Result type for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// What can go wrong while running the node.
///
/// Messages are written to be shown as they are.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A folder or file could not be read or written.
    #[error("{what}: {source}")]
    Io {
        /// What was being done.
        what: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A node program is missing from the app.
    #[error("the node program {0} is missing. Reinstall Lock it.")]
    MissingProgram(String),

    /// A program could not be started.
    #[error("{program} could not be started: {source}")]
    Spawn {
        /// Which program.
        program: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A program stopped by itself while it should be running.
    #[error("{program} stopped unexpectedly (exit code {code:?}). Its log is in {log}.")]
    Exited {
        /// Which program.
        program: String,
        /// Its exit code, if any.
        code: Option<i32>,
        /// Where its log is.
        log: String,
    },

    /// The node did not become ready in time.
    #[error("{0} did not become ready in time")]
    NotReady(String),

    /// The node's RPC answered with an error, or not at all.
    #[error("the node did not answer: {0}")]
    Rpc(String),

    /// Go Max is already running.
    #[error("Go Max is already running")]
    AlreadyRunning,

    /// The operating system refused something.
    #[error("the system reported a problem: {0}")]
    Platform(String),
}

impl Error {
    pub(crate) fn io(what: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io { what: what.into(), source }
    }
}
