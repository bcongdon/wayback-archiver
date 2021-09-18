# wayback-archiver

![crates.io](https://img.shields.io/crates/v/wayback-archiver.svg)

CLI archival tool for the Wayback Machine

## Installation

    $ cargo install wayback-archiver

### Examples:

```sh
$ wayback-archiver google.com

$ wayback-archiver --urls-file urls.txt --out archive.json

$ echo "google.com\nwikipedia.org\ngithub.com" | wayback-archiver --out=archive.json --merge
```

## Usage

```
USAGE:
    wayback-archiver [FLAGS] [OPTIONS] [URLS]...

ARGS:
    <URLS>...    URLs to archive using the Wayback Machine. URLs can also be provided using
                 stdin, or with --urls_file

FLAGS:
    -h, --help       Print help information
    -m, --merge      If set, the results are merged with the (existing) contents of the --out file
    -V, --version    Print version information

OPTIONS:
    -o, --out <OUT>                If set, archived URLs are saved to the path specified by this
                                   flag. Otherwise, URLs are printed at the end of the command run
    -u, --urls-file <URLS_FILE>    A file containing urls to archive
```

## Attribution

This tool only functions because of the Internet Archive. Please consider [dontating](https://archive.org/donate) to their cause.
