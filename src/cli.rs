use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

const ROOT_AFTER_HELP: &str = "\
Selection rules:
  Most note-targeting commands accept either --id or a title/search selector.
  --id is exact and preferred for automation.

Output conventions:
  Human-readable commands usually print tab-separated rows.
  Commands with --json emit structured JSON for agent consumption.

Examples:
  bear -v notes --limit 20
  bear -vv inspect-note --title \"Scratch\"
  bear -vvv auth
  bear notes --limit 20 --json
  bear open-note --title \"Scratch\"
  bear add-text --title \"Scratch\" \"more text\"
";

#[derive(Parser, Debug)]
#[command(name = "bear")]
#[command(about = "CloudKit CLI for Bear notes on macOS", version, after_help = ROOT_AFTER_HELP)]
pub struct Cli {
    /// Increase diagnostic output. Use `-v` for request flow, `-vv` for request/response payloads, `-vvv` for auth callback details.
    #[arg(short = 'v', long = "verbose", global = true, action = ArgAction::Count)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with Bear's CloudKit web flow.
    Auth(AuthCommand),
    /// Print the full text of a single note.
    OpenNote(OpenNoteCommand),
    /// Print the raw CloudKit record for a single note.
    InspectNote(OpenNoteCommand),
    /// List all tags visible in CloudKit.
    Tags,
    /// List notes that belong to one or more tags.
    OpenTag(OpenTagCommand),
    /// Search notes by title, body, and tags.
    Search(SearchCommand),
    /// List notes with lightweight metadata.
    Notes(CloudNotesCommand),
    /// List or delete orphaned notes written to CloudKit's default zone.
    PhantomNotes(PhantomNotesCommand),
    /// Export notes to Markdown files.
    Export(ExportCommand),
    /// Find duplicate note titles.
    Duplicates(DuplicatesCommand),
    /// Print note-library statistics.
    Stats(StatsCommand),
    /// Report empty, large, duplicate, and conflict-looking notes.
    Health(HealthCommand),
    /// List notes that currently have no tags.
    Untagged(FilterCommand),
    /// List notes that contain unchecked Markdown todos.
    Todo(FilterCommand),
    /// List notes modified since local midnight.
    Today(FilterCommand),
    /// List locked or encrypted notes.
    Locked(FilterCommand),
    /// Create a new note.
    Create(CreateCommand),
    /// Insert or replace text in an existing note.
    AddText(AddTextCommand),
    /// Attach a file to an existing note.
    AddFile(AddFileCommand),
    /// Move a note to trash.
    Trash(IdOrSearchCommand),
    /// Permanently delete a note record from CloudKit.
    Delete(IdOrSearchCommand),
    /// Archive a note.
    Archive(IdOrSearchCommand),
    /// Rename a tag.
    RenameTag(RenameTagCommand),
    /// Delete a tag.
    DeleteTag(DeleteTagCommand),
}

#[derive(Args, Debug)]
#[command(after_help = "Examples:\n  bear auth\n  bear auth --token '<CK_WEB_AUTH_TOKEN>'\n")]
pub struct AuthCommand {
    /// Save this `ckWebAuthToken` directly instead of opening the browser auth flow.
    #[arg(long, value_name = "CK_WEB_AUTH_TOKEN")]
    pub token: Option<String>,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Exactly one of --id or --title should be provided.\nExamples:\n  bear open-note --id NOTE_RECORD_NAME\n  bear open-note --title 'Scratch'\n"
)]
pub struct OpenNoteCommand {
    /// Exact CloudKit record name for the note.
    #[arg(long)]
    pub id: Option<String>,
    /// Exact note title. If multiple notes share the title, the most recently modified match is used.
    #[arg(long)]
    pub title: Option<String>,
    /// Exclude trashed notes when resolving --title.
    #[arg(long, default_value_t = false)]
    pub exclude_trashed: bool,
}

