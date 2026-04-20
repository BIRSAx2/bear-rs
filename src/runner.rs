use anyhow::{Result, anyhow, bail};
use chrono::Duration;
use clap::Parser;

use crate::cli::{AddFileMode, AddTextMode, Cli, Commands};
use crate::cloudkit::auth::AuthConfig;
use crate::cloudkit::client::{AttachPosition, CloudKitClient, extract_title, now_ms};
use crate::cloudkit::models::CkRecord;
use crate::dates::parse_bear_date_filter;
use crate::export::{ExportNote, export_notes};

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchResult {
    identifier: String,
    title: String,
    snippet: Option<String>,
    modified_at: Option<i64>,
    rank: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DuplicateNote {
    identifier: String,
    modified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DuplicateGroup {
    title: String,
    notes: Vec<DuplicateNote>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatsSummary {
    total_notes: usize,
    pinned_notes: usize,
    tagged_notes: usize,
    archived_notes: usize,
    trashed_notes: usize,
    unique_tags: usize,
    total_words: usize,
    notes_with_todos: usize,
    oldest_modified: Option<i64>,
    newest_modified: Option<i64>,
    top_tags: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthNoteIssue {
    identifier: String,
    title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LargeNoteIssue {
    identifier: String,
    title: String,
    size_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthSummary {
    total_notes: usize,
    duplicate_groups: usize,
    duplicate_notes: usize,
    empty_notes: Vec<HealthNoteIssue>,
    untagged_notes: usize,
    old_trashed_notes: Vec<HealthNoteIssue>,
    large_notes: Vec<LargeNoteIssue>,
    conflict_notes: Vec<HealthNoteIssue>,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Auth(cmd) => {
            let token = match cmd.token {
                Some(t) => t,
                None => crate::cloudkit::auth_server::acquire_token()?,
            };
            AuthConfig {
                ck_web_auth_token: token,
            }
            .save()?;
            println!("CloudKit auth token saved.");
        }

        // ── Read commands (CloudKit) ──────────────────────────────────────────
        Commands::OpenNote(cmd) => {
            let ck = load_ck()?;
            let note = resolve_note(
                cmd.id.as_deref(),
                cmd.title.as_deref(),
                !cmd.exclude_trashed,
                true,
                &ck,
            )?;
            println!("{}", note.str_field("textADP").unwrap_or(""));
        }
        Commands::Tags => {
            for tag in load_ck()?.list_tags()? {
                if let Some(name) = tag.str_field("name") {
                    println!("{name}");
                }
            }
        }
        Commands::OpenTag(cmd) => {
            let names = split_csv(&cmd.name);
            for note in load_ck()?.list_notes(false, false, None)? {
                let note_tags = note.string_list_field("tagsStrings");
                if names
                    .iter()
                    .any(|name| note_tags.iter().any(|tag| tag == name))
                {
                    println!(
                        "{}\t{}",
                        note.record_name,
                        note.str_field("title").unwrap_or("")
                    );
                }
            }
        }
        Commands::Search(cmd) => {
            let since = cmd
                .since
                .as_deref()
                .map(parse_cloudkit_date_filter)
                .transpose()?;
            let before = cmd
                .before
                .as_deref()
                .map(parse_cloudkit_date_filter)
                .transpose()?;
            let results = search_notes(
                &load_ck()?.list_notes(false, false, None)?,
                cmd.term.as_deref(),
                cmd.tag.as_deref(),
                since,
                before,
            );

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
                        println!("  {snippet}");
                    }
                }
            }
        }
        Commands::Notes(cmd) => {
            let notes = load_ck()?.list_notes(cmd.trashed, cmd.archived, cmd.limit)?;

            if cmd.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "notes": notes.iter().map(|note| serde_json::json!({
                            "recordName": note.record_name,
                            "id": note.str_field("uniqueIdentifier"),
                            "title": note.str_field("title"),
                            "subtitle": note.str_field("subtitleADP"),
                            "created": note.i64_field("sf_creationDate"),
                            "modified": note.i64_field("sf_modificationDate"),
                            "trashed": note.i64_field("trashed").unwrap_or(0) != 0,
                            "archived": note.i64_field("archived").unwrap_or(0) != 0,
                            "pinned": note.i64_field("pinned").unwrap_or(0) != 0,
                            "tags": note.fields.get("tagsStrings").and_then(|f| f.value.as_array()).map(|arr|
                                arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()
                            ).unwrap_or_default(),
                        })).collect::<Vec<_>>()
                    }))?
                );
            } else {
                for note in notes {
                    let title = note.str_field("title").unwrap_or("");
                    println!("{}\t{}", note.record_name, title);
                }
            }
        }
        Commands::Export(cmd) => {
            let notes = exportable_notes(
                &load_ck()?.list_notes(false, false, None)?,
                cmd.tag.as_deref(),
            );
            let written = export_notes(&cmd.output, &notes, cmd.frontmatter, cmd.by_tag)?;
            println!(
                "Exported {} note(s) to {}",
                written.len(),
                cmd.output.display()
            );
        }
        Commands::Duplicates(cmd) => {
            let groups = duplicate_groups(&load_ck()?.list_notes(false, true, None)?);
            if cmd.json {
                let total = groups.iter().map(|g| g.notes.len()).sum::<usize>();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "duplicateGroups": groups.len(),
                        "totalDuplicateNotes": total,
                        "groups": groups.iter().map(|g| serde_json::json!({
                            "title": g.title,
                            "count": g.notes.len(),
                            "notes": g.notes.iter().map(|n| serde_json::json!({
                                "id": n.identifier,
                                "modified": n.modified_at,
                            })).collect::<Vec<_>>()
                        })).collect::<Vec<_>>()
                    }))?
                );
            } else if groups.is_empty() {
                println!("No duplicate titles found.");
            } else {
                for g in groups {
                    println!("\"{}\" ({} copies)", g.title, g.notes.len());
                    for n in g.notes {
                        match n.modified_at {
                            Some(m) => println!("  {}\t{m}", n.identifier),
                            None => println!("  {}", n.identifier),
                        }
                    }
                }
            }
        }
        Commands::Stats(cmd) => {
            let s = stats_summary(
                &load_ck()?.list_notes(true, true, None)?,
                &load_ck()?.list_tags()?,
            );
            let untagged = s.total_notes.saturating_sub(s.tagged_notes);
            if cmd.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "totalNotes": s.total_notes,
                        "pinnedNotes": s.pinned_notes,
                        "taggedNotes": s.tagged_notes,
                        "untaggedNotes": untagged,
                        "archivedNotes": s.archived_notes,
                        "trashedNotes": s.trashed_notes,
                        "uniqueTags": s.unique_tags,
                        "totalWords": s.total_words,
                        "notesWithTodos": s.notes_with_todos,
                        "oldestModified": s.oldest_modified,
                        "newestModified": s.newest_modified,
                        "topTags": s.top_tags.iter().map(|(t, c)| serde_json::json!({"tag": t, "count": c})).collect::<Vec<_>>(),
                    }))?
                );
            } else {
                println!("Notes: {}", s.total_notes);
                println!("Pinned: {}", s.pinned_notes);
                println!("Tagged: {}", s.tagged_notes);
                println!("Untagged: {untagged}");
                println!("Archived: {}", s.archived_notes);
                println!("Trashed: {}", s.trashed_notes);
                println!("Tags: {}", s.unique_tags);
                println!("Words: {}", s.total_words);
                println!("Notes with TODOs: {}", s.notes_with_todos);
                if let Some(oldest) = s.oldest_modified {
                    println!("Oldest modified: {oldest}");
                }
                if let Some(newest) = s.newest_modified {
                    println!("Newest modified: {newest}");
                }
                if !s.top_tags.is_empty() {
                    println!("\nTop tags:");
                    for (tag, count) in s.top_tags {
                        println!("  #{tag}: {count}");
                    }
                }
            }
        }
        Commands::Health(cmd) => {
            let s = health_summary(&load_ck()?.list_notes(true, true, None)?);
            if cmd.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "totalNotes": s.total_notes,
                        "duplicateGroups": s.duplicate_groups,
                        "duplicateNotes": s.duplicate_notes,
                        "emptyNotes": s.empty_notes.iter().map(|n| serde_json::json!({"id": n.identifier, "title": n.title})).collect::<Vec<_>>(),
                        "untaggedNotes": s.untagged_notes,
                        "oldTrashedNotes": s.old_trashed_notes.iter().map(|n| serde_json::json!({"id": n.identifier, "title": n.title})).collect::<Vec<_>>(),
                        "largeNotes": s.large_notes.iter().map(|n| serde_json::json!({"id": n.identifier, "title": n.title, "sizeBytes": n.size_bytes})).collect::<Vec<_>>(),
                        "conflictNotes": s.conflict_notes.iter().map(|n| serde_json::json!({"id": n.identifier, "title": n.title})).collect::<Vec<_>>(),
                    }))?
                );
            } else {
                println!("Bear health report\n");
                println!(
                    "{} duplicate title group(s) covering {} note(s)",
                    s.duplicate_groups, s.duplicate_notes
                );
                println!("{} empty note(s)", s.empty_notes.len());
                println!("{} untagged note(s)", s.untagged_notes);
                println!("{} old trashed note(s)", s.old_trashed_notes.len());
                println!("{} large note(s)", s.large_notes.len());
                println!("{} conflict-looking note(s)", s.conflict_notes.len());
                println!("\n{} active note(s) checked", s.total_notes);
            }
        }
        Commands::Untagged(cmd) => {
            for note in load_ck()?.list_notes(false, false, None)? {
                if note.string_list_field("tagsStrings").is_empty()
                    && note_matches_optional_search(&note, cmd.search.as_deref())
                {
                    println!(
                        "{}\t{}",
                        note.record_name,
                        note.str_field("title").unwrap_or("")
                    );
                }
            }
        }
        Commands::Todo(cmd) => {
            for note in load_ck()?.list_notes(false, false, None)? {
                if note.str_field("textADP").unwrap_or("").contains("- [ ]")
                    && note_matches_optional_search(&note, cmd.search.as_deref())
                {
                    println!(
                        "{}\t{}",
                        note.record_name,
                        note.str_field("title").unwrap_or("")
                    );
                }
            }
        }
        Commands::Today(cmd) => {
            let start = parse_cloudkit_date_filter("today")?;
            for note in load_ck()?.list_notes(false, false, None)? {
                if note
                    .i64_field("sf_modificationDate")
                    .is_some_and(|v| v >= start)
                    && note_matches_optional_search(&note, cmd.search.as_deref())
                {
                    println!(
                        "{}\t{}",
                        note.record_name,
                        note.str_field("title").unwrap_or("")
                    );
                }
            }
        }
        Commands::Locked(cmd) => {
            for note in load_ck()?.list_notes(false, true, None)? {
                if note.bool_field("locked").unwrap_or(false)
                    && note_matches_optional_search(&note, cmd.search.as_deref())
                {
                    println!(
                        "{}\t{}",
                        note.record_name,
                        note.str_field("title").unwrap_or("")
                    );
                }
            }
        }

        // ── Write commands (CloudKit) ─────────────────────────────────────────
        Commands::Create(cmd) => {
            let text = read_text(cmd.text)?;
            let ck = load_ck()?;
            let record = ck.create_note(&text, vec![], cmd.tag)?;
            let title = extract_title(&text);
            println!("Created: {} ({})", title, record.record_name);
        }

        Commands::AddText(cmd) => {
            let ck = load_ck()?;
            let record_name = resolve_note_id(cmd.id.as_deref(), cmd.title.as_deref(), &ck)?;
            let new_text = read_text(cmd.text)?;

            // Fetch current content
            let note = ck.fetch_note(&record_name)?;
            let current = note.str_field("textADP").unwrap_or("").to_string();

            let updated = match cmd.mode {
                AddTextMode::ReplaceAll => new_text,
                AddTextMode::Prepend => {
                    if let Some(header) = cmd.header {
                        insert_after_header(&current, &header, &new_text)
                    } else {
                        format!("{new_text}\n{current}")
                    }
                }
                AddTextMode::Append => {
                    if let Some(header) = cmd.header {
                        insert_after_header(&current, &header, &new_text)
                    } else {
                        format!("{current}\n{new_text}")
                    }
                }
            };

            ck.update_note_text(&record_name, &updated)?;
        }

        Commands::AddFile(cmd) => {
            let ck = load_ck()?;
            let record_name = resolve_note_id(cmd.id.as_deref(), cmd.title.as_deref(), &ck)?;
            let filename = cmd
                .filename
                .or_else(|| {
                    cmd.file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .ok_or_else(|| anyhow!("--filename required when file path has no name"))?;
            let data = std::fs::read(&cmd.file)?;
            let position = match cmd.mode {
                AddFileMode::Append => AttachPosition::Append,
                AddFileMode::Prepend => AttachPosition::Prepend,
            };
            ck.attach_file(&record_name, &filename, &data, position)?;
            println!("Attached {filename} to {record_name}");
        }

        Commands::Trash(cmd) => {
            let ck = load_ck()?;
            let record_name = resolve_note_id(cmd.id.as_deref(), cmd.search.as_deref(), &ck)?;
            load_ck()?.trash_note(&record_name)?;
            println!("Trashed {record_name}");
        }

        Commands::Archive(cmd) => {
            let ck = load_ck()?;
            let record_name = resolve_note_id(cmd.id.as_deref(), cmd.search.as_deref(), &ck)?;
            load_ck()?.archive_note(&record_name)?;
            println!("Archived {record_name}");
        }

        Commands::RenameTag(cmd) => {
            let ck = load_ck()?;
            let tag_uuid = resolve_tag_id(&cmd.name, &ck)?;
            // Fetch the tag record and update its name field
            let record = ck.fetch_note(&tag_uuid)?; // SFNoteTag uses same lookup
            let change_tag = record
                .record_change_tag
                .clone()
                .ok_or_else(|| anyhow!("tag record has no recordChangeTag"))?;
            let mut fields = crate::cloudkit::models::Fields::new();
            fields.insert(
                "name".into(),
                crate::cloudkit::models::CkField::string(&cmd.new_name),
            );
            fields.insert(
                "sf_modificationDate".into(),
                crate::cloudkit::models::CkField::timestamp(crate::cloudkit::client::now_ms()),
            );
            ck.modify(vec![crate::cloudkit::models::ModifyOperation {
                operation_type: "update".into(),
                record: crate::cloudkit::models::CkRecord {
                    record_name: tag_uuid,
                    record_type: "SFNoteTag".into(),
                    fields,
                    record_change_tag: Some(change_tag),
                    deleted: false,
                    server_error_code: None,
                    reason: None,
                },
            }])?;
            println!("Renamed tag '{}' → '{}'", cmd.name, cmd.new_name);
        }

        Commands::DeleteTag(cmd) => {
            let ck = load_ck()?;
            let tag_uuid = resolve_tag_id(&cmd.name, &ck)?;
            let record = ck.fetch_note(&tag_uuid)?;
            let change_tag = record
                .record_change_tag
                .clone()
                .ok_or_else(|| anyhow!("tag record has no recordChangeTag"))?;
            ck.modify(vec![crate::cloudkit::models::ModifyOperation {
                operation_type: "delete".into(),
                record: crate::cloudkit::models::CkRecord {
                    record_name: tag_uuid,
                    record_type: "SFNoteTag".into(),
                    fields: std::collections::HashMap::new(),
                    record_change_tag: Some(change_tag),
                    deleted: true,
                    server_error_code: None,
                    reason: None,
                },
            }])?;
            println!("Deleted tag '{}'", cmd.name);
        }
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_ck() -> Result<CloudKitClient> {
    let auth = AuthConfig::load()?;
    CloudKitClient::new(auth)
}

fn resolve_note_id(id: Option<&str>, title: Option<&str>, ck: &CloudKitClient) -> Result<String> {
    if let Some(id) = id {
        return Ok(id.to_string());
    }
    if let Some(title) = title {
        return resolve_note_by_title(title, ck).map(|note| note.record_name);
    }
    bail!("provide --id or --title to identify the note")
}

fn resolve_note(
    id: Option<&str>,
    title: Option<&str>,
    include_trashed: bool,
    include_archived: bool,
    ck: &CloudKitClient,
) -> Result<CkRecord> {
    if let Some(id) = id {
        return ck.fetch_note(id);
    }
    if let Some(title) = title {
        return resolve_note_by_title_with_flags(title, include_trashed, include_archived, ck);
    }
    bail!("provide --id or --title")
}

fn resolve_note_by_title(title: &str, ck: &CloudKitClient) -> Result<CkRecord> {
    resolve_note_by_title_with_flags(title, false, true, ck)
}

fn resolve_note_by_title_with_flags(
    title: &str,
    include_trashed: bool,
    include_archived: bool,
    ck: &CloudKitClient,
) -> Result<CkRecord> {
    ck.list_notes(include_trashed, include_archived, None)?
        .into_iter()
        .filter(|note| note.str_field("title") == Some(title))
        .max_by_key(|note| note.i64_field("sf_modificationDate").unwrap_or(0))
        .ok_or_else(|| anyhow!("note not found"))
}

fn resolve_tag_id(name: &str, ck: &CloudKitClient) -> Result<String> {
    ck.list_tags()?
        .into_iter()
        .find(|tag| tag.str_field("name") == Some(name))
        .map(|tag| tag.record_name)
        .ok_or_else(|| anyhow!("tag not found: {name}"))
}

fn read_text(arg: Option<String>) -> Result<String> {
    match arg {
        Some(t) => Ok(t),
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

/// Insert `new_text` after the first line that starts with `## <header>`.
/// Falls back to appending if the header is not found.
fn insert_after_header(content: &str, header: &str, new_text: &str) -> String {
    let needle = format!("## {header}");
    let mut result = String::with_capacity(content.len() + new_text.len() + 2);
    let mut inserted = false;

    for line in content.lines() {
        result.push_str(line);
        result.push('\n');
        if !inserted && line.starts_with(&needle) {
            result.push_str(new_text);
            result.push('\n');
            inserted = true;
        }
    }

    if !inserted {
        result.push_str(new_text);
        result.push('\n');
    }
    result
}

fn split_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn note_matches_optional_search(note: &CkRecord, search: Option<&str>) -> bool {
    let Some(search) = search.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let needle = search.to_lowercase();
    note.str_field("title")
        .unwrap_or("")
        .to_lowercase()
        .contains(&needle)
        || note
            .str_field("textADP")
            .unwrap_or("")
            .to_lowercase()
            .contains(&needle)
}

fn search_notes(
    notes: &[CkRecord],
    term: Option<&str>,
    tag: Option<&str>,
    since: Option<i64>,
    before: Option<i64>,
) -> Vec<SearchResult> {
    let term = term.unwrap_or_default().trim().to_lowercase();
    let tag_filter = tag.map(str::trim).filter(|s| !s.is_empty());
    let mut results = Vec::new();

    for note in notes {
        let modified_at = note.i64_field("sf_modificationDate");
        if let Some(since) = since {
            if modified_at.is_some_and(|v| v < since) {
                continue;
            }
        }
        if let Some(before) = before {
            if modified_at.is_some_and(|v| v >= before) {
                continue;
            }
        }

        let tags = note.string_list_field("tagsStrings");
        if let Some(tag_filter) = tag_filter {
            if !tags.iter().any(|candidate| candidate == tag_filter) {
                continue;
            }
        }

        let title = note.str_field("title").unwrap_or("").to_string();
        let text = note.str_field("textADP").unwrap_or("").to_string();
        let title_lower = title.to_lowercase();
        let text_lower = text.to_lowercase();
        let title_match = !term.is_empty() && title_lower.contains(&term);
        let body_match = !term.is_empty() && text_lower.contains(&term);
        let tag_match = !term.is_empty() && tags.iter().any(|t| t.to_lowercase().contains(&term));

        if !term.is_empty() && !title_match && !body_match && !tag_match {
            continue;
        }

        let rank = if title_match {
            0
        } else if tag_match {
            1
        } else {
            2
        };
        results.push(SearchResult {
            identifier: note.record_name.clone(),
            title,
            snippet: if body_match {
                Some(make_snippet(&text, &text_lower, &term))
            } else {
                None
            },
            modified_at,
            rank,
        });
    }

    results.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| right.modified_at.cmp(&left.modified_at))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            .then_with(|| left.identifier.cmp(&right.identifier))
    });
    results
}

fn exportable_notes(notes: &[CkRecord], tag: Option<&str>) -> Vec<ExportNote> {
    let filter = tag.map(str::trim).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    for note in notes {
        let tags = note.string_list_field("tagsStrings");
        if let Some(filter) = filter {
            if !tags.iter().any(|tag| tag == filter) {
                continue;
            }
        }
        out.push(ExportNote {
            identifier: note.record_name.clone(),
            title: note.str_field("title").unwrap_or("").to_string(),
            text: note.str_field("textADP").unwrap_or("").to_string(),
            pinned: note.bool_field("pinned").unwrap_or(false),
            created_at: note.i64_field("sf_creationDate"),
            modified_at: note.i64_field("sf_modificationDate"),
            tags,
        });
    }
    out
}

fn duplicate_groups(notes: &[CkRecord]) -> Vec<DuplicateGroup> {
    let mut groups = std::collections::BTreeMap::<String, Vec<DuplicateNote>>::new();
    for note in notes {
        let title = note.str_field("title").unwrap_or("").trim().to_string();
        if title.is_empty() {
            continue;
        }
        groups.entry(title).or_default().push(DuplicateNote {
            identifier: note.record_name.clone(),
            modified_at: note.i64_field("sf_modificationDate").map(|v| v.to_string()),
        });
    }
    groups
        .into_iter()
        .filter_map(|(title, notes)| (notes.len() > 1).then_some(DuplicateGroup { title, notes }))
        .collect()
}

fn stats_summary(notes: &[CkRecord], tags: &[CkRecord]) -> StatsSummary {
    let mut total_notes = 0usize;
    let mut pinned_notes = 0usize;
    let mut tagged_notes = 0usize;
    let mut archived_notes = 0usize;
    let mut trashed_notes = 0usize;
    let mut total_words = 0usize;
    let mut notes_with_todos = 0usize;
    let mut oldest_modified = None;
    let mut newest_modified = None;
    let mut tag_counts = std::collections::BTreeMap::<String, usize>::new();

    for note in notes {
        if note.bool_field("trashed").unwrap_or(false) {
            trashed_notes += 1;
            continue;
        }
        total_notes += 1;
        if note.bool_field("pinned").unwrap_or(false) {
            pinned_notes += 1;
        }
        if note.bool_field("archived").unwrap_or(false) {
            archived_notes += 1;
        }
        let text = note.str_field("textADP").unwrap_or("");
        if text.contains("- [ ]") {
            notes_with_todos += 1;
        }
        total_words += text.split_whitespace().filter(|s| !s.is_empty()).count();
        let note_tags = note.string_list_field("tagsStrings");
        if !note_tags.is_empty() {
            tagged_notes += 1;
        }
        for tag in note_tags {
            *tag_counts.entry(tag).or_default() += 1;
        }
        if let Some(modified_at) = note.i64_field("sf_modificationDate") {
            oldest_modified =
                Some(oldest_modified.map_or(modified_at, |cur: i64| cur.min(modified_at)));
            newest_modified =
                Some(newest_modified.map_or(modified_at, |cur: i64| cur.max(modified_at)));
        }
    }

    let mut top_tags = tag_counts.into_iter().collect::<Vec<_>>();
    top_tags.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    top_tags.truncate(10);

    StatsSummary {
        total_notes,
        pinned_notes,
        tagged_notes,
        archived_notes,
        trashed_notes,
        unique_tags: tags.len(),
        total_words,
        notes_with_todos,
        oldest_modified,
        newest_modified,
        top_tags,
    }
}

fn health_summary(notes: &[CkRecord]) -> HealthSummary {
    const LARGE_NOTE_THRESHOLD_BYTES: usize = 100_000;
    let duplicate_groups = duplicate_groups(notes);
    let old_trashed_cutoff = now_ms() - Duration::days(30).num_milliseconds();

    let mut total_notes = 0usize;
    let mut empty_notes = Vec::new();
    let mut untagged_notes = 0usize;
    let mut old_trashed_notes = Vec::new();
    let mut large_notes = Vec::new();
    let mut conflict_notes = Vec::new();

    for note in notes {
        let identifier = note.record_name.clone();
        let title = display_title(note);
        let text = note.str_field("textADP").unwrap_or("");
        let trashed = note.bool_field("trashed").unwrap_or(false);

        if trashed {
            if note
                .i64_field("sf_modificationDate")
                .is_some_and(|v| v < old_trashed_cutoff)
            {
                old_trashed_notes.push(HealthNoteIssue { identifier, title });
            }
            continue;
        }

        total_notes += 1;
        if text.trim().is_empty() {
            empty_notes.push(HealthNoteIssue {
                identifier: note.record_name.clone(),
                title: title.clone(),
            });
        }
        if note.string_list_field("tagsStrings").is_empty() {
            untagged_notes += 1;
        }
        if text.len() >= LARGE_NOTE_THRESHOLD_BYTES {
            large_notes.push(LargeNoteIssue {
                identifier: note.record_name.clone(),
                title: title.clone(),
                size_bytes: text.len(),
            });
        }
        if note
            .str_field("conflictUniqueIdentifier")
            .is_some_and(|v| !v.is_empty())
        {
            conflict_notes.push(HealthNoteIssue {
                identifier: note.record_name.clone(),
                title,
            });
        }
    }

    let duplicate_note_count = duplicate_groups.iter().map(|g| g.notes.len()).sum();
    HealthSummary {
        total_notes,
        duplicate_groups: duplicate_groups.len(),
        duplicate_notes: duplicate_note_count,
        empty_notes,
        untagged_notes,
        old_trashed_notes,
        large_notes,
        conflict_notes,
    }
}

fn display_title(note: &CkRecord) -> String {
    let title = note.str_field("title").unwrap_or("").trim();
    if title.is_empty() {
        "(untitled)".to_string()
    } else {
        title.to_string()
    }
}

fn parse_cloudkit_date_filter(input: &str) -> Result<i64> {
    let seconds = parse_bear_date_filter(input)?;
    Ok((seconds + 978_307_200) * 1000)
}

fn make_snippet(text: &str, text_lower: &str, term: &str) -> String {
    if term.is_empty() {
        return text.lines().next().unwrap_or("").trim().to_string();
    }
    if let Some(pos) = text_lower.find(term) {
        let start = pos.saturating_sub(40);
        let end = (pos + term.len() + 60).min(text.len());
        return text[start..end].replace('\n', " ").trim().to_string();
    }
    text.lines().next().unwrap_or("").trim().to_string()
}
