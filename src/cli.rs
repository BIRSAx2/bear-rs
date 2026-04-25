use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "bear",
    about = "Command line tool for reading and writing Bear notes.",
    after_help = "\
Exit codes: 0 success, 1 business error, 64 usage error.

Selection:
  Most commands accept a positional <id> (ZUNIQUEIDENTIFIER) or --title.
  --title does a case-insensitive exact match; most-recently-modified wins
  when multiple notes share the title.

Examples:
  bear list
  bear show --title \"Scratch\"
  bear create \"Quick Note\" --content \"Body\" --tags work
  bear append --title \"Scratch\" --content \"New paragraph\"
  bear search \"@today @todo\"
  bear tags list
  bear mcp-server
"
)]
pub struct Cli {
    /// Increase diagnostic output (-v, -vv, -vvv).
    #[arg(short = 'v', long = "verbose", global = true, action = ArgAction::Count)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Commands,
}

// ── Shared note-selector args ──────────────────────────────────────────────────

#[derive(Args, Debug, Default)]
pub struct NoteSelector {
    /// Note ZUNIQUEIDENTIFIER.
    #[arg(index = 1)]
    pub id: Option<String>,
    /// Identify note by title (case-insensitive).
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,
}

// ── Output args (shared by list/show/search) ──────────────────────────────────

#[derive(Args, Debug)]
pub struct OutputArgs {
    /// Comma-separated field names. Use "all" for all fields, "all,content" to include body.
    #[arg(long, value_name = "FIELDS")]
    pub fields: Option<String>,
    /// Output format: text (default) or json.
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub format: String,
}

// ── Top-level commands ────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List notes.
    #[command(after_help = "\
Default fields: id, title, tags
All fields: id, title, tags, hash, length, created, modified, pins,
            location, todos, done, attachments, content
Content is excluded from \"all\". Use --fields all,content to include it.

Examples:
  bear list
  bear list --tag work
  bear list --tag work --sort modified:asc --fields id,title,modified
  bear list -n 20 --format json --fields all
  bear list --count
")]
    List(ListArgs),

    /// Print raw note content.
    #[command(after_help = "\
Examples:
  bear cat <id>
  bear cat --title \"Mars\"
  bear cat <id> --offset 0 --limit 500
")]
    Cat(CatArgs),

    /// Show note metadata.
    #[command(after_help = "\
Examples:
  bear show <id>
  bear show --title \"Mars\" --format json --fields all
  bear show <id> --fields all,content
")]
    Show(ShowArgs),

    /// Search Bear notes.
    #[command(after_help = "\
Bear search syntax: text, \"phrases\", -negation, #tag, @today, @yesterday,
  @lastXdays, @date(YYYY-MM-DD), @todo, @done, @tagged, @untagged, @pinned,
  @images, @files, @code, @locked, @title, @untitled, @empty.
Full reference: https://bear.app/faq/how-to-search-notes-in-bear/

Examples:
  bear search \"meeting notes\"
  bear search \"@today @todo\" --format json
  bear search --query \"- [ ]\" --fields id,title,matches
  bear search \"@todo\" --count
")]
    Search(SearchArgs),

    /// Search for a string within a single note.
    #[command(after_help = "\
Examples:
  bear search-in <id> --string \"TODO\"
  bear search-in --title \"Mars\" --string \"water\" --format json
  bear search-in <id> --string \"TODO\" --count
")]
    SearchIn(SearchInArgs),

    /// Create a new note.
    #[command(after_help = "\
Examples:
  bear create \"My Note\" --content \"Body text\"
  bear create --content \"# Quick Capture\\nSome thoughts\"
  bear create \"My Note\" --tags \"work,draft\" --format json
  printf \"line1\\nline2\" | bear create \"My Note\" --fields id,hash
")]
    Create(CreateArgs),

    /// Add content to an existing note.
    #[command(after_help = "\
Examples:
  bear append <id> --content \"New paragraph\"
  printf \"New content\" | bear append <id>
  bear append --title \"Mars\" --content \"Update\" --position beginning
")]
    Append(AppendArgs),

    /// Replace the entire content of a note.
    #[command(after_help = "\
