# bear-cli

`bear-cli` is a native Rust CLI for [Bear.app](https://bear.app/) on macOS.

It reads Bear's local SQLite database in read-only mode. It sends write and UI actions through Bear's `bear://x-callback-url/...` scheme.

## Usage

```sh
bear --help
bear open-note --title "My Note"
bear search crypto
bear duplicates
bear stats
bear health
bear create "hello" --title "CLI test" --tag rust
```

## Requirements

- macOS
- Bear.app installed
- local access to Bear's database under a Bear group container such as:

```text
~/Library/Group Containers/<TEAM_ID>.net.shinyfrog.bear/Application Data/database.sqlite
```

By default `bear-cli` discovers this dynamically. You only need `--database` or `BEAR_DATABASE` if you want to override it.

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

### Read commands

These commands read Bear's local SQLite database directly:

- `open-note`
- `tags`
- `open-tag`
- `search`
- `duplicates`
- `stats`
- `health`
- `untagged`
- `todo`
- `today`
- `locked`

Examples:

```sh
bear open-note --id 721FF116-185F-4474-8730-60D29995A4A4
bear open-note --title "Systems Security"
bear search Systems
bear duplicates --json
bear stats --json
bear health --json
bear open-tag work
bear tags
```

### Write and action commands

These commands ask Bear.app itself to perform the action through its URL scheme:

- `create`
- `add-text`
- `add-file`
- `grab-url`
- `trash`
- `archive`
- `rename-tag`
- `delete-tag`
- `raw`

Examples:

```sh
bear create "hello world" --title "Scratch"
bear add-text "append me" --title "Scratch"
bear add-file ./note.txt --title "Scratch"
bear grab-url https://example.com --tag inbox --wait
bear archive --id ABCD-1234
bear trash --search old
bear rename-tag inbox archive/inbox
bear delete-tag old-tag
```

## Authentication

Some Bear x-callback actions support an API token. Save it once with:

```sh
bear auth YOUR_BEAR_API_TOKEN
```

The saved token currently matters most for `raw`:

```sh
bear raw open-tag name=work --use-saved-token
bear raw tags --use-saved-token
bear raw open-note selected=yes --use-saved-token
```

You can also pass a token explicitly:

```sh
bear raw tags --token YOUR_BEAR_API_TOKEN
```

## Notes

- macOS only
- Bear must be installed for write/action commands
- write commands currently launch Bear successfully but do not capture x-callback return payloads back into the terminal
- read commands reflect Bear's local database state, not remote sync state
- encrypted and locked notes are intentionally filtered from several list/search commands

## Development

```sh
cargo fmt --all
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo package --allow-dirty
```
