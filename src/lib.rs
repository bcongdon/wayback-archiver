use chrono::{Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// TODO: Figure out how to refactor this to return `Result<String, ArchiveError>`.
pub async fn archive_url(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Check to see if there's an existing archive of the requested URL.
    let resp = reqwest::get(format!("http://archive.org/wayback/available?url={}", url))
        .await?
        .json::<WaybackAvailabilityResponse>()
        .await?;

    if let Some(snapshots) = resp.archived_snapshots {
        if let Some((_, latest)) = snapshots
            .iter()
            .max_by_key(|(_, snapshot)| &snapshot.timestamp)
        {
            let ts = NaiveDateTime::parse_from_str(&latest.timestamp, "%Y%m%d%H%M%S")
                .expect("snapshot timestamp");
            // Only accept the existing snapshot if it was made recently.
            if (Utc::now() - Duration::days(90)).naive_utc() < ts {
                return Ok(latest.url.clone());
            }
        }
    }

    // Request a new snapshot of the URL.
    let resp = reqwest::get(format!("https://web.archive.org/save/{}", url)).await?;
    match resp.status().as_u16() {
        // Return the redirected URL (which is the archive snapshot URL).
        200 => Ok(resp.url().clone().to_string()),
        404 => {
            // Sometimes, the snapshot URL returns a 404, even though the archival was successful.
            // Probably due to a race condition in the Wayback machine; these URLs do (eventually) exist.
            if resp.url().path().starts_with("/web") {
                Ok(resp.url().to_string())
            } else {
                Err(ArchiveError::Unknown(format!(
                    "Unexpected HTTP 404 at {:#?}",
                    resp.url().to_string()
                ))
                .into())
            }
        }
        509 => Err(ArchiveError::BandwidthExceeded.into()),
        // There may be more status codes that indicate archive failure, but these were the most common.
        520 | 523 => Err(ArchiveError::ArchiveFailed.into()),
        _ => {
            dbg!(&resp);
            Err(ArchiveError::Unknown(format!("Got status {}: {:#?}", resp.status(), resp)).into())
        }
    }
}

#[derive(Deserialize, Debug)]
struct WaybackAvailabilityResponse {
    url: String,
    archived_snapshots: Option<HashMap<String, WaybackSnapshot>>,
}

#[derive(Deserialize, Debug)]
struct WaybackSnapshot {
    status: String,
    available: bool,
    url: String,
    timestamp: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ArchivingResult {
    pub url: Option<String>,
    pub last_archived: NaiveDateTime,
}

#[derive(Debug)]
pub enum ArchiveError {
    BandwidthExceeded,
    ArchiveFailed,
    Unknown(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::BandwidthExceeded => write!(f, "Bandwidth Exceeded"),
            ArchiveError::ArchiveFailed => write!(f, "Wayback Machine unable to archive this URL"),
            ArchiveError::Unknown(err) => write!(f, "Unknown error: {}", err),
        }
    }
}

impl std::error::Error for ArchiveError {}
