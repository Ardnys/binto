use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BintoError {
    #[error("GitHub API error {status}: {message}")]
    ApiError { status: u16, message: String },

    #[error("Rate limit exceeded; resets at {reset_time}")]
    RateLimitExceeded { reset_time: String },

    #[error("No Linux-compatible assets found for {repo} {tag}")]
    NoCompatibleAssets { repo: String, tag: String },

    #[error("Checksum mismatch for {filename}: expected {expected}, got {got}")]
    ChecksumMismatch {
        filename: String,
        expected: String,
        got: String,
    },

    #[error("State file corrupted: {0}")]
    StateCorrupted(String),

    #[error("Binary not found in archive after extraction")]
    BinaryNotFoundInArchive,

    #[error("Tool '{name}' is not managed by binto")]
    UnknownTool { name: String },

    #[error("Install directory {path} does not exist and could not be created")]
    InstallDirMissing { path: PathBuf },
}
