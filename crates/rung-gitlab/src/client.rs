//! GitLab API client.
//!
//! Implements the forge-neutral [`ForgeApi`](rung_forge::ForgeApi) contract
//! against GitLab's REST v4 API. Forge concepts map onto GitLab's vocabulary:
//!
//! | Forge concept        | GitLab                                            |
//! | -------------------- | ------------------------------------------------- |
//! | pull request         | merge request (addressed by project-scoped `iid`) |
//! | `owner/repo`         | project path or ID (URL-encoded, nested-namespace safe) |
//! | check run            | commit status                                     |
//! | PR comment           | merge request note                                |
//!
//! The token is stored as a [`SecretString`] so it is zeroized on drop and
//! never logged.

use std::collections::HashMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Client, RequestBuilder, Response};
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

use rung_forge::{
    CheckRun, CheckStatus, CreateComment, CreatePullRequest, ForgeApi, ForgeError as Error,
    IssueComment, MergeMethod, MergePullRequest, MergeResult, PullRequest, PullRequestState,
    RepoId, Result, UpdateComment, UpdatePullRequest,
};

use crate::auth::Auth;

/// Characters percent-encoded in a GitLab path parameter.
///
/// GitLab addresses a project by its URL-encoded path, so a nested namespace
/// (`group/subgroup/project`) and a slashful branch name (`feat/x`) must each
/// collapse into a single path segment. Everything outside the RFC 3986
/// unreserved set (`A-Z a-z 0-9 - . _ ~`) is encoded, turning `/` into `%2F`.
const PATH_PARAM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// URL-encode a GitLab path parameter (project path or branch name).
fn encode(value: &str) -> String {
    utf8_percent_encode(value, PATH_PARAM).to_string()
}

/// URL-encode a [`RepoId`] into a GitLab project path parameter.
fn project(repo: &RepoId) -> String {
    encode(repo.path())
}

// === Internal API response/request types ===

/// A merge request as returned by the GitLab API.
#[derive(serde::Deserialize)]
struct ApiMergeRequest {
    iid: u64,
    title: String,
    description: Option<String>,
    /// `opened`, `closed`, `merged`, or `locked`.
    state: String,
    #[serde(default)]
    draft: bool,
    source_branch: String,
    target_branch: String,
    web_url: String,
    /// Deprecated coarse status (`can_be_merged` / `cannot_be_merged` / …).
    #[serde(default)]
    merge_status: Option<String>,
    /// Fine-grained status (`mergeable`, `conflict`, `ci_still_running`, …).
    #[serde(default)]
    detailed_merge_status: Option<String>,
}

impl ApiMergeRequest {
    fn into_pull_request(self) -> PullRequest {
        let state = match self.state.as_str() {
            "opened" => PullRequestState::Open,
            "merged" => PullRequestState::Merged,
            // `closed` and `locked` both mean "not open, not merged".
            _ => PullRequestState::Closed,
        };

        // GitLab reports merge readiness via a status string rather than a
        // boolean; translate the coarse form into `Option<bool>`.
        let mergeable = match self.merge_status.as_deref() {
            Some("can_be_merged") => Some(true),
            Some("cannot_be_merged") => Some(false),
            _ => None,
        };

        PullRequest {
            number: self.iid,
            title: self.title,
            body: self.description,
            state,
            draft: self.draft,
            head_branch: self.source_branch,
            base_branch: self.target_branch,
            html_url: self.web_url,
            mergeable,
            mergeable_state: self.detailed_merge_status.or(self.merge_status),
        }
    }
}

/// Body for creating a merge request (GitLab field names).
///
/// GitLab's create-MR endpoint has no `draft` attribute; a draft is signalled
/// by a `Draft:` title prefix, so no draft field appears here.
#[derive(Serialize)]
struct ApiCreateMergeRequest {
    source_branch: String,
    target_branch: String,
    title: String,
    description: String,
}

/// Body for updating a merge request (all fields optional).
#[derive(Serialize)]
struct ApiUpdateMergeRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_branch: Option<String>,
}

/// Body for accepting (merging) a merge request.
#[derive(Serialize)]
struct ApiAcceptMergeRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_commit_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    squash_commit_message: Option<String>,
    squash: bool,
}

