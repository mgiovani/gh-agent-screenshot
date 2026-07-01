pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(
        "Not authenticated with GitHub. Run 'gh auth login' to set up credentials, then retry."
    )]
    AuthFailure,

    #[error(
        "Image file '{path}' not found. Check that the path exists and is readable, then retry."
    )]
    FileNotFound { path: String },

    #[error(
        "GitHub API call failed (HTTP {status}): {message}. Verify your token has the required \
         permissions and the repository exists."
    )]
    ApiError { status: u16, message: String },

    #[error(
        "Issue #{number} not found in {repo}. Verify the repository and issue number are correct, \
         then retry."
    )]
    IssueNotFound { repo: String, number: u32 },

    #[error(
        "GitHub API call failed (HTTP 429). You may be rate-limited. Wait a moment and retry. \
         Retry after: {retry_after:?} seconds."
    )]
    RateLimited { retry_after: Option<u64> },

    #[error(
        "Network error communicating with GitHub: {0}. Check your internet connection and retry."
    )]
    Network(#[from] reqwest::Error),

    #[error("Failed to read or write a local file: {0}. Check permissions and disk space.")]
    Io(#[from] std::io::Error),

    #[error(
        "Upload failed after blob transfer at stage '{stage}'. The partial upload is harmless \
         and will be cleaned up by GitHub. Retry the full command."
    )]
    PartialUpload { stage: String },

    #[error("Failed to parse GitHub API response: {0}. This is likely a bug; please report it.")]
    Serde(#[from] serde_json::Error),

    #[error(
        "The repository appears to be empty (no commits yet). Push an initial commit before \
         uploading images, then retry."
    )]
    EmptyRepo,
}