#[derive(Args, Debug)]
#[command(after_help = "Examples:\n  bear open-tag work\n  bear open-tag work,project\n")]
pub struct OpenTagCommand {
    /// Tag name or comma-separated tag names. A note is listed if it matches any provided tag.
    pub name: String,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Examples:\n  bear search rust\n  bear search meeting --tag work --json\n  bear search '' --since 2026-04-01 --before 2026-04-17\n"
)]
pub struct SearchCommand {
    /// Search term. Matches title, note body, and tag names. Leave empty to filter only by date or tag.
    pub term: Option<String>,
    /// Restrict results to notes containing this exact tag.
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,
    /// Only include notes modified on or after this date filter. Accepts YYYY-MM-DD, today, yesterday, last-week, last-month, last-year.
    #[arg(long, value_name = "DATE")]
    pub since: Option<String>,
    /// Only include notes modified before this date filter.
    #[arg(long, value_name = "DATE")]
    pub before: Option<String>,
    /// Emit machine-readable JSON.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Default output is tab-separated: RECORD_NAME<TAB>TITLE\nExamples:\n  bear notes\n  bear notes --limit 50 --json\n  bear notes --archived\n"
)]
pub struct CloudNotesCommand {
    /// Maximum number of notes to return after CloudKit pagination.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
    /// Include trashed notes in the result set.
    #[arg(long, default_value_t = false)]
    pub trashed: bool,
    /// Include archived notes in the result set.
    #[arg(long, default_value_t = false)]
    pub archived: bool,
    /// Emit machine-readable JSON instead of tab-separated text.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
#[command(
    after_help = "These records exist in CloudKit but are not part of Bear's normal Notes zone.\nExamples:\n  bear phantom-notes\n  bear phantom-notes --delete\n"
)]
pub struct PhantomNotesCommand {
    /// Maximum number of phantom notes to return after CloudKit pagination.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
    /// Hard-delete all listed phantom notes from CloudKit's default zone.
    #[arg(long, default_value_t = false)]
    pub delete: bool,
    /// Emit machine-readable JSON instead of tab-separated text.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
#[command(after_help = "Use --json for structured duplicate groups.\n")]
pub struct DuplicatesCommand {
    /// Emit machine-readable JSON.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Examples:\n  bear export ./notes\n  bear export ./notes --tag work --frontmatter --by-tag\n"
)]
pub struct ExportCommand {
    /// Target directory where Markdown files will be written.
    pub output: PathBuf,
    /// Export only notes that contain this exact tag.
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,
    /// Inject generated YAML frontmatter into exported Markdown.
    #[arg(long, default_value_t = false)]
    pub frontmatter: bool,
    /// Place each note under a directory named after its first tag.
    #[arg(long = "by-tag", default_value_t = false)]
    pub by_tag: bool,
}

#[derive(Args, Debug)]
#[command(after_help = "Use --json for structured metrics.\n")]
pub struct StatsCommand {
    /// Emit machine-readable JSON.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
#[command(after_help = "Use --json for structured issue lists.\n")]
pub struct HealthCommand {
    /// Emit machine-readable JSON.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Examples:\n  bear untagged\n  bear todo meeting\n  bear today standup\n  bear locked finance\n"
)]
pub struct FilterCommand {
    /// Optional free-text filter applied to title and body after the base command predicate.
    pub search: Option<String>,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Examples:\n  bear create '# Scratch'\n  printf '# Scratch\\n\\nBody' | bear create -t work -t inbox\n"
)]
pub struct CreateCommand {
    /// Note body (markdown). If omitted, reads from stdin.
    pub text: Option<String>,
    /// Tag names to assign during creation. Repeat for multiple tags.
    #[arg(long, short = 't', value_name = "TAG")]
    pub tag: Vec<String>,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Exactly one of --id or --title should be provided.\nExamples:\n  bear add-text --title 'Scratch' 'new line'\n  bear add-text --id NOTE_RECORD_NAME --mode replace-all '# Rewritten'\n  bear add-text --title 'Scratch' --header Tasks '- [ ] follow up'\n"
)]
pub struct AddTextCommand {
    /// Text to add. If omitted, reads from stdin.
    pub text: Option<String>,
    /// Exact CloudKit record name for the target note.
    #[arg(long)]
    pub id: Option<String>,
    /// Exact note title. If multiple notes share the title, the most recently modified match is used.
    #[arg(long)]
    pub title: Option<String>,
    /// How to apply the provided text.
    #[arg(long, default_value = "append")]
    pub mode: AddTextMode,
    /// Insert after the first Markdown heading line that starts with `## <HEADER>`.
    #[arg(long)]
    pub header: Option<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum AddTextMode {
    Append,
    Prepend,
    ReplaceAll,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Exactly one of --id or --title should be provided.\nExamples:\n  bear add-file ./report.pdf --title 'Scratch'\n  bear add-file ./image.png --id NOTE_RECORD_NAME --mode prepend\n"
)]
pub struct AddFileCommand {
    /// Path to the file to attach.
    pub file: PathBuf,
    /// Exact CloudKit record name for the target note.
    #[arg(long)]
    pub id: Option<String>,
    /// Exact note title. If multiple notes share the title, the most recently modified match is used.
    #[arg(long)]
    pub title: Option<String>,
    /// Override the filename used for the embedded attachment reference.
    #[arg(long)]
    pub filename: Option<String>,
    /// Where to place the attachment reference inside the note.
    #[arg(long, default_value = "append")]
    pub mode: AddFileMode,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum AddFileMode {
    Append,
    Prepend,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Use --id for exact targeting. Use --search to match by title, preferring the most recently modified note.\nExamples:\n  bear trash --id NOTE_RECORD_NAME\n  bear archive --search 'Scratch'\n"
)]
pub struct IdOrSearchCommand {
    /// Exact CloudKit record name for the target note.
    #[arg(long)]
    pub id: Option<String>,
    /// Exact note title used for lookup when --id is omitted.
    #[arg(long)]
    pub search: Option<String>,
}

#[derive(Args, Debug)]
#[command(after_help = "Example:\n  bear rename-tag inbox archive/inbox\n")]
pub struct RenameTagCommand {
    /// Existing tag name.
    pub name: String,
    /// Replacement tag name.
    pub new_name: String,
}

#[derive(Args, Debug)]
#[command(after_help = "Example:\n  bear delete-tag old-tag\n")]
pub struct DeleteTagCommand {
    /// Tag name to delete.
    pub name: String,
}
