use chrono::{Duration, NaiveDateTime, Utc};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// TODO: Figure out how to refactor this to return `Result<String, ArchiveError>`.
// TODO: Have a way for this to return if it was an existing snapshot, or a fresh archive.
pub async fn archive_url(url: &str) -> Result<ArchivingResult, Box<dyn std::error::Error>> {
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
            // TODO: Fix the expect() here.
            let ts = NaiveDateTime::parse_from_str(&latest.timestamp, "%Y%m%d%H%M%S")
                .expect("snapshot timestamp");
            // Only accept the existing snapshot if it was made recently.
            if (Utc::now() - Duration::days(90)).naive_utc() < ts {
                return Ok(ArchivingResult {
                    existing_snapshot: true,
                    last_archived: ts,
                    url: Some(latest.url.clone()),
                });
            }
        }
    }

    // Request a new snapshot of the URL.
    let resp = reqwest::get(format!("https://web.archive.org/save/{}", url)).await?;
    let archive_url = match resp.status().as_u16() {
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
        403 | 520 | 523 => Err(ArchiveError::UnableToArchive.into()),
        _ => {
            dbg!(&resp);
            Err(ArchiveError::Unknown(format!("Got status {}: {:#?}", resp.status(), resp)).into())
        }
    };
    archive_url
        .and_then(|url| {
            Ok(ArchivingResult {
                last_archived: timestamp_from_archive_url(&url)?,
                url: Some(url),
                existing_snapshot: false,
            })
        })
        .into()
}

fn timestamp_from_archive_url(url: &str) -> Result<NaiveDateTime, Box<dyn std::error::Error>> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"/web/(\d+)/").unwrap();
    }
    let timestamp_url = RE
        .captures(url)
        .and_then(|cap| cap.get(1).map(|ts_str| ts_str.as_str()))
        .ok_or("unable to extract timestamp from url")?;
    // TODO: Fix the expect() here.
    Ok(NaiveDateTime::parse_from_str(timestamp_url, "%Y%m%d%H%M%S").expect("timestamp parse"))
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
    #[serde(skip)]
    pub existing_snapshot: bool,
}

#[derive(Debug)]
pub enum ArchiveError {
    BandwidthExceeded,
    UnableToArchive,
    Unknown(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::BandwidthExceeded => write!(f, "Bandwidth Exceeded"),
            ArchiveError::UnableToArchive => {
                write!(f, "Wayback Machine unable to archive this URL")
            }
            ArchiveError::Unknown(err) => write!(f, "Unknown error: {}", err),
        }
    }
}

impl std::error::Error for ArchiveError {}