Examples:
  bear write <id> --base abc1234 --content \"# Title\\nBody\"
  printf \"# Title\\nBody\" | bear write <id> --base abc1234
  bear write <id> --content \"# Title\\nBody\"
")]
    Write(WriteArgs),

    /// Find and replace text within a note.
    #[command(after_help = "\
Examples:
  bear edit <id> --at \"TODO\" --replace \"DONE\"
  bear edit <id> --at \"## Notes\" --insert \"\\nNew line\"
  bear edit <id> --at \"cat\" --replace \"dog\" --all --word
")]
    Edit(EditArgs),

    /// Open a note in Bear.app.
    #[command(after_help = "\
Examples:
  bear open <id>
  bear open --title \"Mars\" --header \"Moons\" --edit
  bear open <id> --new-window
")]
    Open(OpenArgs),

    /// Move a note to trash.
    Trash(NoteSelector),

    /// Archive a note.
    Archive(NoteSelector),

    /// Restore a note from trash or archive.
    Restore(NoteSelector),

    /// Manage tags on notes.
    Tags(TagsCommand),

    /// Manage pins on notes.
    Pin(PinCommand),

    /// Manage attachments on a note.
    Attachments(AttachmentsCommand),

    /// Start an MCP server over stdio.
    #[command(after_help = "\
Configure an MCP-aware client (Claude Desktop, claude.ai, an IDE) to launch
this binary with the mcp-server argument. The server speaks JSON-RPC 2.0 with
line-delimited JSON framing on stdin/stdout.
")]
    McpServer,
}

// ── list ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Filter to notes carrying this tag (matches subtags too).
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,
    /// Sort: comma-separated field:dir pairs. Valid fields: pinned, modified, created, title.
    #[arg(
        long,
        value_name = "FIELD:DIR",
        default_value = "pinned:desc,modified:desc"
    )]
    pub sort: String,
    /// Maximum number of notes to return.
    #[arg(short = 'n', long = "limit", value_name = "N")]
    pub limit: Option<usize>,
    /// Print only the total note count.
    #[arg(long)]
    pub count: bool,
    #[command(flatten)]
    pub output: OutputArgs,
}

// ── cat ───────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct CatArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Start byte offset.
    #[arg(long, value_name = "N")]
    pub offset: Option<usize>,
    /// Maximum number of bytes to return.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

// ── show ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ShowArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    #[command(flatten)]
    pub output: OutputArgs,
}

// ── search ────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Bear search query. Pass via positional arg or --query.
    #[arg(index = 1, value_name = "QUERY")]
    pub query: Option<String>,
    /// Alternative flag form of the query.
    #[arg(long, value_name = "QUERY", conflicts_with = "query")]
    pub query_flag: Option<String>,
    /// Maximum number of results.
    #[arg(short = 'n', long = "limit", value_name = "N")]
    pub limit: Option<usize>,
    /// Print only the total match count.
    #[arg(long)]
    pub count: bool,
    #[command(flatten)]
    pub output: OutputArgs,
}

impl SearchArgs {
    pub fn effective_query(&self) -> &str {
        self.query
            .as_deref()
            .or(self.query_flag.as_deref())
            .unwrap_or("")
    }
}

// ── search-in ─────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct SearchInArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// String to search for within the note.
    #[arg(long, value_name = "STRING", required = true)]
    pub string: String,
    /// Print only the match count.
    #[arg(long)]
    pub count: bool,
    /// Output format.
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub format: String,
}

// ── create ────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Optional note title (Bear auto-generates the # heading from it).
    #[arg(index = 1, value_name = "TITLE")]
    pub title: Option<String>,
    /// Note body. Reads from stdin when omitted.
    #[arg(long, value_name = "TEXT")]
    pub content: Option<String>,
    /// Comma-separated tags.
    #[arg(long, value_name = "TAGS")]
    pub tags: Option<String>,
    /// Return existing note if one with the same title already exists.
    #[arg(long)]
    pub if_not_exists: bool,
    #[command(flatten)]
    pub output: OutputArgs,
}

