use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::model::{Attachment, Note, PinRecord, Tag};

// ── Output format ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "json" => Ok(OutputFormat::Json),
            "text" | "" => Ok(OutputFormat::Text),
            other => anyhow::bail!("unknown format: {other}"),
        }
    }
}

// ── Note fields ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteField {
    Id,
    Title,
    Tags,
    Hash,
    Length,
    Created,
    Modified,
    Pins,
    Location,
    Todos,
    Done,
    Attachments,
    Content,
    Locked,
}

impl NoteField {
    fn name(&self) -> &'static str {
        match self {
            NoteField::Id => "id",
            NoteField::Title => "title",
            NoteField::Tags => "tags",
            NoteField::Hash => "hash",
            NoteField::Length => "length",
            NoteField::Created => "created",
            NoteField::Modified => "modified",
            NoteField::Pins => "pins",
            NoteField::Location => "location",
            NoteField::Todos => "todos",
            NoteField::Done => "done",
            NoteField::Attachments => "attachments",
            NoteField::Content => "content",
            NoteField::Locked => "locked",
        }
    }
}

/// Parse a `--fields` value into a list of `NoteField`.
///
/// Special values:
/// - `"all"` → all fields except content
/// - `"all,content"` → all fields including content
pub fn parse_note_fields(spec: &str) -> anyhow::Result<Vec<NoteField>> {
    const ALL_FIELDS: &[NoteField] = &[
        NoteField::Id,
        NoteField::Title,
        NoteField::Tags,
        NoteField::Hash,
        NoteField::Length,
        NoteField::Created,
        NoteField::Modified,
        NoteField::Pins,
        NoteField::Location,
        NoteField::Todos,
        NoteField::Done,
        NoteField::Attachments,
        NoteField::Locked,
    ];

    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();

    // Handle "all" expansion
    let mut fields = Vec::new();
    for part in &parts {
        match *part {
            "all" => fields.extend_from_slice(ALL_FIELDS),
            "content" => fields.push(NoteField::Content),
            other => {
                let f = field_from_str(other)?;
                fields.push(f);
            }
        }
    }
    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    fields.retain(|f| seen.insert(f.name()));
    Ok(fields)
}

fn field_from_str(s: &str) -> anyhow::Result<NoteField> {
    match s {
        "id" => Ok(NoteField::Id),
        "title" => Ok(NoteField::Title),
        "tags" => Ok(NoteField::Tags),
        "hash" => Ok(NoteField::Hash),
        "length" => Ok(NoteField::Length),
        "created" => Ok(NoteField::Created),
        "modified" => Ok(NoteField::Modified),
        "pins" => Ok(NoteField::Pins),
        "location" => Ok(NoteField::Location),
        "todos" => Ok(NoteField::Todos),
        "done" => Ok(NoteField::Done),
        "attachments" => Ok(NoteField::Attachments),
        "content" => Ok(NoteField::Content),
        "locked" => Ok(NoteField::Locked),
        other => anyhow::bail!("invalid field: {other}"),
    }
}

/// Default fields for `list` / `search`.
pub fn default_list_fields() -> Vec<NoteField> {
    vec![NoteField::Id, NoteField::Title, NoteField::Tags]
}

/// Default fields for `show`.
pub fn default_show_fields() -> Vec<NoteField> {
    vec![
        NoteField::Id,
        NoteField::Title,
        NoteField::Tags,
        NoteField::Hash,
        NoteField::Length,
        NoteField::Created,
        NoteField::Modified,
        NoteField::Pins,
        NoteField::Location,
        NoteField::Todos,
        NoteField::Done,
        NoteField::Attachments,
        NoteField::Locked,
    ]
}

// ── Timestamp formatting ──────────────────────────────────────────────────────

fn fmt_ts(unix: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| unix.to_string())
}

// ── Note value extraction ─────────────────────────────────────────────────────

fn note_field_value(note: &Note, field: NoteField) -> String {
    match field {
        NoteField::Id => note.id.clone(),
        NoteField::Title => note.title.clone(),
        NoteField::Tags => note.tags.join(","),
        NoteField::Hash => note.hash(),
        NoteField::Length => note.length().to_string(),
        NoteField::Created => fmt_ts(note.created),
        NoteField::Modified => fmt_ts(note.modified),
        NoteField::Pins => note.pinned_in_tags.join(","),
        NoteField::Location => String::new(), // not in schema
        NoteField::Todos => note.todo_incompleted.to_string(),
        NoteField::Done => note.todo_completed.to_string(),
        NoteField::Attachments => note
            .attachments
            .iter()
            .map(|a| a.filename.as_str())
            .collect::<Vec<_>>()
            .join(","),
        NoteField::Content => note.text.clone(),
        NoteField::Locked => note.locked.to_string(),
    }
}

