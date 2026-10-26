//! Strict media manifests, read-only inventories and optical operations.
pub mod device;
pub mod manifest;
pub mod optical;
pub mod plan;
pub mod progress;
pub mod ventoy;

/// Stable, concise operation errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usage: {0}")]
    Usage(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("unsupported capability: {0}")]
    Unsupported(String),
    #[error("verification failed: {0}")]
    Verification(String),
    #[error("resource limit: {0}")]
    Resource(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("{source}; recovery artifact: {}", path.display())]
    Recovery {
        path: std::path::PathBuf,
        #[source]
        source: Box<Error>,
    },
    #[error("optical image: {0}")]
    Optical(String),
}
impl Error {
    /// Machine-readable error category.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Recovery { source, .. } => source.code(),
            Self::Usage(_) => "usage",
            Self::InvalidInput(_) => "invalid_input",
            Self::Unsupported(_) => "unsupported_capability",
            Self::Verification(_) => "verification_failed",
            Self::Resource(_) => "resource_exhaustion",
            Self::Cancelled => "cancelled",
            Self::Io(_) => "io_failure",
            Self::Optical(_) => "invalid_image",
        }
    }
    /// Stable CLI exit status (usage errors use 2).
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Recovery { source, .. } => source.exit_code(),
            Self::Usage(_) => 2,
            Self::InvalidInput(_) | Self::Optical(_) => 3,
            Self::Unsupported(_) => 4,
            Self::Io(_) => 5,
            Self::Verification(_) => 6,
            Self::Resource(_) => 7,
            Self::Cancelled => 130,
        }
    }
}
/// Library operation result.
pub type Result<T> = std::result::Result<T, Error>;
