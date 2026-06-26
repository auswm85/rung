//! GitLab API client.
//!
//! This is the credential-bearing foundation for GitLab support: it resolves an
//! [`Auth`] to a token and builds an authenticated HTTP client. It carries a
//! skeleton [`ForgeApi`](rung_forge::ForgeApi) implementation; the merge
//! request, pipeline, and comment methods are filled in separately (see issue
//! #170).

use std::collections::HashMap;

use reqwest::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use rung_forge::{
    CheckRun, CreateComment, CreatePullRequest, ForgeApi, ForgeError as Error, IssueComment,
    MergePullRequest, MergeResult, PullRequest, RepoId, Result, UpdateComment, UpdatePullRequest,
};

use crate::auth::Auth;

/// The authenticated GitLab user, returned by `GET /user`.
///
/// Used to verify that resolved credentials are valid.
#[derive(Debug, Clone, Deserialize)]
pub struct GitLabUser {
    /// Numeric user ID.
    pub id: u64,
    /// Account username (the `@handle`).
    pub username: String,
}

/// GitLab API client.
///
/// Holds an authenticated [`reqwest::Client`] and the API base URL. The token
/// is stored as a [`SecretString`] so it is zeroized on drop and never logged.
pub struct GitLabClient {
    client: Client,
    base_url: String,
    /// Token stored as `SecretString` for automatic zeroization on drop.
    token: SecretString,
}

impl GitLabClient {
    /// Default GitLab API base URL (gitlab.com, REST v4).
    ///
    /// Self-hosted instances supply their own base URL (see issue #172).
    pub const DEFAULT_API_URL: &'static str = "https://gitlab.com/api/v4";

    /// Create a new GitLab client targeting gitlab.com.
    ///
    /// # Errors
    /// Returns an error if authentication cannot be resolved or the HTTP client
    /// cannot be built.
    pub fn new(auth: &Auth) -> Result<Self> {
        Self::with_base_url(auth, Self::DEFAULT_API_URL)
    }

    /// Create a new GitLab client with a custom API base URL.
    ///
    /// Used for self-hosted instances and for tests pointing at a mock server.
    ///
    /// # Errors
    /// Returns an error if authentication cannot be resolved or the HTTP client
    /// cannot be built.
    pub fn with_base_url(auth: &Auth, base_url: impl Into<String>) -> Result<Self> {
        let token = auth.resolve()?;

        // Normalize so a custom base URL with a trailing slash (common for
        // self-hosted instances) does not produce `.../api/v4//user`.
        let base_url = base_url.into().trim_end_matches('/').to_owned();

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("rung-cli"));

        let client = Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            client,
            base_url,
            token,
        })
    }

    /// Fetch the authenticated user, verifying the resolved credentials.
    ///
    /// # Errors
    /// Returns [`Error::AuthenticationFailed`] if the token is rejected, or an
    /// [`Error::ApiError`] for other non-success responses.
    pub async fn current_user(&self) -> Result<GitLabUser> {
        self.get("/user").await
    }

    /// Make an authenticated GET request and deserialize the JSON body.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let response = self
            .client
            .get(&url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .send()
            .await?;

        let status = response.status();
        if status.is_success() {
            return Ok(response.json().await?);
        }

        match status.as_u16() {
            401 => Err(Error::AuthenticationFailed),
            429 => Err(Error::RateLimited),
            code => {
                let message = response.text().await.unwrap_or_default();
                Err(Error::ApiError {
                    status: code,
                    message,
                })
            }
        }
    }
}

