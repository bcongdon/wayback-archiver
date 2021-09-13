use chrono::{Duration, NaiveDateTime, TimeZone, Utc};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub async fn archive_url(url: &str) -> Result<ArchivingResult, ArchiveError> {
    // Check to see if there's an existing archive of the requested URL.
    let latest_snapshot = fetch_latest_snapshot(url).await;
    if let Ok(ref snapshot) = latest_snapshot {
        // Only accept the existing snapshot if it was made recently.
        if (Utc::now() - Duration::days(90)).naive_utc() < snapshot.last_archived {
            return latest_snapshot;
        }
    }

    // Request a new snapshot of the URL.
    let resp = reqwest::get(format!("https://web.archive.org/save/{}", url))
        .await
        .map_err(|err| ArchiveError::Unknown(err.to_string()))?;
    let archive_url: Result<String, ArchiveError> = match resp.status().as_u16() {
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
                )))
            }
        }
        509 => Err(ArchiveError::BandwidthExceeded),
        // There may be more status codes that indicate archive failure, but these were the most common.
        403 | 520 | 523 => Err(ArchiveError::UnableToArchive),
        _ => {
            dbg!(&resp);
            Err(ArchiveError::Unknown(format!(
                "Got status {}: {:#?}",
                resp.status(),
                resp
            )))
        }
    };
    let result = archive_url.and_then(|url| {
        Ok(ArchivingResult {
            last_archived: timestamp_from_archive_url(&url)?,
            url: Some(url),
            existing_snapshot: false,
        })
    });
    match result {
        Err(ArchiveError::UnableToArchive) => {
            // If we weren't able to archive the URL, but a valid (if old) snapshot exists,
            // then return that older snapshot.
            latest_snapshot.map_err(|_| ArchiveError::UnableToArchive)
        }
        _ => result,
    }
}

fn timestamp_from_archive_url(url: &str) -> Result<NaiveDateTime, ArchiveError> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"/web/(\d+)/").unwrap();
    }
    let timestamp_url_component = RE
        .captures(url)
        .and_then(|cap| cap.get(1).map(|ts_str| ts_str.as_str()))
        .ok_or_else(|| ArchiveError::ParseError("unable to extract timestamp from url".into()))?;
    parse_wayback_timestamp(timestamp_url_component)
}

async fn fetch_latest_snapshot(url: &str) -> Result<ArchivingResult, ArchiveError> {
    let resp = reqwest::get(format!("http://archive.org/wayback/available?url={}", url))
        .await
        .map_err(|err| ArchiveError::Unknown(err.to_string()))?
        .json::<WaybackAvailabilityResponse>()
        .await
        .map_err(|err| ArchiveError::ParseError(err.to_string()))?;

    if let Some(snapshots) = resp.archived_snapshots {
        if let Some((_, latest)) = snapshots
            .iter()
            .max_by_key(|(_, snapshot)| &snapshot.timestamp)
        {
            return Ok(ArchivingResult {
                existing_snapshot: true,
                last_archived: parse_wayback_timestamp(&latest.timestamp)?,
                url: Some(latest.url.clone()),
            });
        }
    }
    Err(ArchiveError::NoExistingSnapshot)
}

fn parse_wayback_timestamp(ts: &str) -> Result<NaiveDateTime, ArchiveError> {
    let naive_utc = NaiveDateTime::parse_from_str(ts, "%Y%m%d%H%M%S")
        .map_err(|err| ArchiveError::ParseError(err.to_string()))?;
    Ok(Utc.from_utc_datetime(&naive_utc).naive_local())
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

#[derive(Debug, PartialEq)]
pub enum ArchiveError {
    BandwidthExceeded,
    UnableToArchive,
    NoExistingSnapshot,
    ParseError(String),
    Unknown(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::BandwidthExceeded => write!(f, "Bandwidth Exceeded"),
            ArchiveError::UnableToArchive => {
                write!(f, "Wayback Machine unable to archive this URL")
            }
            ArchiveError::NoExistingSnapshot => write!(f, "No existing snapshots"),
            ArchiveError::ParseError(err) => write!(f, "Parse error: {}", err),
            ArchiveError::Unknown(err) => write!(f, "Unknown error: {}", err),
        }
    }
}

impl std::error::Error for ArchiveError {}
