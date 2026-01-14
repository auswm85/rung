//! GitHub API client.

use reqwest::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde::de::DeserializeOwned;

use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::types::{
    CheckRun, CreatePullRequest, MergePullRequest, MergeResult, PullRequest, UpdatePullRequest,
};

/// GitHub API client.
pub struct GitHubClient {
    client: Client,
    base_url: String,
    token: String,
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
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
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
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
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
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
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
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
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
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
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
        #[derive(serde::Deserialize)]
        struct ApiPr {
            number: u64,
            title: String,
            body: Option<String>,
            state: String,
            draft: bool,
            html_url: String,
            head: Branch,
            base: Branch,
        }

        #[derive(serde::Deserialize)]
        struct Branch {
            #[serde(rename = "ref")]
            ref_name: String,
        }

        let api_pr: ApiPr = self
            .get(&format!("/repos/{owner}/{repo}/pulls/{number}"))
            .await?;

        Ok(PullRequest {
            number: api_pr.number,
            title: api_pr.title,
            body: api_pr.body,
            state: match api_pr.state.as_str() {
                "open" => crate::types::PullRequestState::Open,
                "merged" => crate::types::PullRequestState::Merged,
                _ => crate::types::PullRequestState::Closed,
            },
            draft: api_pr.draft,
            head_branch: api_pr.head.ref_name,
            base_branch: api_pr.base.ref_name,
            html_url: api_pr.html_url,
        })
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
        #[derive(serde::Deserialize)]
        struct ApiPr {
            number: u64,
            title: String,
            body: Option<String>,
            #[allow(dead_code)]
            state: String,
            draft: bool,
            html_url: String,
            head: Branch,
            base: Branch,
        }

        #[derive(serde::Deserialize)]
        struct Branch {
            #[serde(rename = "ref")]
            ref_name: String,
        }

        // We only query open PRs, so state is always Open
        let prs: Vec<ApiPr> = self
            .get(&format!(
                "/repos/{owner}/{repo}/pulls?head={owner}:{branch}&state=open"
            ))
            .await?;

        Ok(prs.into_iter().next().map(|api_pr| PullRequest {
            number: api_pr.number,
            title: api_pr.title,
            body: api_pr.body,
            state: crate::types::PullRequestState::Open,
            draft: api_pr.draft,
            head_branch: api_pr.head.ref_name,
            base_branch: api_pr.base.ref_name,
            html_url: api_pr.html_url,
        }))
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
        #[derive(serde::Deserialize)]
        struct ApiPr {
            number: u64,
            title: String,
            body: Option<String>,
            #[allow(dead_code)]
            state: String,
            draft: bool,
            html_url: String,
            head: Branch,
            base: Branch,
        }

        #[derive(serde::Deserialize)]
        struct Branch {
            #[serde(rename = "ref")]
            ref_name: String,
        }

        // Newly created PRs are always open
        let api_pr: ApiPr = self
            .post(&format!("/repos/{owner}/{repo}/pulls"), &pr)
            .await?;

        Ok(PullRequest {
            number: api_pr.number,
            title: api_pr.title,
            body: api_pr.body,
            state: crate::types::PullRequestState::Open,
            draft: api_pr.draft,
            head_branch: api_pr.head.ref_name,
            base_branch: api_pr.base.ref_name,
            html_url: api_pr.html_url,
        })
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
        #[derive(serde::Deserialize)]
        struct ApiPr {
            number: u64,
            title: String,
            body: Option<String>,
            state: String,
            draft: bool,
            html_url: String,
            head: Branch,
            base: Branch,
        }

        #[derive(serde::Deserialize)]
        struct Branch {
            #[serde(rename = "ref")]
            ref_name: String,
        }

        let api_pr: ApiPr = self
            .patch(&format!("/repos/{owner}/{repo}/pulls/{number}"), &update)
            .await?;

        Ok(PullRequest {
            number: api_pr.number,
            title: api_pr.title,
            body: api_pr.body,
            state: match api_pr.state.as_str() {
                "open" => crate::types::PullRequestState::Open,
                "merged" => crate::types::PullRequestState::Merged,
                _ => crate::types::PullRequestState::Closed,
            },
            draft: api_pr.draft,
            head_branch: api_pr.head.ref_name,
            base_branch: api_pr.base.ref_name,
            html_url: api_pr.html_url,
        })
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
