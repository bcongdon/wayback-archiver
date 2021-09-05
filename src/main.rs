use chrono::{Duration, Utc};
use clap::{AppSettings, Clap};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};

mod lib;
use crate::lib::{archive_url, ArchiveError, ArchivingResult};

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

        let pb = ProgressBar::new_spinner();
        pb.enable_steady_tick(120);
        pb.set_style(
            ProgressStyle::default_spinner().template("{prefix:.bold.dim} {spinner:.blue} {msg}"),
        );
        pb.set_prefix(format!("[{}/?]", line_idx + 1));

        if let Some(existing) = urls.get(&line) {
            // If the last archival time of the URL was within ~6 months, accept it and move on.
            if (Utc::now().naive_utc() - existing.last_archived) < Duration::days(30 * 6) {
                pb.finish_with_message(format!("URL already archived: {}", line));
                continue;
            }
        }

        pb.set_message(format!("Waiting to archive {}...", line));
        std::thread::sleep(Duration::seconds(5).to_std().expect("sleep duration"));
        pb.set_message(format!("Archiving {}...", line));

        loop {
            let result = match archive_url(&line).await {
                Ok(out) => {
                    pb.finish_with_message(format!("Done: {}", out));
                    ArchivingResult {
                        last_archived: Utc::now().naive_local(),
                        url: Some(out.clone()),
                    }
                }
                Err(err) => {
                    if let Some(ArchiveError::BandwidthExceeded) =
                        err.downcast_ref::<ArchiveError>()
                    {
                        pb.set_message("Bandwidth exceeded. Waiting...");
                        std::thread::sleep(Duration::seconds(15).to_std().expect("sleep duration"));
                        continue;
                    }
                    pb.finish_with_message(format!("Archiving failed: {}", err));
                    ArchivingResult {
                        last_archived: Utc::now().naive_local(),
                        url: None,
                    }
                }
            };
            urls.insert(line.clone(), result);
            break;
        }
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
