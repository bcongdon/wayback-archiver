use chrono::{Duration, NaiveDateTime, Utc};
use clap::{AppSettings, Clap};
use serde::{Deserialize, Serialize};
use spinners::{Spinner, Spinners};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, BufRead, Write};

#[derive(Clap)]
#[clap(version = "1.0", author = "Ben Congdon <ben@congdon.dev>")]
#[clap(setting = AppSettings::ColoredHelp)]
struct Opts {
    #[clap(short, long)]
    out: Option<String>,
    #[clap(short, long)]
    merge: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts = Opts::parse();

    let mut urls: BTreeMap<String, ArchivingResult> = BTreeMap::new();
    if opts.merge {
        let path = opts.out.as_ref().expect("--merge requires --out to be set");
        match fs::read_to_string(path) {
            Ok(existing) => urls = serde_json::from_str(&existing)?,
            Err(error) => match error.kind() {
                // Ignore "file not found" error.
                io::ErrorKind::NotFound => {}
                _ => return Err(error.into()),
            },
        }
    }

    let stdin = io::stdin();
    for (line_idx, line) in stdin.lock().lines().enumerate() {
        let line = line?;
        eprintln!("[{}/?] Archiving {:#?}...", line_idx + 1, line);
        if let Some(existing) = urls.get(&line) {
            // If the last archival time of the URL was within ~6 months, accept it and move on.
            if (Utc::now().naive_utc() - existing.last_archived) < Duration::days(30 * 6) {
                eprintln!("  -> URL already archived");
                continue;
            }
        }

        loop {
            let sp = Spinner::new(&Spinners::Line, "foo".into());
            let result = match archive_url(&line).await {
                Ok(out) => {
                    eprintln!("  -> Done: {}", out);
                    ArchivingResult {
                        last_archived: Utc::now().naive_local(),
                        url: Some(out.clone()),
                    }
                }
                Err(err) => {
                    eprintln!("  -> Archiving failed: {}", err);
                    if let Some(ArchiveError::BandwidthExceeded) =
                        err.downcast_ref::<ArchiveError>()
                    {
                        eprintln!("  -> Bandwidth exceeded. Waiting...");
                        std::thread::sleep(Duration::seconds(15).to_std().expect("sleep duration"));
                        continue;
                    }
                    ArchivingResult {
                        last_archived: Utc::now().naive_local(),
                        url: None,
                    }
                }
            };
            urls.insert(line.clone(), result);
            sp.stop();
            break;
        }
        std::thread::sleep(Duration::seconds(5).to_std().expect("sleep duration"));
    }

    let formatted_urls = serde_json::to_string_pretty(&urls)?;
    match opts.out {
        Some(path) => {
            let mut file = fs::OpenOptions::new().write(true).create(true).open(path)?;
            file.write_all(formatted_urls.as_bytes())?;
        }
        None => {
            println!("{}", formatted_urls);
        }
    }

    Ok(())
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
struct ArchivingResult {
    url: Option<String>,
    last_archived: NaiveDateTime,
}

#[derive(Debug)]
enum ArchiveError {
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

// TODO: Figure out how to refactor this to return `Result<String, ArchiveError>`.
async fn archive_url(url: &str) -> Result<String, Box<dyn std::error::Error>> {
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
