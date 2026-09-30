use chrono::{DateTime, Utc};
use serde::Deserialize;

/// The asset shape is part of the harness contract (it is what `binto match` reads), so it
/// is defined once in `binto-contract` and re-exported here for the rest of binto.
pub use binto_contract::Asset;

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub published_at: DateTime<Utc>,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<Asset>,
    pub html_url: String,
}