/// Merge-request payload returned by the accept endpoint, carrying the SHAs.
#[derive(serde::Deserialize)]
struct ApiMergeResult {
    state: String,
    #[serde(default)]
    merge_commit_sha: Option<String>,
    #[serde(default)]
    squash_commit_sha: Option<String>,
    #[serde(default)]
    sha: Option<String>,
}

/// A commit status (GitLab's analog of a GitHub check run).
#[derive(serde::Deserialize)]
struct ApiCommitStatus {
    name: String,
    /// `created`, `pending`, `running`, `success`, `failed`, `canceled`,
    /// `skipped`, or `manual`.
    status: String,
    target_url: Option<String>,
}

impl ApiCommitStatus {
    fn into_check_run(self) -> CheckRun {
        let status = match self.status.as_str() {
            "created" | "pending" | "manual" => CheckStatus::Queued,
            "running" => CheckStatus::InProgress,
            "success" => CheckStatus::Success,
            "skipped" => CheckStatus::Skipped,
            "canceled" => CheckStatus::Cancelled,
            // `failed` and anything unrecognized are treated as failure.
            _ => CheckStatus::Failure,
        };

        CheckRun {
            name: self.name,
            status,
            details_url: self.target_url,
        }
    }
}

/// Project metadata (only the default branch is needed).
#[derive(serde::Deserialize)]
struct ApiProject {
    default_branch: Option<String>,
}

