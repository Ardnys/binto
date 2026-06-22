use anyhow::{Context, Result, bail};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, IF_NONE_MATCH};

use super::types::Release;

pub struct ApiResponse<T> {
    pub data: T,
    pub etag: Option<String>,
}

pub enum ConditionalResult<T> {
    Changed(ApiResponse<T>),
    NotModified,
}

#[derive(Clone)]
pub struct GithubClient {
    client: reqwest::Client,
    token: Option<String>,
}

impl GithubClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Accept",
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );

        if let Some(ref tok) = token {
            let val =
                HeaderValue::from_str(&format!("Bearer {tok}")).context("invalid token value")?;
            headers.insert(AUTHORIZATION, val);
        }

        let client = reqwest::Client::builder()
            .user_agent(concat!("binto/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self { client, token })
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Resolve token: GITHUB_TOKEN env var takes precedence over config.
    pub fn resolve_token(config_token: Option<String>) -> Option<String> {
        std::env::var("GITHUB_TOKEN").ok().or(config_token)
    }

    #[tracing::instrument(skip(self), fields(repo = %repo), err(level = "debug"))]
    pub async fn list_releases(&self, repo: &str) -> Result<Vec<Release>> {
        let url = format!("https://api.github.com/repos/{repo}/releases");
        tracing::debug!("listing releases");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("failed to fetch releases for {repo}"))?;

        self.check_rate_limit(&resp);

        if resp.status() == reqwest::StatusCode::FORBIDDEN
            && let Some(reset) = resp
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        {
            bail!(crate::error::BintoError::RateLimitExceeded { reset_time: reset });
        }

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            bail!(crate::error::BintoError::ApiError {
                status,
                message: body
            });
        }

        resp.json::<Vec<Release>>()
            .await
            .with_context(|| format!("failed to deserialize releases for {repo}"))
    }

    /// Fetch a single release by its exact tag (`GET /repos/{repo}/releases/tags/{tag}`).
    /// Used for version pinning. A 404 means there is no release with that tag.
    #[tracing::instrument(skip(self), fields(repo = %repo, tag = %tag), err(level = "debug"))]
    pub async fn get_release_by_tag(&self, repo: &str, tag: &str) -> Result<Release> {
        let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
        tracing::debug!("fetching release by tag");
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("failed to fetch release {tag} for {repo}"))?;

        self.check_rate_limit(&resp);

        if resp.status() == reqwest::StatusCode::FORBIDDEN
            && let Some(reset) = resp
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        {
            bail!(crate::error::BintoError::RateLimitExceeded { reset_time: reset });
        }

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            bail!("no release tagged '{tag}' found for {repo}");
        }

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            bail!(crate::error::BintoError::ApiError {
                status,
                message: body
            });
        }

        resp.json::<Release>()
            .await
            .with_context(|| format!("failed to deserialize release {tag} for {repo}"))
    }

    #[tracing::instrument(skip(self, etag), fields(repo = %repo, conditional = etag.is_some()), err(level = "debug"))]
    pub async fn get_latest_release(
        &self,
        repo: &str,
        etag: Option<&str>,
    ) -> Result<ConditionalResult<Release>> {
        let url = format!("https://api.github.com/repos/{repo}/releases/latest");
        tracing::debug!("fetching latest release for {repo}");
        let mut req = self.client.get(&url);

        if let Some(tag) = etag {
            req = req.header(IF_NONE_MATCH, tag);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("failed to fetch latest release for {repo}"))?;

        self.check_rate_limit(&resp);

        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            tracing::debug!("304 Not Modified");
            return Ok(ConditionalResult::NotModified);
        }

        if resp.status() == reqwest::StatusCode::FORBIDDEN
            && let Some(reset) = resp
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        {
            bail!(crate::error::BintoError::RateLimitExceeded { reset_time: reset });
        }

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            bail!(crate::error::BintoError::ApiError {
                status,
                message: body
            });
        }

        let new_etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let release = resp
            .json::<Release>()
            .await
            .with_context(|| format!("failed to deserialize release for {repo}"))?;

        Ok(ConditionalResult::Changed(ApiResponse {
            data: release,
            etag: new_etag,
        }))
    }

    fn check_rate_limit(&self, resp: &reqwest::Response) {
        if let Some(remaining) = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u32>().ok())
            && remaining < 10
            && remaining > 0
        {
            let hint = if self.token.is_none() {
                " Set GITHUB_TOKEN to increase the limit."
            } else {
                ""
            };
            crate::output::print_warning(&format!(
                "GitHub API rate limit almost exhausted ({remaining} requests remaining).{hint}"
            ));
        }
    }
}
