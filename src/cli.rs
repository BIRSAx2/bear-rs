use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "bear")]
#[command(about = "Rust CLI for Bear.app on macOS", version)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        env = "BEAR_DATABASE",
        help = "Path to Bear's macOS SQLite database. If omitted, bear-cli discovers it dynamically."
    )]
    pub database: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Auth(AuthCommand),
    OpenNote(OpenNoteCommand),
    Tags,
    OpenTag(OpenTagCommand),
    Search(SearchCommand),
    Duplicates(DuplicatesCommand),
    Stats(StatsCommand),
    Health(HealthCommand),
    Untagged(FilterCommand),
    Todo(FilterCommand),
    Today(FilterCommand),
    Locked(FilterCommand),
    Create(CreateCommand),
    AddText(AddTextCommand),
    AddFile(AddFileCommand),
    GrabUrl(GrabUrlCommand),
    Trash(IdOrSearchCommand),
    Archive(IdOrSearchCommand),
    RenameTag(RenameTagCommand),
    DeleteTag(DeleteTagCommand),
    Raw(RawCommand),
}

#[derive(Args, Debug)]
pub struct AuthCommand {
    pub token: String,
}

#[derive(Args, Debug)]
pub struct OpenNoteCommand {
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long, default_value_t = false)]
    pub exclude_trashed: bool,
}

#[derive(Args, Debug)]
pub struct OpenTagCommand {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct SearchCommand {
    pub term: Option<String>,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub since: Option<String>,
    #[arg(long)]
    pub before: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DuplicatesCommand {
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct StatsCommand {
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct HealthCommand {
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct FilterCommand {
    pub search: Option<String>,
}

#[derive(Args, Debug)]
pub struct CreateCommand {
    pub text: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub file: Option<PathBuf>,
    #[arg(long)]
    pub filename: Option<String>,
    #[arg(short = 't', long = "tag")]
    pub tag: Vec<String>,
    #[arg(long, default_value_t = false)]
    pub open_note: bool,
    #[arg(long, default_value_t = false)]
    pub new_window: bool,
    #[arg(long, default_value_t = false)]
    pub float: bool,
    #[arg(long, default_value_t = false)]
    pub show_window: bool,
    #[arg(long, default_value_t = false)]
    pub pin: bool,
    #[arg(long, default_value_t = false)]
    pub edit: bool,
    #[arg(long, default_value_t = false)]
    pub timestamp: bool,
    #[arg(long = "type")]
    pub kind: Option<String>,
    #[arg(long)]
    pub url: Option<String>,
}

#[derive(Args, Debug)]
pub struct AddTextCommand {
    pub text: Option<String>,
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub header: Option<String>,
    #[arg(long, default_value = "append")]
    pub mode: String,
    #[arg(short = 't', long = "tag")]
    pub tag: Vec<String>,
    #[arg(long, default_value_t = false)]
    pub exclude_trashed: bool,
    #[arg(long, default_value_t = false)]
    pub new_line: bool,
    #[arg(long, default_value_t = false)]
    pub open_note: bool,
    #[arg(long, default_value_t = false)]
    pub new_window: bool,
    #[arg(long, default_value_t = false)]
    pub show_window: bool,
    #[arg(long, default_value_t = false)]
    pub edit: bool,
    #[arg(long, default_value_t = false)]
    pub timestamp: bool,
}

#[derive(Args, Debug)]
pub struct AddFileCommand {
    pub file: PathBuf,
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub header: Option<String>,
    #[arg(long)]
    pub filename: Option<String>,
    #[arg(long, default_value = "append")]
    pub mode: String,
    #[arg(long, default_value_t = false)]
    pub open_note: bool,
    #[arg(long, default_value_t = false)]
    pub new_window: bool,
    #[arg(long, default_value_t = false)]
    pub show_window: bool,
    #[arg(long, default_value_t = false)]
    pub edit: bool,
}

#[derive(Args, Debug)]
pub struct GrabUrlCommand {
    pub url: String,
    #[arg(short = 't', long = "tag")]
    pub tag: Vec<String>,
    #[arg(long, default_value_t = false)]
    pub pin: bool,
    #[arg(long, default_value_t = false)]
    pub wait: bool,
}

#[derive(Args, Debug)]
pub struct IdOrSearchCommand {
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub search: Option<String>,
    #[arg(long, default_value_t = false)]
    pub show_window: bool,
}

#[derive(Args, Debug)]
pub struct RenameTagCommand {
    pub name: String,
    pub new_name: String,
    #[arg(long, default_value_t = false)]
    pub show_window: bool,
}

#[derive(Args, Debug)]
pub struct DeleteTagCommand {
    pub name: String,
    #[arg(long, default_value_t = false)]
    pub show_window: bool,
}

#[derive(Args, Debug)]
pub struct RawCommand {
    pub action: String,
    #[arg(long)]
    pub token: Option<String>,
    #[arg(long, default_value_t = false)]
    pub use_saved_token: bool,
    #[arg(value_parser = parse_key_value)]
    pub params: Vec<(String, String)>,
}

fn parse_key_value(input: &str) -> Result<(String, String), String> {
    let (key, value) = input
        .split_once('=')
        .ok_or_else(|| "expected KEY=VALUE".to_string())?;
    Ok((key.to_string(), value.to_string()))
}
