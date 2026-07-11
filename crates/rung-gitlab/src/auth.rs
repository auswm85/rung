//! Authentication handling for the GitLab API.
//!
//! Tokens are stored using `SecretString` from the `secrecy` crate, which
//! automatically zeroizes memory when dropped and prevents accidental logging.
//!
//! Mirrors [`rung_github::Auth`] so both forges resolve credentials the same
//! way: an environment variable first, then the forge's own CLI.

use std::process::Command;

#[cfg(test)]
use secrecy::ExposeSecret;
use secrecy::SecretString;

use rung_forge::{ForgeError as Error, Result};

/// Default GitLab host the `glab` CLI stores credentials under.
///
/// Self-hosted instances supply their own host via [`Auth::auto_for_host`].
const DEFAULT_GLAB_HOST: &str = "gitlab.com";

/// Authentication method for the GitLab API.
#[derive(Debug, Clone)]
pub enum Auth {
    /// Use the token stored by the `glab` CLI for the given host.
    GlabCli {
        /// GitLab host to read credentials for (`gitlab.com` or self-hosted).
        host: String,
    },

    /// Use a token from the named environment variable.
    EnvVar(String),

    /// Use a specific token (zeroized on drop).
    Token(SecretString),
}

impl Auth {
    /// Create auth from the first available method for `gitlab.com`.
    ///
    /// Tries in order: `GITLAB_TOKEN` env var, then the `glab` CLI.
    ///
    /// A blank or whitespace-only `GITLAB_TOKEN` is ignored so it cannot mask a
    /// usable `glab` credential.
    #[must_use]
    pub fn auto() -> Self {
        Self::auto_for_host(DEFAULT_GLAB_HOST)
    }

    /// Create auth from the first available method for a specific GitLab host.
    ///
    /// Behaves like [`Auth::auto`] but, when falling back to the `glab` CLI,
    /// reads the credential stored for `host` — supporting self-hosted GitLab
    /// instances. `GITLAB_TOKEN` still takes precedence and is host-agnostic.
    #[must_use]
    pub fn auto_for_host(host: &str) -> Self {
        match std::env::var("GITLAB_TOKEN") {
            Ok(token) if !token.trim().is_empty() => Self::EnvVar("GITLAB_TOKEN".into()),
            _ => Self::GlabCli {
                host: host.to_owned(),
            },
        }
    }

    /// Resolve the authentication to a token string.
    ///
    /// Returns a `SecretString` that will be zeroized when dropped.
    ///
    /// # Errors
    /// Returns [`ForgeError::NoToken`] if no token can be obtained.
    ///
    /// [`ForgeError::NoToken`]: rung_forge::ForgeError::NoToken
    pub fn resolve(&self) -> Result<SecretString> {
        match self {
            Self::GlabCli { host } => get_glab_token(host),
            Self::EnvVar(var) => {
                let token = std::env::var(var).map_err(|_| Error::NoToken)?;
                let token = token.trim();
                if token.is_empty() {
                    return Err(Error::NoToken);
                }
                Ok(SecretString::from(token))
            }
            Self::Token(t) => Ok(t.clone()),
        }
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self::auto()
    }
}

/// Get the GitLab token stored by the `glab` CLI.
///
/// Unlike `gh`, the `glab` CLI has no `auth token` subcommand; the stored token
/// is read with `glab config get token --host <host>`, which prints only the
/// value (or nothing if it is unset).
///
/// A missing `glab` binary is treated as "no credential source" ([`Error::NoToken`])
/// rather than an I/O error, matching the [`Auth::resolve`] contract.
fn get_glab_token(host: &str) -> Result<SecretString> {
    let output = Command::new("glab")
        .args(["config", "get", "token", "--host", host])
        .output()
        .map_err(|_| Error::NoToken)?;

    if !output.status.success() {
        return Err(Error::NoToken);
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if token.is_empty() {
        return Err(Error::NoToken);
    }

    Ok(SecretString::from(token))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_auto_does_not_panic() {
        // Depends on the environment, so just ensure it resolves to a variant.
        let _auth = Auth::auto();
    }

    #[test]
    fn test_token_auth_resolves_to_value() {
        let auth = Auth::Token(SecretString::from("glpat-test-token"));
        assert_eq!(auth.resolve().unwrap().expose_secret(), "glpat-test-token");
    }

    #[test]
    fn test_env_var_auth_missing_is_error() {
        // A unique var name that is extremely unlikely to exist.
        let auth = Auth::EnvVar("RUNG_TEST_NONEXISTENT_VAR_9a8b7c6d5e4f".into());
        assert!(auth.resolve().is_err());
    }

    #[test]
    fn test_auto_for_host_uses_host_for_glab() {
        // With no GITLAB_TOKEN set, auto_for_host falls back to the glab CLI
        // for the requested host. Guarded so an ambient token does not flake.
        if std::env::var("GITLAB_TOKEN").is_ok_and(|t| !t.trim().is_empty()) {
            return;
        }
        match Auth::auto_for_host("gitlab.example.com") {
            Auth::GlabCli { host } => assert_eq!(host, "gitlab.example.com"),
            other => panic!("expected GlabCli, got {other:?}"),
        }
    }

    #[test]
    fn test_auth_default_is_auto() {
        // Default must never be a bare Token; it picks env var or CLI.
        match Auth::default() {
            Auth::GlabCli { .. } | Auth::EnvVar(_) => {}
            Auth::Token(_) => panic!("Default should not return Token"),
        }
    }
}
