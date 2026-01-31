//! GitHub API client.

use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;

use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::traits::GitHubApi;
use crate::types::{
    CheckRun, CreateComment, CreatePullRequest, IssueComment, MergePullRequest, MergeResult,
    PullRequest, PullRequestState, UpdateComment, UpdatePullRequest,
};

// === Internal API response types (shared across methods) ===

/// Internal representation of a PR from the GitHub API.
#[derive(serde::Deserialize)]
struct ApiPullRequest {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    /// Whether the PR was merged (GitHub returns state="closed" + merged=true for merged PRs).
    #[serde(default)]
    merged: bool,
    draft: bool,
    html_url: String,
    head: ApiBranch,
    base: ApiBranch,
    /// Whether the PR is mergeable (None if GitHub is still computing).
    mergeable: Option<bool>,
    /// The mergeable state (e.g., "clean", "dirty", "blocked", "behind").
    mergeable_state: Option<String>,
}

/// Internal representation of a branch ref from the GitHub API.
#[derive(serde::Deserialize)]
struct ApiBranch {
    #[serde(rename = "ref")]
    ref_name: String,
}

impl ApiPullRequest {
    /// Convert API response to domain type, parsing state string.
    fn into_pull_request(self) -> PullRequest {
        // GitHub API returns state="closed" + merged=true for merged PRs
        let state = if self.merged {
            PullRequestState::Merged
        } else {
            match self.state.as_str() {
                "open" => PullRequestState::Open,
                _ => PullRequestState::Closed,
            }
        };

        PullRequest {
            number: self.number,
            title: self.title,
            body: self.body,
            state,
            draft: self.draft,
            head_branch: self.head.ref_name,
            base_branch: self.base.ref_name,
            html_url: self.html_url,
            mergeable: self.mergeable,
            mergeable_state: self.mergeable_state,
        }
    }

    /// Convert API response to domain type with a known state.
    fn into_pull_request_with_state(self, state: PullRequestState) -> PullRequest {
        PullRequest {
            number: self.number,
            title: self.title,
            body: self.body,
            state,
            draft: self.draft,
            head_branch: self.head.ref_name,
            base_branch: self.base.ref_name,
            html_url: self.html_url,
            mergeable: self.mergeable,
            mergeable_state: self.mergeable_state,
        }
    }
}

// === GraphQL types for batch PR fetching ===

/// GraphQL request wrapper.
#[derive(serde::Serialize)]
struct GraphQLRequest {
    query: String,
    variables: GraphQLVariables,
}

/// GraphQL variables for PR batch query.
#[derive(serde::Serialize)]
struct GraphQLVariables {
    owner: String,
    repo: String,
}

/// GraphQL PR response (different field names than REST API).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLPullRequest {
    number: u64,
    state: String,
    merged: bool,
    is_draft: bool,
    head_ref_name: String,
    base_ref_name: String,
    url: String,
}

impl GraphQLPullRequest {
    fn into_pull_request(self) -> PullRequest {
        let state = if self.merged {
            PullRequestState::Merged
        } else if self.state == "OPEN" {
            PullRequestState::Open
        } else {
            PullRequestState::Closed
        };

        PullRequest {
            number: self.number,
            title: String::new(), // Not fetched in batch query
            body: None,
            state,
            draft: self.is_draft,
            head_branch: self.head_ref_name,
            base_branch: self.base_ref_name,
            html_url: self.url,
            mergeable: None, // Not fetched in batch query
            mergeable_state: None,
        }
    }
}

#[derive(serde::Deserialize)]
struct GraphQLResponse {
    data: Option<GraphQLData>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(serde::Deserialize)]
struct GraphQLData {
    repository: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct GraphQLError {
    message: String,
}

/// GitHub API client.
pub struct GitHubClient {
    client: Client,
    base_url: String,
    /// Token stored as `SecretString` for automatic zeroization on drop.
    token: SecretString,
}

impl GitHubClient {
    /// Default GitHub API URL.
    pub const DEFAULT_API_URL: &'static str = "https://api.github.com";