// ── append ────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct AppendArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Content to append. Reads from stdin when omitted.
    #[arg(long, value_name = "TEXT")]
    pub content: Option<String>,
    /// Where to insert: beginning or end (default: end).
    #[arg(long, value_name = "POSITION", default_value = "end")]
    pub position: String,
    /// Preserve the note's modification date.
    #[arg(long = "no-update-modified")]
    pub no_update_modified: bool,
}

// ── write ─────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct WriteArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Replacement content. Reads from stdin when omitted.
    #[arg(long, value_name = "TEXT")]
    pub content: Option<String>,
    /// Reject write if the note's current hash does not match this value.
    #[arg(long, value_name = "HASH")]
    pub base: Option<String>,
}

// ── edit ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct EditArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Exact text to find. Repeat for batch edits.
    #[arg(long, value_name = "TEXT", required = true, action = ArgAction::Append)]
    pub at: Vec<String>,
    /// Replacement text. Repeat for batch edits.
    #[arg(long, value_name = "TEXT", action = ArgAction::Append)]
    pub replace: Vec<String>,
    /// Text to insert after the match. Repeat for batch edits.
    #[arg(long, value_name = "TEXT", action = ArgAction::Append, conflicts_with = "replace")]
    pub insert: Vec<String>,
    /// Apply to all occurrences.
    #[arg(long)]
    pub all: bool,
    /// Case-insensitive matching.
    #[arg(long = "ignore-case")]
    pub ignore_case: bool,
    /// Match whole words only.
    #[arg(long)]
    pub word: bool,
}

// ── open ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct OpenArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Scroll to this heading.
    #[arg(long, value_name = "HEADING")]
    pub header: Option<String>,
    /// Drop the cursor into the editor.
    #[arg(long)]
    pub edit: bool,
    /// Open in a new window.
    #[arg(long = "new-window")]
    pub new_window: bool,
    /// Open in a floating panel that stays on top.
    #[arg(long)]
    pub float: bool,
}

// ── tags ──────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct TagsCommand {
    #[command(subcommand)]
    pub subcommand: TagsSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum TagsSubcommand {
    /// List tags (all or for a specific note).
    #[command(after_help = "\
Examples:
  bear tags list
  bear tags list <id>
  bear tags list --title \"Mars\" --format json
  bear tags list --count
")]
    List(TagsListArgs),

    /// Add tags to a note.
    #[command(after_help = "\
Examples:
  bear tags add <id> work \"work/meetings\"
  bear tags add --title \"Mars\" favorite
")]
    Add(TagsAddArgs),

    /// Remove tags from a note.
    #[command(after_help = "\
Examples:
  bear tags remove <id> draft wip
  bear tags remove --title \"Mars\" draft
")]
    Remove(TagsRemoveArgs),

    /// Rename a tag across all notes.
    #[command(after_help = "\
Examples:
  bear tags rename work job
  bear tags rename --from draft --to published
  bear tags rename old-tag existing-tag --force
")]
    Rename(TagsRenameArgs),

    /// Delete a tag and remove it from all notes.
    #[command(after_help = "\
Examples:
  bear tags delete draft
  bear tags delete --name \"work/old\"
")]
    Delete(TagsDeleteArgs),
}

#[derive(Args, Debug)]
pub struct TagsListArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Print only the tag count.
    #[arg(long)]
    pub count: bool,
    /// Output format: text or json.
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct TagsAddArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Tags to add (positional, one or more).
    #[arg(required = true)]
    pub tags: Vec<String>,
}

#[derive(Args, Debug)]
pub struct TagsRemoveArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Tags to remove (positional, one or more).
    #[arg(required = true)]
    pub tags: Vec<String>,
}

#[derive(Args, Debug)]
pub struct TagsRenameArgs {
    /// Old tag name (positional or --from).
    #[arg(index = 1, value_name = "OLD")]
    pub old: Option<String>,
    /// New tag name (positional or --to).
    #[arg(index = 2, value_name = "NEW")]
    pub new: Option<String>,
    /// Old tag name (flag form).
    #[arg(long = "from", value_name = "TAG", conflicts_with = "old")]
    pub from: Option<String>,
    /// New tag name (flag form).
    #[arg(long = "to", value_name = "TAG", conflicts_with = "new")]
    pub to: Option<String>,
    /// Proceed even if the new name already exists (merge).
    #[arg(long)]
    pub force: bool,
}

