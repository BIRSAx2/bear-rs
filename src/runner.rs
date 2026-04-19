use anyhow::{Result, anyhow};
use clap::Parser;

use crate::bear::{join_tags, maybe_push, maybe_push_bool, open_bear_action};
use crate::cli::Cli;
use crate::cli::Commands;
use crate::config::{encode_file, load_token, resolve_database_path, save_token};
use crate::dates::parse_bear_date_filter;
use crate::db::BearDb;
use crate::export::export_notes;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let db = match &cli.command {
        Commands::OpenNote(_)
        | Commands::Tags
        | Commands::OpenTag(_)
        | Commands::Search(_)
        | Commands::Export(_)
        | Commands::Duplicates(_)
        | Commands::Stats(_)
        | Commands::Health(_)
        | Commands::Untagged(_)
        | Commands::Todo(_)
        | Commands::Today(_)
        | Commands::Locked(_) => Some(BearDb::open(resolve_database_path(
            cli.database.as_deref(),
        )?)?),
        _ => None,
    };

    match cli.command {
        Commands::Auth(cmd) => {
            save_token(&cmd.token)?;
            println!("Saved API token.");
        }
        Commands::OpenNote(cmd) => {
            let note = db
                .as_ref()
                .expect("db available for read command")
                .find_note(cmd.id.as_deref(), cmd.title.as_deref(), cmd.exclude_trashed)?;
            println!("{}", note.text);
        }
        Commands::Tags => {
            for tag in db.as_ref().expect("db available for read command").tags()? {
                println!("{tag}");
            }
        }
        Commands::OpenTag(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .notes_for_tags(&split_csv(&cmd.name), false)?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Search(cmd) => {
            let since = cmd
                .since
                .as_deref()
                .map(parse_bear_date_filter)
                .transpose()?;
            let before = cmd
                .before
                .as_deref()
                .map(parse_bear_date_filter)
                .transpose()?;
            let results = db.as_ref().expect("db available for read command").search(
                cmd.term.as_deref(),
                cmd.tag.as_deref(),
                false,
                since,
                before,
            )?;

            if cmd.json {
                let output = serde_json::json!({
                    "results": results.iter().map(|note| serde_json::json!({
                        "id": note.identifier,
                        "title": note.title,
                        "snippet": note.snippet,
                        "modified": note.modified_at,
                        "rank": note.rank,
                    })).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                for note in results {
                    println!("{}\t{}", note.identifier, note.title);
                    if let Some(snippet) = note.snippet {
                        println!("  {}", snippet);
                    }
                }
            }
        }
        Commands::Export(cmd) => {
            let notes = db
                .as_ref()
                .expect("db available for read command")
                .export_notes(cmd.tag.as_deref())?;
            let written = export_notes(&cmd.output, &notes, cmd.frontmatter, cmd.by_tag)?;
            println!(
                "Exported {} note(s) to {}",
                written.len(),
                cmd.output.display()
            );
        }
        Commands::Duplicates(cmd) => {
            let groups = db
                .as_ref()
                .expect("db available for read command")
                .duplicate_titles()?;

            if cmd.json {
                let total_duplicate_notes =
                    groups.iter().map(|group| group.notes.len()).sum::<usize>();
                let output = serde_json::json!({
                    "duplicateGroups": groups.len(),
                    "totalDuplicateNotes": total_duplicate_notes,
                    "groups": groups.iter().map(|group| serde_json::json!({
                        "title": group.title,
                        "count": group.notes.len(),
                        "notes": group.notes.iter().map(|note| serde_json::json!({
                            "id": note.identifier,
                            "modified": note.modified_at,
                        })).collect::<Vec<_>>()
                    })).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else if groups.is_empty() {
                println!("No duplicate titles found.");
            } else {
                println!("Found {} duplicate titles:\n", groups.len());
                for group in groups {
                    println!("\"{}\" ({} copies)", group.title, group.notes.len());
                    for note in group.notes {
                        if let Some(modified) = note.modified_at {
                            println!("  {}\t{}", note.identifier, modified);
                        } else {
                            println!("  {}", note.identifier);
                        }
                    }
                    println!();
                }
            }
        }
        Commands::Stats(cmd) => {
            let summary = db
                .as_ref()
                .expect("db available for read command")
                .stats_summary()?;
            let untagged_notes = summary.total_notes.saturating_sub(summary.tagged_notes);

            if cmd.json {
                let output = serde_json::json!({
                    "totalNotes": summary.total_notes,
                    "pinnedNotes": summary.pinned_notes,
                    "taggedNotes": summary.tagged_notes,
                    "untaggedNotes": untagged_notes,
                    "archivedNotes": summary.archived_notes,
                    "trashedNotes": summary.trashed_notes,
                    "uniqueTags": summary.unique_tags,
                    "totalWords": summary.total_words,
                    "notesWithTodos": summary.notes_with_todos,
                    "oldestModified": summary.oldest_modified,
                    "newestModified": summary.newest_modified,
                    "topTags": summary.top_tags.iter().map(|(tag, count)| serde_json::json!({
                        "tag": tag,
                        "count": count,
                    })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Notes: {}", summary.total_notes);
                println!("Pinned: {}", summary.pinned_notes);
                println!("Tagged: {}", summary.tagged_notes);
                println!("Untagged: {}", untagged_notes);
                println!("Archived: {}", summary.archived_notes);
                println!("Trashed: {}", summary.trashed_notes);
                println!("Tags: {}", summary.unique_tags);
                println!("Words: {}", summary.total_words);
                println!("Notes with TODOs: {}", summary.notes_with_todos);
                if let Some(oldest) = summary.oldest_modified {
                    println!("Oldest modified: {}", oldest);
                }
                if let Some(newest) = summary.newest_modified {
                    println!("Newest modified: {}", newest);
                }
                if !summary.top_tags.is_empty() {
                    println!("\nTop tags:");
                    for (tag, count) in summary.top_tags {
                        println!("  #{}: {}", tag, count);
                    }
                }
            }
        }
        Commands::Health(cmd) => {
            let summary = db
                .as_ref()
                .expect("db available for read command")
                .health_summary()?;

            if cmd.json {
                let output = serde_json::json!({
                    "totalNotes": summary.total_notes,
                    "duplicateGroups": summary.duplicate_groups,
                    "duplicateNotes": summary.duplicate_notes,
                    "emptyNotes": summary.empty_notes.iter().map(|note| serde_json::json!({
                        "id": note.identifier,
                        "title": note.title,
                    })).collect::<Vec<_>>(),
                    "untaggedNotes": summary.untagged_notes,
                    "oldTrashedNotes": summary.old_trashed_notes.iter().map(|note| serde_json::json!({
                        "id": note.identifier,
                        "title": note.title,
                    })).collect::<Vec<_>>(),
                    "largeNotes": summary.large_notes.iter().map(|note| serde_json::json!({
                        "id": note.identifier,
                        "title": note.title,
                        "sizeBytes": note.size_bytes,
                    })).collect::<Vec<_>>(),
                    "conflictNotes": summary.conflict_notes.iter().map(|note| serde_json::json!({
                        "id": note.identifier,
                        "title": note.title,
                    })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Bear health report\n");
                println!(
                    "{} duplicate title group(s) covering {} note(s)",
                    summary.duplicate_groups, summary.duplicate_notes
                );
                println!("{} empty note(s)", summary.empty_notes.len());
                println!("{} untagged note(s)", summary.untagged_notes);
                println!("{} old trashed note(s)", summary.old_trashed_notes.len());
                println!("{} large note(s)", summary.large_notes.len());
                println!("{} conflict-looking note(s)", summary.conflict_notes.len());
                println!("\n{} active note(s) checked", summary.total_notes);
            }
        }
        Commands::Untagged(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .untagged(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Todo(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .todo(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Today(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .today(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Locked(cmd) => {
            for note in db
                .as_ref()
                .expect("db available for read command")
                .locked(cmd.search.as_deref())?
            {
                println!("{}\t{}", note.identifier, note.title);
            }
        }
        Commands::Create(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "title", cmd.title);
            maybe_push(&mut query, "text", cmd.text);
            maybe_push(&mut query, "tags", join_tags(&cmd.tag));
            maybe_push_bool(&mut query, "open_note", cmd.open_note);
            maybe_push_bool(&mut query, "new_window", cmd.new_window);
            maybe_push_bool(&mut query, "float", cmd.float);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            maybe_push_bool(&mut query, "pin", cmd.pin);
            maybe_push_bool(&mut query, "edit", cmd.edit);
            maybe_push_bool(&mut query, "timestamp", cmd.timestamp);
            maybe_push(&mut query, "type", cmd.kind);
            maybe_push(&mut query, "url", cmd.url);

            if let Some(file) = cmd.file {
                let filename = cmd
                    .filename
                    .or_else(|| {
                        file.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .ok_or_else(|| {
                        anyhow!("--filename is required when the file path has no file name")
                    })?;
                let encoded = encode_file(&file)?;
                maybe_push(&mut query, "filename", Some(filename));
                maybe_push(&mut query, "file", Some(encoded));
            }

            open_bear_action("create", &query)?;
        }
        Commands::AddText(cmd) => {
            let text = match (cmd.text, cmd.file) {
                (Some(_), Some(_)) => {
                    anyhow::bail!("--file and positional TEXT are mutually exclusive")
                }
                (_, Some(path)) => Some(std::fs::read_to_string(&path)?),
                (text, None) => text,
            };
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "title", cmd.title);
            maybe_push(&mut query, "text", text);
            maybe_push(&mut query, "header", cmd.header);
            maybe_push(&mut query, "mode", Some(cmd.mode));
            maybe_push(&mut query, "tags", join_tags(&cmd.tag));
            maybe_push_bool(&mut query, "exclude_trashed", cmd.exclude_trashed);
            maybe_push_bool(&mut query, "new_line", cmd.new_line);
            maybe_push_bool(&mut query, "open_note", cmd.open_note);
            maybe_push_bool(&mut query, "new_window", cmd.new_window);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            maybe_push_bool(&mut query, "edit", cmd.edit);
            maybe_push_bool(&mut query, "timestamp", cmd.timestamp);
            open_bear_action("add-text", &query)?;
        }
        Commands::AddFile(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "title", cmd.title);
            maybe_push(&mut query, "header", cmd.header);
            maybe_push(&mut query, "mode", Some(cmd.mode));
            maybe_push_bool(&mut query, "open_note", cmd.open_note);
            maybe_push_bool(&mut query, "new_window", cmd.new_window);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            maybe_push_bool(&mut query, "edit", cmd.edit);
            maybe_push(
                &mut query,
                "filename",
                cmd.filename.or_else(|| {
                    cmd.file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                }),
            );
            maybe_push(&mut query, "file", Some(encode_file(&cmd.file)?));
            open_bear_action("add-file", &query)?;
        }
        Commands::GrabUrl(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "url", Some(cmd.url));
            maybe_push(&mut query, "tags", join_tags(&cmd.tag));
            maybe_push_bool(&mut query, "pin", cmd.pin);
            maybe_push_bool(&mut query, "wait", cmd.wait);
            open_bear_action("grab-url", &query)?;
        }
        Commands::Trash(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "search", cmd.search);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("trash", &query)?;
        }
        Commands::Archive(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "id", cmd.id);
            maybe_push(&mut query, "search", cmd.search);
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("archive", &query)?;
        }
        Commands::RenameTag(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "name", Some(cmd.name));
            maybe_push(&mut query, "new_name", Some(cmd.new_name));
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("rename-tag", &query)?;
        }
        Commands::DeleteTag(cmd) => {
            let mut query = Vec::new();
            maybe_push(&mut query, "name", Some(cmd.name));
            maybe_push_bool(&mut query, "show_window", cmd.show_window);
            open_bear_action("delete-tag", &query)?;
        }
        Commands::Raw(cmd) => {
            let mut params = cmd.params;
            let has_token = params.iter().any(|(key, _)| key == "token");
            if !has_token {
                if let Some(token) = cmd.token {
                    params.push(("token".into(), token));
                } else if cmd.use_saved_token {
                    if let Some(token) = load_token()? {
                        params.push(("token".into(), token));
                    }
                }
            }
            open_bear_action(&cmd.action, &params)?;
        }
    }

    Ok(())
}

fn split_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
