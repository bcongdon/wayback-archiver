use chrono::{Duration, Utc};
use clap::{AppSettings, Clap};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};
use std::thread;

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
    urls_file: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts = Opts::parse();

    let (tx, rx) = crossbeam_channel::unbounded::<String>();

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

    let total_lines_count = Arc::new(Mutex::new(0));
    let total_lines_count_clone = total_lines_count.clone();

    // Spawn a separate thread to pull from the lines source.
    let urls_file = opts.urls_file;
    thread::spawn(move ||
        // This could probably be refactored...
        match urls_file {
        // Read URLs from a file.
        Some(path) => {
            // TODO: Propagate error better here.
            let file = fs::File::open(path).expect("unable to open file");
            for line in std::io::BufReader::new(file).lines() {
                tx.send(line.expect("line")).expect("send");
                *total_lines_count.lock().unwrap() += 1;
            }
        }
        // Fall back on stdin.
        None => {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                tx.send(line.expect("line")).expect("send");
                *total_lines_count.lock().unwrap() += 1;
            }
        }
    });

    for (line_idx, line) in rx.clone().into_iter().enumerate() {
        let pb = ProgressBar::new_spinner();
        pb.enable_steady_tick(120);
        pb.set_style(
            ProgressStyle::default_spinner().template("{prefix:.bold.dim} {spinner:.blue} {msg}"),
        );
        pb.set_prefix(format!(
            "[{}/{}]",
            line_idx + 1,
            *total_lines_count_clone.lock().unwrap()
        ));

        if let Some(existing) = urls.get(&line) {
            // If the last archival time of the URL was within ~6 months, accept it and move on.
            if (Utc::now().naive_utc() - existing.last_archived) < Duration::days(30 * 6) {
                pb.finish_with_message(format!("URL already archived: {}", line));
                continue;
            }
        }

        pb.set_message(format!("Archiving {}...", line));
        loop {
            let result = match archive_url(&line).await {
                Ok(success) => {
                    if !success.existing_snapshot {
                        pb.set_message("Cooldown after archiving...");
                        std::thread::sleep(Duration::seconds(5).to_std().expect("sleep duration"));
                    }
                    pb.finish_with_message(format!(
                        "Done: {}",
                        &success.url.as_ref().expect("archive url")
                    ));
                    success
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
                        existing_snapshot: false,
                    }
                }
            };
            urls.insert(line.clone(), result);
            break;
        }

        if line_idx != 0 && line_idx % 100 == 0 {
            if let Some(out_path) = &opts.out {
                eprintln!("Writing intermediate results...");
                write_results(&urls, out_path)?;
            }
        }
    }

    match opts.out {
        Some(path) => write_results(&urls, &path)?,
        None => {
            println!("{}", serde_json::to_string_pretty(&urls)?);
        }
    }
    Ok(())
}

fn write_results(
    results: &BTreeMap<String, ArchivingResult>,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let formatted_urls = serde_json::to_string_pretty(&results)?;
    let mut file = fs::OpenOptions::new().write(true).create(true).open(path)?;
    file.write_all(formatted_urls.as_bytes())?;
    Ok(())
}
