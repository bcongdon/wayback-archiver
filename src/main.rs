use chrono::{Duration, Utc};
use clap::{AppSettings, Clap};
use spinners::{Spinner, Spinners};
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
