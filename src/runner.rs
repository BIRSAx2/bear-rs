use std::io::Read;

use anyhow::{Result, bail};
use clap::Parser;

use crate::cli::*;
use crate::model::{InsertPosition, SortDir, SortField};
use crate::output::{
    OutputFormat, default_list_fields, default_show_fields, parse_note_fields, print_attachments,
    print_note_count, print_notes, print_pins, print_tags,
};
use crate::store::{EditOp, ListInput, SqliteStore};
use crate::verbose;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    verbose::set(cli.verbose);

    match cli.command {
        Commands::List(cmd) => cmd_list(cmd),
        Commands::Cat(cmd) => cmd_cat(cmd),
        Commands::Show(cmd) => cmd_show(cmd),
        Commands::Search(cmd) => cmd_search(cmd),
        Commands::SearchIn(cmd) => cmd_search_in(cmd),
        Commands::Create(cmd) => cmd_create(cmd),
        Commands::Append(cmd) => cmd_append(cmd),
        Commands::Write(cmd) => cmd_write(cmd),
        Commands::Edit(cmd) => cmd_edit(cmd),
        Commands::Open(cmd) => cmd_open(cmd),
        Commands::Trash(sel) => cmd_trash(sel),
        Commands::Archive(sel) => cmd_archive(sel),
        Commands::Restore(sel) => cmd_restore(sel),
        Commands::Tags(cmd) => cmd_tags(cmd),
        Commands::Pin(cmd) => cmd_pin(cmd),
        Commands::Attachments(cmd) => cmd_attachments(cmd),
        Commands::McpServer => crate::mcp::run_server(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_stdin_or(arg: Option<String>) -> Result<String> {
    match arg {
        Some(s) => Ok(unescape(&s)),
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

/// Interpret \n \t \r \\ escape sequences in CLI argument strings.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('t') => {
                    chars.next();
                    out.push('\t');
                }
                Some('r') => {
                    chars.next();
                    out.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_format(s: &str) -> Result<OutputFormat> {
    s.parse::<OutputFormat>()
}

fn parse_sort(sort_spec: &str) -> Result<Vec<(SortField, SortDir)>> {
    let mut result = Vec::new();
    for part in sort_spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (field_str, dir_str) = if let Some((f, d)) = part.split_once(':') {
            (f, d)
        } else {
            (part, "desc")
        };
        let field = match field_str {
            "pinned" => SortField::Pinned,
            "modified" => SortField::Modified,
            "created" => SortField::Created,
            "title" => SortField::Title,
            other => bail!("invalid sort field: {other}"),
        };
        let dir = match dir_str {
            "asc" => SortDir::Asc,
            "desc" => SortDir::Desc,
            other => bail!("invalid sort order: {other}"),
        };
        result.push((field, dir));
    }
    Ok(result)
}

// ── list ──────────────────────────────────────────────────────────────────────

fn cmd_list(cmd: ListArgs) -> Result<()> {
    let store = SqliteStore::open_ro()?;
    let sort = parse_sort(&cmd.sort)?;
    let input = ListInput {
        tag: cmd.tag.as_deref(),
        sort,
        limit: cmd.limit,
        include_trashed: false,
        include_archived: false,
        include_tags: true,
    };
    let notes = store.list_notes(&input)?;

    if cmd.count {
        print_note_count(notes.len());
        return Ok(());
    }

    let fields = match &cmd.output.fields {
        Some(f) => parse_note_fields(f)?,
        None => default_list_fields(),
    };
    let format = parse_format(&cmd.output.format)?;
    print_notes(&notes, &fields, format);
    Ok(())
}

// ── cat ───────────────────────────────────────────────────────────────────────

fn cmd_cat(cmd: CatArgs) -> Result<()> {
    let store = SqliteStore::open_ro()?;
    let text = store.cat_note(
        cmd.selector.id.as_deref(),
        cmd.selector.title.as_deref(),
        cmd.offset,
        cmd.limit,
    )?;
    print!("{text}");
    Ok(())
}

// ── show ──────────────────────────────────────────────────────────────────────

fn cmd_show(cmd: ShowArgs) -> Result<()> {
    let fields = match &cmd.output.fields {
        Some(f) => parse_note_fields(f)?,
        None => default_show_fields(),
    };
    let needs_attachments = fields.contains(&crate::output::NoteField::Attachments);
    let needs_pins = fields.contains(&crate::output::NoteField::Pins);

    let store = SqliteStore::open_ro()?;
    let note = store.get_note(
        cmd.selector.id.as_deref(),
        cmd.selector.title.as_deref(),
        needs_attachments,
        needs_pins,
    )?;

    let format = parse_format(&cmd.output.format)?;
    print_notes(&[note], &fields, format);
    Ok(())
}

// ── search ────────────────────────────────────────────────────────────────────

fn cmd_search(cmd: SearchArgs) -> Result<()> {
    let query = cmd.effective_query();
    if query.starts_with('-') {
        bail!("query starts with \"-\"; wrap it in quotes if intentional: \"{query}\"");
    }

    let store = SqliteStore::open_ro()?;
    let notes = store.search_notes(query, cmd.limit)?;

    if cmd.count {
        print_note_count(notes.len());
        return Ok(());
    }

    let fields = match &cmd.output.fields {
        Some(f) => parse_note_fields(f)?,
        None => default_list_fields(),
    };
    let format = parse_format(&cmd.output.format)?;
    print_notes(&notes, &fields, format);
    Ok(())
}

// ── search-in ─────────────────────────────────────────────────────────────────

fn cmd_search_in(cmd: SearchInArgs) -> Result<()> {
    let store = SqliteStore::open_ro()?;
    let matches = store.search_in_note(
        cmd.selector.id.as_deref(),
        cmd.selector.title.as_deref(),
        &cmd.string,
        false,
    )?;

    if cmd.count {
        println!("{}", matches.len());
        return Ok(());
    }

    if matches.is_empty() {
        eprintln!("No notes found.");
        return Ok(());
    }

    let format = parse_format(&cmd.format)?;
    match format {
        OutputFormat::Text => {
            for (line_no, line) in &matches {
                println!("{line_no}\t{line}");
            }
        }
        OutputFormat::Json => {
            let arr: Vec<_> = matches
                .iter()
                .map(|(n, l)| serde_json::json!({"line": n, "text": l}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr)?);
        }
    }
    Ok(())
}

// ── create ────────────────────────────────────────────────────────────────────

fn cmd_create(cmd: CreateArgs) -> Result<()> {
    // Build the note text: if title given, construct "# Title\n\nBody"
    let body = read_stdin_or(cmd.content)?;

    let text = if let Some(title) = &cmd.title {
        // Prepend heading if body doesn't already start with it
        let heading = format!("# {title}");
        if body.trim_start().starts_with(&heading) {
            body
        } else {
            format!("{heading}\n\n{}", body.trim_start_matches('\n'))
        }
    } else {
        body
    };

    let tags: Vec<&str> = cmd
        .tags
        .as_deref()
        .map(|t| t.split(',').map(str::trim).collect())
        .unwrap_or_default();

    let store = SqliteStore::open_rw()?;
    let note = store.create_note(&text, &tags, cmd.if_not_exists)?;

    let fields = match &cmd.output.fields {
        Some(f) => parse_note_fields(f)?,
        None => vec![crate::output::NoteField::Id],
    };
    let format = parse_format(&cmd.output.format)?;
    print_notes(&[note], &fields, format);
    Ok(())
}

// ── append ────────────────────────────────────────────────────────────────────

fn cmd_append(cmd: AppendArgs) -> Result<()> {
    let content = read_stdin_or(cmd.content)?;
    let position = match cmd.position.as_str() {
        "beginning" => InsertPosition::Beginning,
        _ => InsertPosition::End,
    };

    let prefs = crate::prefs::load_prefs()?;
    let store = SqliteStore::open_rw()?;
    store.append_to_note(
        cmd.selector.id.as_deref(),
        cmd.selector.title.as_deref(),
        &content,
        position,
        !cmd.no_update_modified,
        prefs.tag_position,
    )?;
    Ok(())
}

// ── write ─────────────────────────────────────────────────────────────────────

fn cmd_write(cmd: WriteArgs) -> Result<()> {
    let content = read_stdin_or(cmd.content)?;
    let store = SqliteStore::open_rw()?;
    store.write_note(
        cmd.selector.id.as_deref(),
        cmd.selector.title.as_deref(),
        &content,
        cmd.base.as_deref(),
    )?;
    Ok(())
}

// ── edit ──────────────────────────────────────────────────────────────────────

fn cmd_edit(cmd: EditArgs) -> Result<()> {
    if cmd.replace.is_empty() && cmd.insert.is_empty() {
        bail!("--replace or --insert is required");
    }
    if !cmd.replace.is_empty() && !cmd.insert.is_empty() {
        bail!("--replace and --insert are mutually exclusive");
    }

    let ops: Vec<EditOp> = cmd
        .at
        .iter()
        .enumerate()
        .map(|(i, at)| {
            let replace = cmd.replace.get(i).cloned();
            let insert = cmd.insert.get(i).cloned();
            EditOp {
                at: unescape(at),
                replace: replace.map(|r| unescape(&r)),
                insert: insert.map(|ins| unescape(&ins)),
                all: cmd.all,
                ignore_case: cmd.ignore_case,
                word: cmd.word,
            }
        })
        .collect();

    let store = SqliteStore::open_rw()?;
    store.edit_note(
        cmd.selector.id.as_deref(),
        cmd.selector.title.as_deref(),
        &ops,
    )?;
    Ok(())
}

// ── open ──────────────────────────────────────────────────────────────────────

fn cmd_open(cmd: OpenArgs) -> Result<()> {
    let store = SqliteStore::open_ro()?;
    let note = store.resolve_note(
        cmd.selector.id.as_deref(),
        cmd.selector.title.as_deref(),
        false,
        false,
    )?;

    let mut url = format!("bear://x-callback-url/open-note?id={}", note.id);
    if let Some(header) = &cmd.header {
        url.push_str(&format!("&header={}", urlenccode(header)));
    }
    if cmd.edit {
        url.push_str("&edit=yes");
    }
    if cmd.new_window {
        url.push_str("&new_window=yes");
    }
    if cmd.float {
        url.push_str("&float=yes");
    }

    let status = std::process::Command::new("open")
        .arg(&url)
        .status()
        .map_err(|_| anyhow::anyhow!("Could not open Bear. Is the app installed?"))?;

    if !status.success() {
        bail!("Could not construct open-note URL");
    }
    Ok(())
}

fn urlenccode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || "-._~".contains(c) {
                vec![c]
            } else {
                format!("%{:02X}", c as u32).chars().collect()
            }
        })
        .collect()
}

