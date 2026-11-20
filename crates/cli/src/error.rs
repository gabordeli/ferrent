//! CLI error type with stable exit codes.

use std::fmt;

use tonic::Code;

/// Errors surfaced by the CLI. Exit codes are part of the documented
/// interface (see `docs/cli.md`).
#[derive(Debug)]
pub enum CliError {
    Config(String),
    Io(std::io::Error),
    /// The daemon could not be reached. The address travels with the error so
    /// both the message and the telemetry report name the target, and the
    /// boxed cause keeps the reason (connection refused, TLS, ...) reachable.
    Transport {
        server: String,
        error: Box<dyn std::error::Error + Send + Sync>,
    },
    Status(tonic::Status),
}

impl CliError {
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Config(_) => 2,
            Self::Io(_) => 1,
            Self::Transport { .. } => 5,
            Self::Status(status) => match status.code() {
                Code::NotFound => 3,
                Code::Unauthenticated | Code::PermissionDenied => 4,
                _ => 1,
            },
        }
    }

    /// Whether this is a defect worth a telemetry report.
    ///
    /// A daemon that is not running, a typo in a torrent id or a broken pipe
    /// from `| head` are normal answers to how the CLI is used; reporting them
    /// turns the issue tracker into noise. Only unexpected failures are sent.
    #[must_use]
    pub fn reportable(&self) -> bool {
        match self {
            Self::Config(_) => true,
            Self::Io(error) => error.kind() != std::io::ErrorKind::BrokenPipe,
            Self::Transport { .. } => false,
            Self::Status(status) => matches!(
                status.code(),
                Code::Internal | Code::Unknown | Code::DataLoss | Code::Unimplemented
            ),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => formatter.write_str(message),
            Self::Transport { server, error } => {
                write!(formatter, "cannot connect to {server}: {error}")?;
                // tonic's Display is only "transport error"; the cause
                // (connection refused, TLS handshake, ...) is in the chain.
                let mut source = std::error::Error::source(error.as_ref());
                while let Some(cause) = source {
                    write!(formatter, ": {cause}")?;
                    source = cause.source();
                }
                Ok(())
            }
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Status(status) => {
                write!(formatter, "{} ({:?})", status.message(), status.code())
            }
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Transport { error, .. } => Some(error.as_ref()),
            Self::Status(status) => Some(status),
            Self::Config(_) => None,
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<tonic::Status> for CliError {
    fn from(status: tonic::Status) -> Self {
        Self::Status(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_answers_are_not_reported() {
        // A daemon that is down, a typo in an id, and `| head` are normal.
        assert!(!CliError::Status(tonic::Status::not_found("torrent 1 not found")).reportable());
        assert!(!CliError::Status(tonic::Status::unavailable("busy")).reportable());
        assert!(!CliError::Io(std::io::Error::from(std::io::ErrorKind::BrokenPipe)).reportable());
        // Real defects still are.
        assert!(CliError::Status(tonic::Status::internal("boom")).reportable());
        assert!(CliError::Io(std::io::Error::from(std::io::ErrorKind::NotFound)).reportable());
        assert!(CliError::Config("bad config".into()).reportable());
    }

    #[test]
    fn transport_error_names_the_server_and_the_cause() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let error = runtime.block_on(async {
            tonic::transport::Endpoint::from_static("http://127.0.0.1:1")
                .connect()
                .await
                .expect_err("port 1 is closed")
        });
        let cli = CliError::Transport {
            server: "127.0.0.1:1".into(),
            error: Box::new(error),
        };
        let text = cli.to_string();
        assert!(
            text.starts_with("cannot connect to 127.0.0.1:1: "),
            "message should name the target: {text}"
        );
        assert!(
            text.len() > "cannot connect to 127.0.0.1:1: transport error".len(),
            "the cause should be in the message: {text}"
        );
        assert_eq!(cli.exit_code(), 5);
        assert!(!cli.reportable());
    }
}