/// The authenticated GitLab user, returned by `GET /user`.
///
/// Used to verify that resolved credentials are valid.
#[derive(Debug, Clone, serde::Deserialize)]
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
    /// Self-hosted instances supply their own base URL via
    /// [`GitLabClient::with_base_url`], configured from `gitlab.api_url`.
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

    // === HTTP plumbing ===

    /// Build a full request URL from an API path.
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Attach the bearer token to a request builder.
    ///
    /// GitLab accepts an OAuth-style `Authorization: Bearer` header for both
    /// personal access tokens and OAuth tokens.
    fn authed(&self, req: RequestBuilder) -> RequestBuilder {
        req.header(
            AUTHORIZATION,
            format!("Bearer {}", self.token.expose_secret()),
        )
    }

    /// Make an authenticated GET request and deserialize the JSON body.
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.authed(self.client.get(self.url(path))).send().await?;
        self.handle_response(response).await
    }

    /// Fetch every page of a paginated GitLab list endpoint.
    ///
    /// GitLab caps `per_page` at 100 and returns longer collections across
    /// offset pages, so a single request can silently truncate results. This
    /// requests `per_page=100` and follows successive pages until one comes
    /// back with fewer than 100 items (the final page).
    async fn get_paginated<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Vec<T>> {
        const PER_PAGE: usize = 100;
        let url = self.url(path);
        let mut all = Vec::new();
        let mut page = 1u32;
        loop {
            let response = self
                .authed(self.client.get(&url))
                .query(query)
                .query(&[
                    ("per_page", PER_PAGE.to_string()),
                    ("page", page.to_string()),
                ])
                .send()
                .await?;
            let batch: Vec<T> = self.handle_response(response).await?;
            let batch_len = batch.len();
            all.extend(batch);
            if batch_len < PER_PAGE {
                break;
            }
            page += 1;
        }
        Ok(all)
    }

    /// Make an authenticated POST request with a JSON body.
    async fn post<T: DeserializeOwned, B: Serialize + Sync>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = self
            .authed(self.client.post(self.url(path)))
            .json(body)
            .send()
            .await?;
        self.handle_response(response).await
    }

    /// Make an authenticated PUT request with a JSON body.
    async fn put<T: DeserializeOwned, B: Serialize + Sync>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = self
            .authed(self.client.put(self.url(path)))
            .json(body)
            .send()
            .await?;
        self.handle_response(response).await
    }

    /// Make an authenticated DELETE request, discarding any body.
    async fn delete(&self, path: &str) -> Result<()> {
        let response = self
            .authed(self.client.delete(self.url(path)))
            .send()
            .await?;

        if response.status().is_success() {
            return Ok(());
        }
        Err(self.error_for(response).await)
    }

    /// Deserialize a successful response, or map a failure to a [`ForgeError`].
    async fn handle_response<T: DeserializeOwned>(&self, response: Response) -> Result<T> {
        if response.status().is_success() {
            return Ok(response.json().await?);
        }
        Err(self.error_for(response).await)
    }

    /// Map a non-success response to a neutral [`ForgeError`].
    async fn error_for(&self, response: Response) -> Error {
        match response.status().as_u16() {
            401 => Error::AuthenticationFailed,
            429 => Error::RateLimited,
            code => {
                let message = response.text().await.unwrap_or_default();
                Error::ApiError {
                    status: code,
                    message,
                }
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

impl ForgeApi for GitLabClient {
    async fn get_pr(&self, repo: &RepoId, number: u64) -> Result<PullRequest> {
        let mr: ApiMergeRequest = self
            .get(&format!(
                "/projects/{}/merge_requests/{number}",
                project(repo)
            ))
            .await?;
        Ok(mr.into_pull_request())
    }

    async fn get_prs_batch(
        &self,
        repo: &RepoId,
        numbers: &[u64],
    ) -> Result<HashMap<u64, PullRequest>> {
        if numbers.is_empty() {
            return Ok(HashMap::new());
        }

        // GitLab filters a listing by internal IDs via repeated `iids[]` params.
        // The listing is paginated, so page through it to fetch every requested
        // MR even when more than one page (100) is asked for.
        let query: Vec<(&str, String)> =
            numbers.iter().map(|n| ("iids[]", n.to_string())).collect();
        let mrs: Vec<ApiMergeRequest> = self
            .get_paginated(
                &format!("/projects/{}/merge_requests", project(repo)),
                &query,
            )
            .await?;
        Ok(mrs
            .into_iter()
            .map(|mr| (mr.iid, mr.into_pull_request()))
            .collect())
    }

    async fn find_pr_for_branch(&self, repo: &RepoId, branch: &str) -> Result<Option<PullRequest>> {
        let url = self.url(&format!("/projects/{}/merge_requests", project(repo)));
        let response = self
            .authed(self.client.get(url))
            .query(&[("source_branch", branch), ("state", "opened")])
            .send()
            .await?;

        let mrs: Vec<ApiMergeRequest> = self.handle_response(response).await?;
        Ok(mrs
            .into_iter()
            .next()
            .map(ApiMergeRequest::into_pull_request))
    }

    async fn create_pr(&self, repo: &RepoId, pr: CreatePullRequest) -> Result<PullRequest> {
        // GitLab's create-MR endpoint ignores a `draft` field; the draft state is
        // conveyed by a `Draft:` title prefix instead.
        let title = if pr.draft {
            format!("Draft: {}", pr.title)
        } else {
            pr.title
        };
        let body = ApiCreateMergeRequest {
            source_branch: pr.head,
            target_branch: pr.base,
            title,
            description: pr.body,
        };
        let mr: ApiMergeRequest = self
            .post(
                &format!("/projects/{}/merge_requests", project(repo)),
                &body,
            )
            .await?;
        Ok(mr.into_pull_request())
    }

    async fn update_pr(
        &self,
        repo: &RepoId,
        number: u64,
        update: UpdatePullRequest,
    ) -> Result<PullRequest> {
        let body = ApiUpdateMergeRequest {
            title: update.title,
            description: update.body,
            target_branch: update.base,
        };
        let mr: ApiMergeRequest = self
            .put(
                &format!("/projects/{}/merge_requests/{number}", project(repo)),
                &body,
            )
            .await?;
        Ok(mr.into_pull_request())
    }

    async fn get_check_runs(&self, repo: &RepoId, commit_sha: &str) -> Result<Vec<CheckRun>> {
        // Commit statuses are paginated; page through them so a commit with many
        // CI jobs does not lose statuses past the first page.
        let statuses: Vec<ApiCommitStatus> = self
            .get_paginated(
                &format!(
                    "/projects/{}/repository/commits/{commit_sha}/statuses",
                    project(repo)
                ),
                &[],
            )
            .await?;
        Ok(statuses
            .into_iter()
            .map(ApiCommitStatus::into_check_run)
            .collect())
    }

    async fn merge_pr(
        &self,
        repo: &RepoId,
        number: u64,
        merge: MergePullRequest,
    ) -> Result<MergeResult> {
        // GitLab's accept endpoint exposes only a `squash` toggle; the merge vs.
        // rebase strategy is a project setting, so `Merge` and `Rebase` both map
        // to a non-squash accept.
        let squash = matches!(merge.merge_method, MergeMethod::Squash);
        let message = merge.commit_message.or(merge.commit_title);
        let body = if squash {
            ApiAcceptMergeRequest {
                merge_commit_message: None,
                squash_commit_message: message,
                squash: true,
            }
        } else {
            ApiAcceptMergeRequest {
                merge_commit_message: message,
                squash_commit_message: None,
                squash: false,
            }
        };

        let merged: ApiMergeResult = self
            .put(
                &format!("/projects/{}/merge_requests/{number}/merge", project(repo)),
                &body,
            )
            .await?;

        let sha = merged
            .merge_commit_sha
            .or(merged.squash_commit_sha)
            .or(merged.sha)
            .unwrap_or_default();
        Ok(MergeResult {
            sha,
            merged: merged.state == "merged",
            message: merged.state,
        })
    }

    async fn delete_ref(&self, repo: &RepoId, ref_name: &str) -> Result<()> {
        // The forge-neutral ref name is a branch name; tolerate a `refs/heads/`
        // prefix, then encode it as GitLab addresses branches by name.
        let branch = ref_name.strip_prefix("refs/heads/").unwrap_or(ref_name);
        self.delete(&format!(
            "/projects/{}/repository/branches/{}",
            project(repo),
            encode(branch)
        ))
        .await
    }

    async fn get_default_branch(&self, repo: &RepoId) -> Result<String> {
        let info: ApiProject = self.get(&format!("/projects/{}", project(repo))).await?;
        // A missing `default_branch` (e.g. an empty repo) is not a valid branch
        // name; surface it as an error rather than an empty string.
        info.default_branch
            .filter(|b| !b.is_empty())
            .ok_or_else(|| Error::RepoNotFound(repo.path().to_string()))
    }

    async fn list_pr_comments(&self, repo: &RepoId, pr_number: u64) -> Result<Vec<IssueComment>> {
        // GitLab notes expose `id` and `body`, matching `IssueComment`; extra
        // fields are ignored by serde.
        self.get(&format!(
            "/projects/{}/merge_requests/{pr_number}/notes",
            project(repo)
        ))
        .await
    }

    async fn create_pr_comment(
        &self,
        repo: &RepoId,
        pr_number: u64,
        comment: CreateComment,
    ) -> Result<IssueComment> {
        self.post(
            &format!(
                "/projects/{}/merge_requests/{pr_number}/notes",
                project(repo)
            ),
            &comment,
        )
        .await
    }

    async fn update_pr_comment(
        &self,
        repo: &RepoId,
        pr_number: u64,
        comment_id: u64,
        comment: UpdateComment,
    ) -> Result<IssueComment> {
        // GitLab scopes a note update to its merge request, so `pr_number` is
        // required here (unlike GitHub, which addresses comments by id alone).
        self.put(
            &format!(
                "/projects/{}/merge_requests/{pr_number}/notes/{comment_id}",
                project(repo)
            ),
            &comment,
        )
        .await
    }

    /// GitLab references merge requests with `!` (a `#` would link to an issue).
    fn pr_reference_prefix(&self) -> &'static str {
        "!"
    }

    /// GitLab calls them merge requests.
    fn pr_noun(&self) -> &'static str {
        "MR"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rung_forge::MergeMethod;
    use secrecy::SecretString;
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn test_client(base_url: &str) -> GitLabClient {
        let auth = Auth::Token(SecretString::from("glpat-test-token"));
        GitLabClient::with_base_url(&auth, base_url).unwrap()
    }

    #[test]
    fn test_pr_reference_prefix_is_bang() {
        // GitLab references merge requests with `!`; `#` would link to an issue.
        assert_eq!(
            test_client("https://gitlab.com/api/v4").pr_reference_prefix(),
            "!"
        );
    }

    /// Standard merge-request response JSON for testing.
    fn mr_response_json(iid: u64, state: &str) -> serde_json::Value {
        serde_json::json!({
            "iid": iid,
            "title": format!("MR !{iid}"),
            "description": "Test body",
            "state": state,
            "draft": false,
            "source_branch": "feature-branch",
            "target_branch": "main",
            "web_url": format!("https://gitlab.com/owner/repo/-/merge_requests/{iid}"),
            "merge_status": "can_be_merged",
            "detailed_merge_status": "mergeable"
        })
    }

    // === Encoding ===

    #[test]
    fn test_encode_flat_path() {
        assert_eq!(encode("owner/repo"), "owner%2Frepo");
    }

    #[test]
    fn test_encode_nested_namespace() {
        assert_eq!(
            encode("group/subgroup/project"),
            "group%2Fsubgroup%2Fproject"
        );
    }

    #[test]
    fn test_encode_leaves_unreserved_chars() {
        // RFC 3986 unreserved characters must survive verbatim.
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    // === Debug redaction ===

    #[test]
    fn test_debug_redacts_token() {
        let client = test_client("https://gitlab.example.com/api/v4");
        let debug = format!("{client:?}");
        assert!(debug.contains("base_url"));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("glpat-test-token"));
    }

    // === current_user (credential probe) ===

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

        let client = test_client(&format!("{}/", server.uri()));
        assert!(client.current_user().await.is_ok());
    }

    // === get_pr ===

    #[tokio::test]
    async fn test_get_pr_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests/123"))
            .and(header("authorization", "Bearer glpat-test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_response_json(123, "opened")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .get_pr(&RepoId::new("owner/repo"), 123)
            .await
            .unwrap();

        assert_eq!(pr.number, 123);
        assert_eq!(pr.title, "MR !123");
        assert_eq!(pr.state, PullRequestState::Open);
        assert_eq!(pr.head_branch, "feature-branch");
        assert_eq!(pr.base_branch, "main");
        assert_eq!(pr.mergeable, Some(true));
        assert_eq!(pr.mergeable_state.as_deref(), Some("mergeable"));
    }

    #[tokio::test]
    async fn test_get_pr_merged_state() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests/456"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_response_json(456, "merged")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .get_pr(&RepoId::new("owner/repo"), 456)
            .await
            .unwrap();
        assert_eq!(pr.state, PullRequestState::Merged);
    }

    #[tokio::test]
    async fn test_get_pr_locked_maps_to_closed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests/789"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_response_json(789, "locked")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .get_pr(&RepoId::new("owner/repo"), 789)
            .await
            .unwrap();
        assert_eq!(pr.state, PullRequestState::Closed);
    }

    #[tokio::test]
    async fn test_get_pr_nested_namespace_is_encoded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/projects/group%2Fsubgroup%2Fproject/merge_requests/1",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_response_json(1, "opened")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .get_pr(&RepoId::new("group/subgroup/project"), 1)
            .await
            .unwrap();
        assert_eq!(pr.number, 1);
    }

    #[tokio::test]
    async fn test_get_pr_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests/999"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "404 Not found"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client
            .get_pr(&RepoId::new("owner/repo"), 999)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ApiError { status: 404, .. }));
    }

    // === Error mapping ===

    #[tokio::test]
    async fn test_maps_401_to_auth_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests/1"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client
            .get_pr(&RepoId::new("owner/repo"), 1)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::AuthenticationFailed));
    }

    #[tokio::test]
    async fn test_maps_429_to_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests/1"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client
            .get_pr(&RepoId::new("owner/repo"), 1)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::RateLimited));
    }

    // === get_prs_batch ===

    #[tokio::test]
    async fn test_get_prs_batch_empty_makes_no_request() {
        let server = MockServer::start().await;
        let client = test_client(&server.uri());
        let result = client
            .get_prs_batch(&RepoId::new("owner/repo"), &[])
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_get_prs_batch_keys_by_iid() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests"))
            .and(query_param("iids[]", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                mr_response_json(1, "opened"),
                mr_response_json(2, "merged"),
            ])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client
            .get_prs_batch(&RepoId::new("owner/repo"), &[1, 2])
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(&1).unwrap().state, PullRequestState::Open);
        assert_eq!(result.get(&2).unwrap().state, PullRequestState::Merged);
    }

    #[tokio::test]
    async fn test_get_prs_batch_paginates_beyond_one_page() {
        let server = MockServer::start().await;
        // A full first page (100 items) must trigger a follow-up request; the
        // final page returns fewer than 100 and stops the loop.
        let page1: Vec<serde_json::Value> =
            (1..=100).map(|i| mr_response_json(i, "opened")).collect();
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(page1)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([mr_response_json(101, "merged")])),
            )
            .mount(&server)
            .await;

        let numbers: Vec<u64> = (1..=101).collect();
        let client = test_client(&server.uri());
        let result = client
            .get_prs_batch(&RepoId::new("owner/repo"), &numbers)
            .await
            .unwrap();

        assert_eq!(result.len(), 101);
        assert_eq!(result.get(&101).unwrap().state, PullRequestState::Merged);
    }

    // === find_pr_for_branch ===

    #[tokio::test]
    async fn test_find_pr_for_branch_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests"))
            .and(query_param("source_branch", "feature"))
            .and(query_param("state", "opened"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([mr_response_json(42, "opened")])),
            )
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .find_pr_for_branch(&RepoId::new("owner/repo"), "feature")
            .await
            .unwrap();
        assert_eq!(pr.unwrap().number, 42);
    }

    #[tokio::test]
    async fn test_find_pr_for_branch_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .find_pr_for_branch(&RepoId::new("owner/repo"), "missing")
            .await
            .unwrap();
        assert!(pr.is_none());
    }

    // === create_pr ===

    #[tokio::test]
    async fn test_create_pr_maps_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/projects/owner%2Frepo/merge_requests"))
            // GitHub-vocabulary head/base/body must reach GitLab's field names.
            .and(body_partial_json(serde_json::json!({
                "source_branch": "feature",
                "target_branch": "main",
                "title": "New Feature",
                "description": "Description"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(mr_response_json(100, "opened")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .create_pr(
                &RepoId::new("owner/repo"),
                CreatePullRequest {
                    title: "New Feature".into(),
                    body: "Description".into(),
                    head: "feature".into(),
                    base: "main".into(),
                    draft: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(pr.number, 100);
        assert_eq!(pr.state, PullRequestState::Open);
    }

    #[tokio::test]
    async fn test_create_pr_draft_prefixes_title() {
        let server = MockServer::start().await;
        // GitLab has no `draft` field; a draft MR is signalled by a `Draft:`
        // title prefix, and the raw `draft` key must not be sent.
        Mock::given(method("POST"))
            .and(path("/projects/owner%2Frepo/merge_requests"))
            .and(body_partial_json(serde_json::json!({
                "title": "Draft: New Feature"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(mr_response_json(101, "opened")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .create_pr(
                &RepoId::new("owner/repo"),
                CreatePullRequest {
                    title: "New Feature".into(),
                    body: "Description".into(),
                    head: "feature".into(),
                    base: "main".into(),
                    draft: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(pr.number, 101);
    }

    // === update_pr ===

    #[tokio::test]
    async fn test_update_pr_maps_base_to_target_branch() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/projects/owner%2Frepo/merge_requests/123"))
            .and(body_partial_json(serde_json::json!({
                "title": "Updated",
                "target_branch": "develop"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_response_json(123, "opened")))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let pr = client
            .update_pr(
                &RepoId::new("owner/repo"),
                123,
                UpdatePullRequest {
                    title: Some("Updated".into()),
                    body: None,
                    base: Some("develop".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(pr.number, 123);
    }

    // === get_check_runs ===

    #[tokio::test]
    async fn test_get_check_runs_maps_statuses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/projects/owner%2Frepo/repository/commits/abc123/statuses",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "name": "build", "status": "success", "target_url": "https://gitlab.com/ci/1" },
                { "name": "test", "status": "running", "target_url": null },
                { "name": "deploy", "status": "pending", "target_url": null },
                { "name": "lint", "status": "failed", "target_url": null },
                { "name": "docs", "status": "skipped", "target_url": null },
                { "name": "e2e", "status": "canceled", "target_url": null },
                { "name": "gate", "status": "manual", "target_url": null }
            ])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let checks = client
            .get_check_runs(&RepoId::new("owner/repo"), "abc123")
            .await
            .unwrap();

        assert_eq!(checks[0].status, CheckStatus::Success);
        assert_eq!(
            checks[0].details_url.as_deref(),
            Some("https://gitlab.com/ci/1")
        );
        assert_eq!(checks[1].status, CheckStatus::InProgress);
        assert_eq!(checks[2].status, CheckStatus::Queued);
        assert_eq!(checks[3].status, CheckStatus::Failure);
        assert_eq!(checks[4].status, CheckStatus::Skipped);
        assert_eq!(checks[5].status, CheckStatus::Cancelled);
        assert_eq!(checks[6].status, CheckStatus::Queued);
    }

    // === merge_pr ===

    #[tokio::test]
    async fn test_merge_pr_squash_sets_squash_true() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/projects/owner%2Frepo/merge_requests/123/merge"))
            .and(body_partial_json(serde_json::json!({
                "squash": true,
                "squash_commit_message": "Squash it"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "state": "merged",
                "merge_commit_sha": null,
                "squash_commit_sha": "sq123",
                "sha": "head123"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client
            .merge_pr(
                &RepoId::new("owner/repo"),
                123,
                MergePullRequest {
                    commit_title: None,
                    commit_message: Some("Squash it".into()),
                    merge_method: MergeMethod::Squash,
                },
            )
            .await
            .unwrap();

        assert!(result.merged);
        assert_eq!(result.sha, "sq123");
    }

    #[tokio::test]
    async fn test_merge_pr_merge_method_sets_squash_false() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/projects/owner%2Frepo/merge_requests/7/merge"))
            .and(body_partial_json(serde_json::json!({
                "squash": false,
                "merge_commit_message": "Merge it"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "state": "merged",
                "merge_commit_sha": "mc123"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let result = client
            .merge_pr(
                &RepoId::new("owner/repo"),
                7,
                MergePullRequest {
                    commit_title: None,
                    commit_message: Some("Merge it".into()),
                    merge_method: MergeMethod::Merge,
                },
            )
            .await
            .unwrap();

        assert!(result.merged);
        assert_eq!(result.sha, "mc123");
    }

    // === delete_ref ===

    #[tokio::test]
    async fn test_delete_ref_success() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(
                "/projects/owner%2Frepo/repository/branches/feature-branch",
            ))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        assert!(
            client
                .delete_ref(&RepoId::new("owner/repo"), "feature-branch")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_delete_ref_strips_refs_heads_and_encodes() {
        let server = MockServer::start().await;
        // `refs/heads/feat/x` -> branch `feat/x` -> encoded `feat%2Fx`.
        Mock::given(method("DELETE"))
            .and(path("/projects/owner%2Frepo/repository/branches/feat%2Fx"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        assert!(
            client
                .delete_ref(&RepoId::new("owner/repo"), "refs/heads/feat/x")
                .await
                .is_ok()
        );
    }

    // === get_default_branch ===

    #[tokio::test]
    async fn test_get_default_branch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "default_branch": "main"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let branch = client
            .get_default_branch(&RepoId::new("owner/repo"))
            .await
            .unwrap();
        assert_eq!(branch, "main");
    }

    #[tokio::test]
    async fn test_get_default_branch_missing_is_error() {
        let server = MockServer::start().await;
        // An empty repo reports a null default branch; that must not surface as
        // an empty string that downstream treats as a real branch.
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "default_branch": null
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let err = client
            .get_default_branch(&RepoId::new("owner/repo"))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::RepoNotFound(_)));
    }

    // === Comments (notes) ===

    #[tokio::test]
    async fn test_list_pr_comments() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/projects/owner%2Frepo/merge_requests/123/notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 1, "body": "First" },
                { "id": 2, "body": "Second" }
            ])))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let comments = client
            .list_pr_comments(&RepoId::new("owner/repo"), 123)
            .await
            .unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].id, 1);
        assert_eq!(comments[0].body.as_deref(), Some("First"));
    }

    #[tokio::test]
    async fn test_create_pr_comment() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/projects/owner%2Frepo/merge_requests/123/notes"))
            .and(body_partial_json(serde_json::json!({ "body": "New note" })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": 42, "body": "New note"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let comment = client
            .create_pr_comment(
                &RepoId::new("owner/repo"),
                123,
                CreateComment {
                    body: "New note".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(comment.id, 42);
    }

    #[tokio::test]
    async fn test_update_pr_comment_scopes_to_merge_request() {
        let server = MockServer::start().await;
        // The note update must target the MR iid, not just the note id.
        Mock::given(method("PUT"))
            .and(path("/projects/owner%2Frepo/merge_requests/123/notes/42"))
            .and(body_partial_json(serde_json::json!({ "body": "Edited" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 42, "body": "Edited"
            })))
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let comment = client
            .update_pr_comment(
                &RepoId::new("owner/repo"),
                123,
                42,
                UpdateComment {
                    body: "Edited".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(comment.body.as_deref(), Some("Edited"));
    }
}