impl TagsRenameArgs {
    pub fn old_name(&self) -> Option<&str> {
        self.old.as_deref().or(self.from.as_deref())
    }
    pub fn new_name(&self) -> Option<&str> {
        self.new.as_deref().or(self.to.as_deref())
    }
}

#[derive(Args, Debug)]
pub struct TagsDeleteArgs {
    /// Tag to delete (positional or --name).
    #[arg(index = 1, value_name = "TAG")]
    pub tag: Option<String>,
    /// Tag to delete (flag form).
    #[arg(long, value_name = "TAG", conflicts_with = "tag")]
    pub name: Option<String>,
}

impl TagsDeleteArgs {
    pub fn tag_name(&self) -> Option<&str> {
        self.tag.as_deref().or(self.name.as_deref())
    }
}

// ── pin ───────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct PinCommand {
    #[command(subcommand)]
    pub subcommand: PinSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum PinSubcommand {
    /// List pin contexts.
    #[command(after_help = "\
Examples:
  bear pin list                   # every pin context in use
  bear pin list <id>              # pins on a single note
  bear pin list --title \"Mars\" --format json
")]
    List(PinListArgs),

    /// Add pins to a note.
    #[command(after_help = "\
Examples:
  bear pin add <id> global
  bear pin add <id> work projects
  bear pin add --title \"Mars\" global work
")]
    Add(PinAddArgs),

    /// Remove pins from a note.
    #[command(after_help = "\
Examples:
  bear pin remove <id> global
  bear pin remove <id> work
  bear pin remove --title \"Mars\" global work
")]
    Remove(PinRemoveArgs),
}

#[derive(Args, Debug)]
pub struct PinListArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Output format: text or json.
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct PinAddArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Pin contexts: "global" or tag names. One or more required.
    #[arg(required = true)]
    pub contexts: Vec<String>,
}

#[derive(Args, Debug)]
pub struct PinRemoveArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Pin contexts to remove. One or more required.
    #[arg(required = true)]
    pub contexts: Vec<String>,
}

// ── attachments ───────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct AttachmentsCommand {
    #[command(subcommand)]
    pub subcommand: AttachmentsSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum AttachmentsSubcommand {
    /// List attachments on a note.
    #[command(after_help = "\
Examples:
  bear attachments list <id>
  bear attachments list --title \"Mars\" --format json
")]
    List(AttachmentsListArgs),

    /// Write attachment bytes to stdout.
    #[command(after_help = "\
Examples:
  bear attachments save <id> --filename photo.jpg > photo.jpg
")]
    Save(AttachmentsSaveArgs),

    /// Add an attachment to a note (reads from stdin).
    #[command(after_help = "\
Examples:
  cat photo.jpg | bear attachments add <id> --filename photo.jpg
  bear attachments add <id> --filename photo.jpg < photo.jpg
")]
    Add(AttachmentsAddArgs),

    /// Delete an attachment from a note.
    #[command(after_help = "\
Examples:
  bear attachments delete <id> --filename photo.jpg
")]
    Delete(AttachmentsDeleteArgs),
}

#[derive(Args, Debug)]
pub struct AttachmentsListArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Output format: text or json.
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct AttachmentsSaveArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Filename of the attachment to save.
    #[arg(long, required = true, value_name = "FILENAME")]
    pub filename: String,
}

#[derive(Args, Debug)]
pub struct AttachmentsAddArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Filename for the attachment.
    #[arg(long, required = true, value_name = "FILENAME")]
    pub filename: String,
}

#[derive(Args, Debug)]
pub struct AttachmentsDeleteArgs {
    #[command(flatten)]
    pub selector: NoteSelector,
    /// Filename of the attachment to delete.
    #[arg(long, required = true, value_name = "FILENAME")]
    pub filename: String,
}
