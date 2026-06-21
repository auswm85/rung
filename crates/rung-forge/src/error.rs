//! Error types for forge (code-hosting) operations.
//!
//! [`ForgeError`] is the neutral error returned by every [`crate::ForgeApi`]
//! implementation, so callers handle the same error type regardless of which
//! backend (GitHub, GitLab, …) produced it.

/// Result type alias using [`ForgeError`].
pub type Result<T> = std::result::Result<T, ForgeError>;

/// Errors that can occur during forge API operations.
#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    /// Authentication failed or token missing.
    #[error("forge authentication failed - check your access token")]
    AuthenticationFailed,

    /// Token not found.
    #[error("no forge token found - configure authentication for your forge")]
    NoToken,

    /// API rate limit exceeded.
    #[error("forge API rate limit exceeded - wait and try again")]
    RateLimited,

    /// Repository not found or no access.
    #[error("repository not found or no access: {0}")]
    RepoNotFound(String),

    /// PR/MR not found.
    #[error("pull request not found: #{0}")]
    PrNotFound(u64),

    /// API error with status code.
    #[error("forge API error ({status}): {message}")]
    ApiError {
        /// HTTP status code returned by the forge.
        status: u16,
        /// Error message returned by the forge.
        message: String,
    },

    /// Network error.
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// JSON parsing error.
    #[error("failed to parse forge response: {0}")]
    Parse(#[from] serde_json::Error),

    /// IO error (e.g., reading a CLI token).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_display() {
        let err = ForgeError::ApiError {
            status: 422,
            message: "Validation failed".to_string(),
        };
        assert_eq!(err.to_string(), "forge API error (422): Validation failed");
    }

    #[test]
    fn test_pr_not_found_display() {
        let err = ForgeError::PrNotFound(42);
        assert_eq!(err.to_string(), "pull request not found: #42");
    }

    #[test]
    fn test_repo_not_found_display() {
        let err = ForgeError::RepoNotFound("owner/repo".to_string());
        assert_eq!(
            err.to_string(),
            "repository not found or no access: owner/repo"
        );
    }

    #[test]
    fn test_messages_are_forge_neutral() {
        // The contract crate must not leak a specific backend's branding.
        for err in [
            ForgeError::AuthenticationFailed,
            ForgeError::NoToken,
            ForgeError::RateLimited,
        ] {
            let msg = err.to_string().to_lowercase();
            assert!(!msg.contains("github"), "leaked backend branding: {msg}");
            assert!(!msg.contains("gitlab"), "leaked backend branding: {msg}");
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_from_json_error_maps_to_parse() {
        let json_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let err: ForgeError = json_err.into();
        assert!(matches!(err, ForgeError::Parse(_)));
    }

    #[test]
    fn test_from_io_error_maps_to_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err: ForgeError = io_err.into();
        assert!(matches!(err, ForgeError::Io(_)));
    }
}