    /// Create a new GitHub client.
    ///
    /// # Errors
    /// Returns error if authentication fails.
    pub fn new(auth: &Auth) -> Result<Self> {
        Self::with_base_url(auth, Self::DEFAULT_API_URL)
    }

    /// Create a new GitHub client with a custom API URL (for GitHub Enterprise).
    ///
    /// # Errors
    /// Returns error if authentication fails.
    pub fn with_base_url(auth: &Auth, base_url: impl Into<String>) -> Result<Self> {
        let token = auth.resolve()?;

        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("rung-cli"));
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );

        let client = Client::builder().default_headers(headers).build()?;

        Ok(Self {
            client,
            base_url: base_url.into(),
            token,
        })
    }

    /// Make a GET request.
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .get(&url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Make a POST request.
    async fn post<T: DeserializeOwned, B: serde::Serialize + Sync>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .post(&url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .json(body)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Make a PATCH request.
    async fn patch<T: DeserializeOwned, B: serde::Serialize + Sync>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .patch(&url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .json(body)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Make a PUT request.
    async fn put<T: DeserializeOwned, B: serde::Serialize + Sync>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .put(&url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .json(body)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Make a DELETE request.
    async fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .delete(&url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .send()
            .await?;

        let status = response.status();
        if status.is_success() || status.as_u16() == 204 {
            return Ok(());
        }

        let status_code = status.as_u16();
        match status_code {
            401 => Err(Error::AuthenticationFailed),
            403 if response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0") =>
            {
                Err(Error::RateLimited)
            }
            _ => {
                let text = response.text().await.unwrap_or_default();
                Err(Error::ApiError {
                    status: status_code,
                    message: text,
                })
            }
        }
    }

    /// Handle API response.
    async fn handle_response<T: DeserializeOwned>(&self, response: reqwest::Response) -> Result<T> {
        let status = response.status();

        if status.is_success() {
            let body = response.json().await?;
            return Ok(body);
        }

        // Handle error responses
        let status_code = status.as_u16();

        match status_code {
            401 => Err(Error::AuthenticationFailed),
            403 if response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0") =>
            {
                Err(Error::RateLimited)
            }
            _ => {
                let text = response.text().await.unwrap_or_default();
                Err(Error::ApiError {
                    status: status_code,
                    message: text,
                })
            }
        }
    }

    // === PR Operations ===

    /// Get a pull request by number.
    ///
    /// # Errors
    /// Returns error if PR not found or API call fails.
    pub async fn get_pr(&self, owner: &str, repo: &str, number: u64) -> Result<PullRequest> {
        let api_pr: ApiPullRequest = self
            .get(&format!("/repos/{owner}/{repo}/pulls/{number}"))
            .await?;

        Ok(api_pr.into_pull_request())
    }

    /// Get multiple pull requests by number using GraphQL (single API call).
    ///
    /// This is more efficient than calling `get_pr` multiple times when fetching
    /// many PRs, as it uses a single GraphQL query instead of N REST calls.
    ///
    /// Returns a map of PR number to PR data. PRs that don't exist or can't be
    /// fetched are omitted from the result (no error is returned for missing PRs).
    ///
    /// # Errors
    /// Returns error if the GraphQL request fails entirely.
    pub async fn get_prs_batch(
        &self,
        owner: &str,
        repo: &str,
        numbers: &[u64],
    ) -> Result<std::collections::HashMap<u64, PullRequest>> {
        if numbers.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let query = build_graphql_pr_query(numbers);
        let request = GraphQLRequest {
            query,
            variables: GraphQLVariables {
                owner: owner.to_string(),
                repo: repo.to_string(),
            },
        };
        let url = format!("{}/graphql", self.base_url);

        let response = self
            .client
            .post(&url)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.token.expose_secret()),
            )
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            return match status_code {
                401 => Err(Error::AuthenticationFailed),
                403 if response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .is_some_and(|v| v == "0") =>
                {
                    Err(Error::RateLimited)
                }
                _ => {
                    let text = response.text().await.unwrap_or_default();
                    Err(Error::ApiError {
                        status: status_code,
                        message: text,
                    })
                }
            };
        }

        let graphql_response: GraphQLResponse = response.json().await?;

        // Check for GraphQL-level errors
        if let Some(errors) = graphql_response.errors
            && !errors.is_empty()
        {
            let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
            return Err(Error::ApiError {
                status: 200,
                message: messages.join("; "),
            });
        }

        let mut result = std::collections::HashMap::new();

        if let Some(data) = graphql_response.data
            && let Some(repo_data) = data.repository
        {
            // Parse each pr0, pr1, pr2... field
            for (i, &num) in numbers.iter().enumerate() {
                let key = format!("pr{i}");
                if let Some(pr_value) = repo_data.get(&key)
                    && !pr_value.is_null()
                    && let Ok(pr) = serde_json::from_value::<GraphQLPullRequest>(pr_value.clone())
                {
                    result.insert(num, pr.into_pull_request());
                }
            }
        }

        Ok(result)
    }

    /// Find a PR for a branch.
    ///
    /// # Errors
    /// Returns error if API call fails.
    pub async fn find_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PullRequest>> {
        // We only query open PRs, so state is always Open
        let prs: Vec<ApiPullRequest> = self
            .get(&format!(
                "/repos/{owner}/{repo}/pulls?head={owner}:{branch}&state=open"
            ))
            .await?;

        Ok(prs
            .into_iter()
            .next()
            .map(|api_pr| api_pr.into_pull_request_with_state(PullRequestState::Open)))
    }

    /// Create a pull request.
    ///
    /// # Errors
    /// Returns error if PR creation fails.
    pub async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        pr: CreatePullRequest,
    ) -> Result<PullRequest> {
        // Newly created PRs are always open
        let api_pr: ApiPullRequest = self
            .post(&format!("/repos/{owner}/{repo}/pulls"), &pr)
            .await?;

        Ok(api_pr.into_pull_request_with_state(PullRequestState::Open))
    }

    /// Update a pull request.
    ///
    /// # Errors
    /// Returns error if PR update fails.
    pub async fn update_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        update: UpdatePullRequest,
    ) -> Result<PullRequest> {
        let api_pr: ApiPullRequest = self
            .patch(&format!("/repos/{owner}/{repo}/pulls/{number}"), &update)
            .await?;

        Ok(api_pr.into_pull_request())
    }

    // === Check Runs ===

    /// Get check runs for a commit.
    ///
    /// # Errors
    /// Returns error if API call fails.
    pub async fn get_check_runs(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> Result<Vec<CheckRun>> {
        #[derive(serde::Deserialize)]
        struct Response {
            check_runs: Vec<ApiCheckRun>,
        }

        #[derive(serde::Deserialize)]
        struct ApiCheckRun {
            name: String,
            status: String,
            conclusion: Option<String>,
            details_url: Option<String>,
        }

        let response: Response = self
            .get(&format!(
                "/repos/{owner}/{repo}/commits/{commit_sha}/check-runs"
            ))
            .await?;

        Ok(response
            .check_runs
            .into_iter()
            .map(|cr| CheckRun {
                name: cr.name,
                status: match (cr.status.as_str(), cr.conclusion.as_deref()) {
                    ("queued", _) => crate::types::CheckStatus::Queued,
                    ("in_progress", _) => crate::types::CheckStatus::InProgress,
                    ("completed", Some("success")) => crate::types::CheckStatus::Success,
                    ("completed", Some("skipped")) => crate::types::CheckStatus::Skipped,
                    ("completed", Some("cancelled")) => crate::types::CheckStatus::Cancelled,
                    // Any other status (failure, timed_out, action_required, etc.) treated as failure
                    _ => crate::types::CheckStatus::Failure,
                },
                details_url: cr.details_url,
            })
            .collect())
    }

    // === Merge Operations ===

    /// Merge a pull request.
    ///
    /// # Errors
    /// Returns error if merge fails.
    pub async fn merge_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        merge: MergePullRequest,
    ) -> Result<MergeResult> {
        self.put(
            &format!("/repos/{owner}/{repo}/pulls/{number}/merge"),
            &merge,
        )
        .await
    }

    // === Ref Operations ===

    /// Delete a git reference (branch).
    ///
    /// # Errors
    /// Returns error if deletion fails.
    pub async fn delete_ref(&self, owner: &str, repo: &str, ref_name: &str) -> Result<()> {
        self.delete(&format!("/repos/{owner}/{repo}/git/refs/heads/{ref_name}"))
            .await
    }

    // === Repository Operations ===

    /// Get the repository's default branch name.
    ///
    /// # Errors
    /// Returns error if API call fails.
    pub async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        #[derive(serde::Deserialize)]
        struct RepoInfo {
            default_branch: String,
        }

        let info: RepoInfo = self.get(&format!("/repos/{owner}/{repo}")).await?;
        Ok(info.default_branch)
    }

    // === Comment Operations ===

    /// List comments on a pull request.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn list_pr_comments(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<crate::types::IssueComment>> {
        self.get(&format!(
            "/repos/{owner}/{repo}/issues/{pr_number}/comments"
        ))
        .await
    }

    /// Create a comment on a pull request.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn create_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        comment: crate::types::CreateComment,
    ) -> Result<crate::types::IssueComment> {
        self.post(
            &format!("/repos/{owner}/{repo}/issues/{pr_number}/comments"),
            &comment,
        )
        .await
    }

    /// Update a comment on a pull request.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn update_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        comment: crate::types::UpdateComment,
    ) -> Result<crate::types::IssueComment> {
        self.patch(
            &format!("/repos/{owner}/{repo}/issues/comments/{comment_id}"),
            &comment,
        )
        .await
    }
}

