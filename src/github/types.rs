use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub published_at: DateTime<Utc>,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<Asset>,
    pub html_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    // GitHub always sends these; defaulting them lets the `match` test harness feed minimal
    // fixtures ({"name": "..."}) since the matcher only looks at `name`.
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub content_type: String,
}
