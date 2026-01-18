//! GitHub API client.

use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;

use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::types::{
    CheckRun, CreatePullRequest, MergePullRequest, MergeResult, PullRequest, PullRequestState,
    UpdatePullRequest,
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
        if let Some(errors) = graphql_response.errors {
            if !errors.is_empty() {
                let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
                return Err(Error::ApiError {
                    status: 200,
                    message: messages.join("; "),
                });
            }
        }

        let mut result = std::collections::HashMap::new();

        if let Some(data) = graphql_response.data {
            if let Some(repo_data) = data.repository {
                // Parse each pr0, pr1, pr2... field
                for (i, &num) in numbers.iter().enumerate() {
                    let key = format!("pr{i}");
                    if let Some(pr_value) = repo_data.get(&key) {
                        // Skip null values (PR doesn't exist)
                        if !pr_value.is_null() {
                            if let Ok(pr) =
                                serde_json::from_value::<GraphQLPullRequest>(pr_value.clone())
                            {
                                result.insert(num, pr.into_pull_request());
                            }
                        }
                    }
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