impl std::fmt::Debug for GitHubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubClient")
            .field("base_url", &self.base_url)
            .field("token", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// Build a GraphQL query to fetch multiple PRs in a single request.
fn build_graphql_pr_query(numbers: &[u64]) -> String {
    const PR_FIELDS: &str = "number state merged isDraft headRefName baseRefName url";

    let pr_queries: Vec<String> = numbers
        .iter()
        .enumerate()
        .map(|(i, num)| format!("pr{i}: pullRequest(number: {num}) {{ {PR_FIELDS} }}"))
        .collect();

    format!(
        r"query($owner: String!, $repo: String!) {{ repository(owner: $owner, name: $repo) {{ {pr_queries} }} }}",
        pr_queries = pr_queries.join(" ")
    )
}

// === Trait Implementation ===

impl GitHubApi for GitHubClient {
    async fn get_pr(&self, owner: &str, repo: &str, number: u64) -> Result<PullRequest> {
        self.get_pr(owner, repo, number).await
    }

    async fn get_prs_batch(
        &self,
        owner: &str,
        repo: &str,
        numbers: &[u64],
    ) -> Result<std::collections::HashMap<u64, PullRequest>> {
        self.get_prs_batch(owner, repo, numbers).await
    }

    async fn find_pr_for_branch(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PullRequest>> {
        self.find_pr_for_branch(owner, repo, branch).await
    }

    async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        pr: CreatePullRequest,
    ) -> Result<PullRequest> {
        self.create_pr(owner, repo, pr).await
    }

    async fn update_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        update: UpdatePullRequest,
    ) -> Result<PullRequest> {
        self.update_pr(owner, repo, number, update).await
    }

    async fn get_check_runs(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> Result<Vec<CheckRun>> {
        self.get_check_runs(owner, repo, commit_sha).await
    }

    async fn merge_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        merge: MergePullRequest,
    ) -> Result<MergeResult> {
        self.merge_pr(owner, repo, number, merge).await
    }

    async fn delete_ref(&self, owner: &str, repo: &str, ref_name: &str) -> Result<()> {
        self.delete_ref(owner, repo, ref_name).await
    }

    async fn get_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        self.get_default_branch(owner, repo).await
    }

    async fn list_pr_comments(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<Vec<IssueComment>> {
        self.list_pr_comments(owner, repo, pr_number).await
    }

    async fn create_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        comment: CreateComment,
    ) -> Result<IssueComment> {
        self.create_pr_comment(owner, repo, pr_number, comment)
            .await
    }

    async fn update_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        comment: UpdateComment,
    ) -> Result<IssueComment> {
        self.update_pr_comment(owner, repo, comment_id, comment)
            .await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::types::{CheckStatus, MergeMethod};
    use secrecy::SecretString;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create a test client pointing to the mock server.
    fn test_client(base_url: &str) -> GitHubClient {
        let auth = Auth::Token(SecretString::from("test-token"));
        GitHubClient::with_base_url(&auth, base_url).unwrap()
    }

    /// Standard PR response JSON for testing.
    fn pr_response_json(number: u64, state: &str, merged: bool) -> serde_json::Value {
        serde_json::json!({
            "number": number,
            "title": format!("PR #{number}"),
            "body": "Test body",
            "state": state,
            "merged": merged,
            "draft": false,
            "html_url": format!("https://github.com/owner/repo/pull/{number}"),
            "head": { "ref": "feature-branch" },
            "base": { "ref": "main" },
            "mergeable": true,
            "mergeable_state": "clean"
        })
    }

    // === GET PR Tests ===

    #[tokio::test]
    async fn test_get_pr_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls/123"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pr_response_json(123, "open", false)),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let pr = client.get_pr("owner", "repo", 123).await.unwrap();

        assert_eq!(pr.number, 123);
        assert_eq!(pr.title, "PR #123");
        assert_eq!(pr.state, PullRequestState::Open);
        assert_eq!(pr.head_branch, "feature-branch");
        assert_eq!(pr.base_branch, "main");
    }

    #[tokio::test]
    async fn test_get_pr_merged() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls/456"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pr_response_json(456, "closed", true)),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let pr = client.get_pr("owner", "repo", 456).await.unwrap();

        assert_eq!(pr.state, PullRequestState::Merged);
    }

    #[tokio::test]
    async fn test_get_pr_closed() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls/789"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pr_response_json(789, "closed", false)),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let pr = client.get_pr("owner", "repo", 789).await.unwrap();

        assert_eq!(pr.state, PullRequestState::Closed);
    }

    #[tokio::test]
    async fn test_get_pr_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls/999"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "Not Found"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.get_pr("owner", "repo", 999).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, Error::ApiError { status: 404, .. }));
    }

    // === Authentication Error Tests ===

    #[tokio::test]
    async fn test_unauthorized_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls/123"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "message": "Bad credentials"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.get_pr("owner", "repo", 123).await;

        assert!(matches!(result, Err(Error::AuthenticationFailed)));
    }

    #[tokio::test]
    async fn test_rate_limited_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls/123"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .set_body_json(serde_json::json!({
                        "message": "API rate limit exceeded"
                    })),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.get_pr("owner", "repo", 123).await;

        assert!(matches!(result, Err(Error::RateLimited)));
    }

    // === Find PR for Branch Tests ===

    #[tokio::test]
    async fn test_find_pr_for_branch_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .and(query_param("head", "owner:feature"))
            .and(query_param("state", "open"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([pr_response_json(42, "open", false)])),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let pr = client
            .find_pr_for_branch("owner", "repo", "feature")
            .await
            .unwrap();

        assert!(pr.is_some());
        assert_eq!(pr.unwrap().number, 42);
    }

    #[tokio::test]
    async fn test_find_pr_for_branch_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let pr = client
            .find_pr_for_branch("owner", "repo", "nonexistent")
            .await
            .unwrap();

        assert!(pr.is_none());
    }

    // === Create PR Tests ===

    #[tokio::test]
    async fn test_create_pr_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/pulls"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(pr_response_json(100, "open", false)),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let create_pr = CreatePullRequest {
            title: "New Feature".into(),
            body: "Description".into(),
            head: "feature".into(),
            base: "main".into(),
            draft: false,
        };

        let pr = client.create_pr("owner", "repo", create_pr).await.unwrap();

        assert_eq!(pr.number, 100);
        assert_eq!(pr.state, PullRequestState::Open);
    }

    // === Update PR Tests ===

    #[tokio::test]
    async fn test_update_pr_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("PATCH"))
            .and(path("/repos/owner/repo/pulls/123"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(pr_response_json(123, "open", false)),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let update = UpdatePullRequest {
            title: Some("Updated Title".into()),
            body: None,
            base: None,
        };

        let pr = client
            .update_pr("owner", "repo", 123, update)
            .await
            .unwrap();

        assert_eq!(pr.number, 123);
    }

    // === Get Check Runs Tests ===

    #[tokio::test]
    async fn test_get_check_runs_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/commits/abc123/check-runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 3,
                "check_runs": [
                    {
                        "name": "CI",
                        "status": "completed",
                        "conclusion": "success",
                        "details_url": "https://example.com/ci"
                    },
                    {
                        "name": "Lint",
                        "status": "in_progress",
                        "conclusion": null,
                        "details_url": null
                    },
                    {
                        "name": "Deploy",
                        "status": "queued",
                        "conclusion": null,
                        "details_url": null
                    }
                ]
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let checks = client
            .get_check_runs("owner", "repo", "abc123")
            .await
            .unwrap();

        assert_eq!(checks.len(), 3);
        assert_eq!(checks[0].name, "CI");
        assert_eq!(checks[0].status, CheckStatus::Success);
        assert_eq!(checks[1].name, "Lint");
        assert_eq!(checks[1].status, CheckStatus::InProgress);
        assert_eq!(checks[2].name, "Deploy");
        assert_eq!(checks[2].status, CheckStatus::Queued);
    }

    #[tokio::test]
    async fn test_get_check_runs_various_statuses() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/commits/def456/check-runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 4,
                "check_runs": [
                    { "name": "skipped", "status": "completed", "conclusion": "skipped", "details_url": null },
                    { "name": "cancelled", "status": "completed", "conclusion": "cancelled", "details_url": null },
                    { "name": "failure", "status": "completed", "conclusion": "failure", "details_url": null },
                    { "name": "timed_out", "status": "completed", "conclusion": "timed_out", "details_url": null }
                ]
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let checks = client
            .get_check_runs("owner", "repo", "def456")
            .await
            .unwrap();

        assert_eq!(checks[0].status, CheckStatus::Skipped);
        assert_eq!(checks[1].status, CheckStatus::Cancelled);
        assert_eq!(checks[2].status, CheckStatus::Failure);
        assert_eq!(checks[3].status, CheckStatus::Failure); // timed_out maps to failure
    }

    // === Merge PR Tests ===

    #[tokio::test]
    async fn test_merge_pr_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/repos/owner/repo/pulls/123/merge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sha": "abc123def456",
                "merged": true,
                "message": "Pull Request successfully merged"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let merge = MergePullRequest {
            commit_title: Some("Merge PR #123".into()),
            commit_message: None,
            merge_method: MergeMethod::Squash,
        };

        let result = client.merge_pr("owner", "repo", 123, merge).await.unwrap();

        assert!(result.merged);
        assert_eq!(result.sha, "abc123def456");
    }

    // === Delete Ref Tests ===

    #[tokio::test]
    async fn test_delete_ref_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/repos/owner/repo/git/refs/heads/feature-branch"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.delete_ref("owner", "repo", "feature-branch").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_ref_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/repos/owner/repo/git/refs/heads/nonexistent"))
            .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "message": "Reference does not exist"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.delete_ref("owner", "repo", "nonexistent").await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_ref_rate_limited() {
        let mock_server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/repos/owner/repo/git/refs/heads/branch"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .set_body_json(serde_json::json!({ "message": "Rate limited" })),
            )
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.delete_ref("owner", "repo", "branch").await;

        assert!(matches!(result, Err(Error::RateLimited)));
    }

    // === Get Default Branch Tests ===

    #[tokio::test]
    async fn test_get_default_branch_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "default_branch": "main"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let branch = client.get_default_branch("owner", "repo").await.unwrap();

        assert_eq!(branch, "main");
    }

    // === Comment Tests ===

    #[tokio::test]
    async fn test_list_pr_comments_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/issues/123/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 1, "body": "First comment" },
                { "id": 2, "body": "Second comment" }
            ])))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let comments = client.list_pr_comments("owner", "repo", 123).await.unwrap();

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].id, 1);
        assert_eq!(comments[0].body, Some("First comment".into()));
    }

    #[tokio::test]
    async fn test_create_pr_comment_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/123/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": 42,
                "body": "New comment"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let comment = CreateComment {
            body: "New comment".into(),
        };

        let result = client
            .create_pr_comment("owner", "repo", 123, comment)
            .await
            .unwrap();

        assert_eq!(result.id, 42);
        assert_eq!(result.body, Some("New comment".into()));
    }

    #[tokio::test]
    async fn test_update_pr_comment_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("PATCH"))
            .and(path("/repos/owner/repo/issues/comments/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 42,
                "body": "Updated comment"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let update = UpdateComment {
            body: "Updated comment".into(),
        };

        let result = client
            .update_pr_comment("owner", "repo", 42, update)
            .await
            .unwrap();

        assert_eq!(result.body, Some("Updated comment".into()));
    }

    // === GraphQL Batch Tests ===

    #[tokio::test]
    async fn test_get_prs_batch_empty() {
        let mock_server = MockServer::start().await;
        let client = test_client(&mock_server.uri());

        // Empty input should return empty map without making any requests
        let result = client.get_prs_batch("owner", "repo", &[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_get_prs_batch_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "repository": {
                        "pr0": {
                            "number": 1,
                            "state": "OPEN",
                            "merged": false,
                            "isDraft": false,
                            "headRefName": "feature-1",
                            "baseRefName": "main",
                            "url": "https://github.com/owner/repo/pull/1"
                        },
                        "pr1": {
                            "number": 2,
                            "state": "MERGED",
                            "merged": true,
                            "isDraft": false,
                            "headRefName": "feature-2",
                            "baseRefName": "main",
                            "url": "https://github.com/owner/repo/pull/2"
                        },
                        "pr2": null
                    }
                }
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client
            .get_prs_batch("owner", "repo", &[1, 2, 999])
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(&1).unwrap().state, PullRequestState::Open);
        assert_eq!(result.get(&2).unwrap().state, PullRequestState::Merged);
        assert!(!result.contains_key(&999)); // PR 999 was null
    }

    #[tokio::test]
    async fn test_get_prs_batch_graphql_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": null,
                "errors": [
                    { "message": "Something went wrong" }
                ]
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.get_prs_batch("owner", "repo", &[1]).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, Error::ApiError { status: 200, .. }));
    }

    #[tokio::test]
    async fn test_get_prs_batch_auth_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "message": "Bad credentials"
            })))
            .mount(&mock_server)
            .await;

        let client = test_client(&mock_server.uri());
        let result = client.get_prs_batch("owner", "repo", &[1]).await;

        assert!(matches!(result, Err(Error::AuthenticationFailed)));
    }

    // === Helper Function Tests ===

    #[test]
    fn test_build_graphql_pr_query() {
        let query = build_graphql_pr_query(&[1, 42, 100]);

        assert!(query.contains("pr0: pullRequest(number: 1)"));
        assert!(query.contains("pr1: pullRequest(number: 42)"));
        assert!(query.contains("pr2: pullRequest(number: 100)"));
        assert!(query.contains("$owner: String!"));
        assert!(query.contains("$repo: String!"));
    }

    // === Debug Implementation Test ===

    #[test]
    fn test_github_client_debug_redacts_token() {
        let auth = Auth::Token(SecretString::from("super-secret-token"));
        let client = GitHubClient::with_base_url(&auth, "https://api.example.com").unwrap();

        let debug_output = format!("{client:?}");

        assert!(debug_output.contains("[redacted]"));
        assert!(!debug_output.contains("super-secret-token"));
    }
}