impl std::fmt::Debug for GitLabClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitLabClient")
            .field("base_url", &self.base_url)
            .field("token", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// Skeleton [`ForgeApi`] implementation.
///
/// Every method is currently `unimplemented!()`; the real GitLab merge-request,
/// pipeline, and comment logic is added in issue #170. The impl exists now so
/// the CLI can dispatch to GitLab through the forge-neutral trait (issue #171)
/// once those methods are filled in.
#[allow(clippy::unused_async)]
impl ForgeApi for GitLabClient {
    async fn get_pr(&self, _repo: &RepoId, _number: u64) -> Result<PullRequest> {
        unimplemented!("GitLab get_pr: see #170")
    }

    async fn get_prs_batch(
        &self,
        _repo: &RepoId,
        _numbers: &[u64],
    ) -> Result<HashMap<u64, PullRequest>> {
        unimplemented!("GitLab get_prs_batch: see #170")
    }

    async fn find_pr_for_branch(
        &self,
        _repo: &RepoId,
        _branch: &str,
    ) -> Result<Option<PullRequest>> {
        unimplemented!("GitLab find_pr_for_branch: see #170")
    }

    async fn create_pr(&self, _repo: &RepoId, _pr: CreatePullRequest) -> Result<PullRequest> {
        unimplemented!("GitLab create_pr: see #170")
    }

    async fn update_pr(
        &self,
        _repo: &RepoId,
        _number: u64,
        _update: UpdatePullRequest,
    ) -> Result<PullRequest> {
        unimplemented!("GitLab update_pr: see #170")
    }

    async fn get_check_runs(&self, _repo: &RepoId, _commit_sha: &str) -> Result<Vec<CheckRun>> {
        unimplemented!("GitLab get_check_runs: see #170")
    }

    async fn merge_pr(
        &self,
        _repo: &RepoId,
        _number: u64,
        _merge: MergePullRequest,
    ) -> Result<MergeResult> {
        unimplemented!("GitLab merge_pr: see #170")
    }

    async fn delete_ref(&self, _repo: &RepoId, _ref_name: &str) -> Result<()> {
        unimplemented!("GitLab delete_ref: see #170")
    }

    async fn get_default_branch(&self, _repo: &RepoId) -> Result<String> {
        unimplemented!("GitLab get_default_branch: see #170")
    }

    async fn list_pr_comments(&self, _repo: &RepoId, _pr_number: u64) -> Result<Vec<IssueComment>> {
        unimplemented!("GitLab list_pr_comments: see #170")
    }

    async fn create_pr_comment(
        &self,
        _repo: &RepoId,
        _pr_number: u64,
        _comment: CreateComment,
    ) -> Result<IssueComment> {
        unimplemented!("GitLab create_pr_comment: see #170")
    }

    async fn update_pr_comment(
        &self,
        _repo: &RepoId,
        _comment_id: u64,
        _comment: UpdateComment,
    ) -> Result<IssueComment> {
        unimplemented!("GitLab update_pr_comment: see #170")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use secrecy::SecretString;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn test_client(base_url: &str) -> GitLabClient {
        let auth = Auth::Token(SecretString::from("glpat-test-token"));
        GitLabClient::with_base_url(&auth, base_url).unwrap()
    }

    #[test]
    fn test_debug_redacts_token() {
        let client = test_client("https://gitlab.example.com/api/v4");
        let debug = format!("{client:?}");
        assert!(debug.contains("base_url"));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("glpat-test-token"));
    }

    #[tokio::test]
    async fn test_current_user_sends_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("authorization", "Bearer glpat-test-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 42, "username": "octocat"})),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let user = client.current_user().await.unwrap();

        assert_eq!(user.id, 42);
        assert_eq!(user.username, "octocat");
    }

    #[tokio::test]
    async fn test_trailing_slash_base_url_does_not_double_slash() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 1, "username": "u"})),
            )
            .mount(&server)
            .await;

        // A base URL with a trailing slash must still hit `/user`, not `//user`.
        let client = test_client(&format!("{}/", server.uri()));
        assert!(client.current_user().await.is_ok());
    }

    #[tokio::test]
    async fn test_current_user_maps_401_to_auth_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client.current_user().await.unwrap_err();

        assert!(matches!(err, Error::AuthenticationFailed));
    }
}