// ── trash / archive / restore ─────────────────────────────────────────────────

fn cmd_trash(sel: NoteSelector) -> Result<()> {
    let store = SqliteStore::open_rw()?;
    store.trash_note(sel.id.as_deref(), sel.title.as_deref())?;
    Ok(())
}

fn cmd_archive(sel: NoteSelector) -> Result<()> {
    let store = SqliteStore::open_rw()?;
    store.archive_note(sel.id.as_deref(), sel.title.as_deref())?;
    Ok(())
}

fn cmd_restore(sel: NoteSelector) -> Result<()> {
    let store = SqliteStore::open_rw()?;
    store.restore_note(sel.id.as_deref(), sel.title.as_deref())?;
    Ok(())
}

// ── tags ──────────────────────────────────────────────────────────────────────

fn cmd_tags(cmd: TagsCommand) -> Result<()> {
    match cmd.subcommand {
        TagsSubcommand::List(args) => {
            let store = SqliteStore::open_ro()?;
            let tags =
                store.list_tags(args.selector.id.as_deref(), args.selector.title.as_deref())?;
            if args.count {
                println!("{}", tags.len());
                return Ok(());
            }
            let format = parse_format(&args.format)?;
            print_tags(&tags, format);
        }
        TagsSubcommand::Add(args) => {
            let store = SqliteStore::open_rw()?;
            let tag_refs: Vec<&str> = args.tags.iter().map(String::as_str).collect();
            store.add_tags(
                args.selector.id.as_deref(),
                args.selector.title.as_deref(),
                &tag_refs,
            )?;
        }
        TagsSubcommand::Remove(args) => {
            let store = SqliteStore::open_rw()?;
            let tag_refs: Vec<&str> = args.tags.iter().map(String::as_str).collect();
            store.remove_tags(
                args.selector.id.as_deref(),
                args.selector.title.as_deref(),
                &tag_refs,
            )?;
        }
        TagsSubcommand::Rename(args) => {
            let old = args
                .old_name()
                .ok_or_else(|| anyhow::anyhow!("old tag name required"))?;
            let new = args
                .new_name()
                .ok_or_else(|| anyhow::anyhow!("new tag name required"))?;
            let store = SqliteStore::open_rw()?;
            store.rename_tag(old, new, args.force)?;
        }
        TagsSubcommand::Delete(args) => {
            let name = args
                .tag_name()
                .ok_or_else(|| anyhow::anyhow!("tag name required"))?;
            let store = SqliteStore::open_rw()?;
            store.delete_tag(name)?;
        }
    }
    Ok(())
}

