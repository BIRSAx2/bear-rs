# bear-cli

`bear-cli` is a native Rust CLI for [Bear](https://bear.app) that talks directly to Bear's CloudKit container.

## Features

- authenticate with Bear's CloudKit web flow
- list, open, search, and export notes
- inspect tags and tag membership
- create notes, edit note text, attach files, trash, and archive
- rename and delete tags
- compute duplicates, stats, health checks, and other library summaries from CloudKit data

## Requirements

- macOS
- a Bear account with iCloud sync enabled
- network access to Apple's CloudKit endpoints

## Installation

### From crates.io

```sh
cargo install bear-cli
```

### From source

```sh
git clone https://github.com/BIRSAx2/bear-cli
cd bear-cli
cargo install --path .
```

The installed binary name is `bear`.

## Authentication

Authenticate once before using CloudKit-backed commands:

```sh
bear auth
```

The auth flow opens a localhost page and prefers Safari for CloudKit sign-in on macOS.

If you already have a valid `ckWebAuthToken`, you can save it directly:

```sh
bear auth --token '<CK_WEB_AUTH_TOKEN>'
```

On success the token is saved locally and reused by subsequent commands.

## Usage

```sh
bear --help
bear notes
bear open-note --title "My Note"
bear search crypto --json
bear create "# Scratch"
bear add-text --title "Scratch" "more text"
```

## Commands

### Reading notes and tags

- `notes`
- `open-note`
- `tags`
- `open-tag`
- `search`
- `export`
- `duplicates`
- `stats`
- `health`
- `untagged`
- `todo`
- `today`
- `locked`

Examples:

```sh
bear notes
bear notes --limit 50
bear notes --json
bear open-note --id 721FF116-185F-4474-8730-60D29995A4A4
bear open-note --title "Systems Security"
bear search Systems
bear search Systems --since 2026-04-01 --before 2026-04-17 --json
bear open-tag work
bear tags
bear export ./notes --frontmatter --by-tag
bear duplicates --json
bear stats --json
bear health --json
bear todo
bear today
```

### Writing notes and tags

- `create`
- `add-text`
- `add-file`
- `trash`
- `archive`
- `rename-tag`
- `delete-tag`

Examples:

```sh
bear create "# Scratch"
bear create "# Project note" -t work -t rust
bear add-text --title "Scratch" "append me"
bear add-text --title "Scratch" --mode prepend "top section"
bear add-text --title "Scratch" --mode replace-all "# Rewritten"
bear add-file ./note.txt --title "Scratch"
bear archive --search "old note"
bear trash --search "temporary"
bear rename-tag inbox archive/inbox
bear delete-tag old-tag
```

## Notes

- All operational commands are CloudKit-based.
- Authentication is required for both reads and writes.
- The CloudKit API token used by this project was reverse-engineered from Bear Web's public frontend bundle.
- The auth flow is browser-sensitive. Safari is the preferred path on macOS.
- Some large-note edge cases may still require additional CloudKit asset-handling work if Bear stores note bodies outside `textADP`.

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