fn note_field_json(note: &Note, field: NoteField) -> (&'static str, Value) {
    match field {
        NoteField::Id => ("id", json!(note.id)),
        NoteField::Title => ("title", json!(note.title)),
        NoteField::Tags => ("tags", json!(note.tags)),
        NoteField::Hash => ("hash", json!(note.hash())),
        NoteField::Length => ("length", json!(note.length())),
        NoteField::Created => ("created", json!(fmt_ts(note.created))),
        NoteField::Modified => ("modified", json!(fmt_ts(note.modified))),
        NoteField::Pins => ("pins", json!(note.pinned_in_tags)),
        NoteField::Location => ("location", json!(null)),
        NoteField::Todos => ("todos", json!(note.todo_incompleted)),
        NoteField::Done => ("done", json!(note.todo_completed)),
        NoteField::Attachments => (
            "attachments",
            json!(
                note.attachments
                    .iter()
                    .map(|a| json!({"filename": a.filename, "size": a.size}))
                    .collect::<Vec<_>>()
            ),
        ),
        NoteField::Content => ("content", json!(note.text)),
        NoteField::Locked => ("locked", json!(note.locked)),
    }
}

// ── Print notes ───────────────────────────────────────────────────────────────

pub fn print_notes(notes: &[Note], fields: &[NoteField], format: OutputFormat) {
    if notes.is_empty() {
        eprintln!("No notes found.");
        return;
    }

    match format {
        OutputFormat::Text => {
            for note in notes {
                let row: Vec<String> = fields.iter().map(|&f| note_field_value(note, f)).collect();
                println!("{}", row.join("\t"));
            }
        }
        OutputFormat::Json => {
            let arr: Vec<Value> = notes
                .iter()
                .map(|note| {
                    let mut map = serde_json::Map::new();
                    for &f in fields {
                        let (key, val) = note_field_json(note, f);
                        map.insert(key.to_string(), val);
                    }
                    Value::Object(map)
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap());
        }
    }
}

pub fn print_note_count(count: usize) {
    println!("{count}");
}

// ── Print tags ────────────────────────────────────────────────────────────────

pub fn print_tags(tags: &[Tag], format: OutputFormat) {
    if tags.is_empty() {
        eprintln!("No notes found.");
        return;
    }
    match format {
        OutputFormat::Text => {
            for tag in tags {
                println!("{}", tag.name);
            }
        }
        OutputFormat::Json => {
            let arr: Vec<Value> = tags.iter().map(|t| json!({"name": t.name})).collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap());
        }
    }
}

// ── Print pins ────────────────────────────────────────────────────────────────

pub fn print_pins(pins: &[PinRecord], format: OutputFormat) {
    if pins.is_empty() {
        eprintln!("No notes found.");
        return;
    }
    match format {
        OutputFormat::Text => {
            for pin in pins {
                println!("{}", pin.pin);
            }
        }
        OutputFormat::Json => {
            let arr: Vec<Value> = pins
                .iter()
                .map(|p| json!({"noteId": p.note_id, "pin": p.pin}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap());
        }
    }
}

// ── Print attachments ─────────────────────────────────────────────────────────

pub fn print_attachments(attachments: &[Attachment], format: OutputFormat) {
    if attachments.is_empty() {
        eprintln!("No notes found.");
        return;
    }
    match format {
        OutputFormat::Text => {
            for att in attachments {
                println!("{}\t{}", att.filename, att.size);
            }
        }
        OutputFormat::Json => {
            let arr: Vec<Value> = attachments
                .iter()
                .map(|a| json!({"filename": a.filename, "size": a.size}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_fields() {
        let fields = parse_note_fields("id,title,tags").unwrap();
        assert_eq!(
            fields,
            vec![NoteField::Id, NoteField::Title, NoteField::Tags]
        );
    }

    #[test]
    fn parse_all_fields() {
        let fields = parse_note_fields("all").unwrap();
        assert!(!fields.contains(&NoteField::Content));
        assert!(fields.contains(&NoteField::Hash));
    }

    #[test]
    fn parse_all_with_content() {
        let fields = parse_note_fields("all,content").unwrap();
        assert!(fields.contains(&NoteField::Content));
    }

    #[test]
    fn parse_unknown_field_errors() {
        assert!(parse_note_fields("id,bogus").is_err());
    }
}