// ── pin ───────────────────────────────────────────────────────────────────────

fn cmd_pin(cmd: PinCommand) -> Result<()> {
    match cmd.subcommand {
        PinSubcommand::List(args) => {
            let store = SqliteStore::open_ro()?;
            let pins =
                store.list_pins(args.selector.id.as_deref(), args.selector.title.as_deref())?;
            let format = parse_format(&args.format)?;
            print_pins(&pins, format);
        }
        PinSubcommand::Add(args) => {
            let ctx_refs: Vec<&str> = args.contexts.iter().map(String::as_str).collect();
            let store = SqliteStore::open_rw()?;
            store.add_pins(
                args.selector.id.as_deref(),
                args.selector.title.as_deref(),
                &ctx_refs,
            )?;
        }
        PinSubcommand::Remove(args) => {
            let ctx_refs: Vec<&str> = args.contexts.iter().map(String::as_str).collect();
            let store = SqliteStore::open_rw()?;
            store.remove_pins(
                args.selector.id.as_deref(),
                args.selector.title.as_deref(),
                &ctx_refs,
            )?;
        }
    }
    Ok(())
}

// ── attachments ───────────────────────────────────────────────────────────────

fn cmd_attachments(cmd: AttachmentsCommand) -> Result<()> {
    match cmd.subcommand {
        AttachmentsSubcommand::List(args) => {
            let store = SqliteStore::open_ro()?;
            let attachments = store
                .list_attachments(args.selector.id.as_deref(), args.selector.title.as_deref())?;
            let format = parse_format(&args.format)?;
            print_attachments(&attachments, format);
        }
        AttachmentsSubcommand::Save(args) => {
            let store = SqliteStore::open_ro()?;
            let data = store.read_attachment(
                args.selector.id.as_deref(),
                args.selector.title.as_deref(),
                &args.filename,
            )?;
            use std::io::Write;
            std::io::stdout().write_all(&data)?;
        }
        AttachmentsSubcommand::Add(args) => {
            let mut data = Vec::new();
            std::io::stdin().read_to_end(&mut data)?;
            if data.is_empty() {
                bail!("no data on stdin; pipe file content to stdin");
            }
            let store = SqliteStore::open_rw()?;
            store.add_attachment(
                args.selector.id.as_deref(),
                args.selector.title.as_deref(),
                &args.filename,
                &data,
            )?;
        }
        AttachmentsSubcommand::Delete(args) => {
            let store = SqliteStore::open_rw()?;
            store.delete_attachment(
                args.selector.id.as_deref(),
                args.selector.title.as_deref(),
                &args.filename,
            )?;
        }
    }
    Ok(())
}
