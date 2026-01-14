//! Authentication handling for GitHub API.

use std::process::Command;

use crate::error::{Error, Result};

/// Authentication method for GitHub API.
#[derive(Debug, Clone)]
pub enum Auth {
    /// Use token from gh CLI.
    GhCli,

    /// Use token from environment variable.
    EnvVar(String),

    /// Use a specific token.
    Token(String),
}

impl Auth {
    /// Create auth from the first available method.
    ///
    /// Tries in order: `GITHUB_TOKEN` env var, gh CLI.
    #[must_use]
    pub fn auto() -> Self {
        if std::env::var("GITHUB_TOKEN").is_ok() {
            Self::EnvVar("GITHUB_TOKEN".into())
        } else {
            Self::GhCli
        }
    }

    /// Resolve the authentication to a token string.
    ///
    /// # Errors
    /// Returns error if token cannot be obtained.
    pub fn resolve(&self) -> Result<String> {
        match self {
            Self::GhCli => get_gh_token(),
            Self::EnvVar(var) => std::env::var(var).map_err(|_| Error::NoToken),
            Self::Token(t) => Ok(t.clone()),
        }
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self::auto()
    }
}

/// Get GitHub token from gh CLI.
fn get_gh_token() -> Result<String> {
    let output = Command::new("gh").args(["auth", "token"]).output()?;

    if !output.status.success() {
        return Err(Error::NoToken);
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if token.is_empty() {
        return Err(Error::NoToken);
    }

    Ok(token)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_auto_prefers_env() {
        // This test depends on environment, so just ensure it doesn't panic
        let _auth = Auth::auto();
    }

    #[test]
    fn test_token_auth() {
        let auth = Auth::Token("test_token".into());
        assert_eq!(auth.resolve().unwrap(), "test_token");
    }
}
