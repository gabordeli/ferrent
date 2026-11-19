//! CLI error type with stable exit codes.

use std::fmt;

use tonic::Code;

/// Errors surfaced by the CLI. Exit codes are part of the documented
/// interface (see `docs/cli.md`).
#[derive(Debug)]
pub enum CliError {
    Config(String),
    Io(std::io::Error),
    Transport(String),
    Status(tonic::Status),
}

impl CliError {
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Config(_) => 2,
            Self::Io(_) => 1,
            Self::Transport(_) => 5,
            Self::Status(status) => match status.code() {
                Code::NotFound => 3,
                Code::Unauthenticated | Code::PermissionDenied => 4,
                _ => 1,
            },
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) | Self::Transport(message) => formatter.write_str(message),
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
            Self::Status(status) => Some(status),
            Self::Config(_) | Self::Transport(_) => None,
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
