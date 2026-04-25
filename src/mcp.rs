/// MCP server. JSON-RPC 2.0 over stdin/stdout, line-delimited.
/// Synchronous read/dispatch/write loop, no async runtime.
use std::io::{BufRead, Write as _};

use anyhow::Result;
use serde_json::{Value, json};

use crate::model::{InsertPosition, SortDir, SortField};
use crate::output::{default_list_fields, default_show_fields, parse_note_fields};
use crate::store::{EditOp, ListInput, SqliteStore};

// ── Wire types ────────────────────────────────────────────────────────────────

fn ok_response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn err_response(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[allow(dead_code)]
fn internal_err(id: &Value, e: anyhow::Error) -> Value {
    err_response(id, -32603, &e.to_string())
}

fn invalid_params(id: &Value, msg: &str) -> Value {
    err_response(id, -32602, msg)
}

// ── Tool definitions ──────────────────────────────────────────────────────────

fn tool_list() -> Value {
    json!([
        {
            "name": "list",
            "description": "List notes (id, title, tags by default).",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tag": {"type": "string"},
                    "sort": {"type": "string", "description": "e.g. modified:desc"},
                    "n": {"type": "integer"},
                    "fields": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        },
        {
            "name": "cat",
            "description": "Print raw note text.",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "offset": {"type": "integer"},
                    "limit": {"type": "integer"}
                }
            }
        },
        {
            "name": "show",
            "description": "Show note metadata and optionally content.",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "fields": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        },
        {
            "name": "search",
            "description": "Search notes using Bear query syntax.",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {"type": "string"},
                    "n": {"type": "integer"},
                    "fields": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        },
        {
            "name": "search_in",
            "description": "Search within a single note.",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["string"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "string": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        },
        {
            "name": "create",
            "description": "Create a new note.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "content": {"type": "string"},
                    "tags": {"type": "string", "description": "Comma-separated"},
                    "if_not_exists": {"type": "boolean"},
                    "fields": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        },
        {
            "name": "append",
            "description": "Append text to a note.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["content"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "content": {"type": "string"},
                    "position": {"type": "string", "enum": ["beginning", "end"]},
                    "no_update_modified": {"type": "boolean"}
                }
            }
        },
        {
            "name": "write",
            "description": "Overwrite note content.",
            "readOnlyHint": false,
            "destructiveHint": true,
            "inputSchema": {
                "type": "object",
                "required": ["content"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "content": {"type": "string"},
                    "base": {"type": "string", "description": "Expected SHA-256 hash of current content"}
                }
            }
        },
        {
            "name": "edit",
            "description": "Find/replace in a note.",
            "readOnlyHint": false,
            "destructiveHint": true,
            "inputSchema": {
                "type": "object",
                "required": ["at"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "at": {"type": "string"},
                    "replace": {"type": "string"},
                    "insert": {"type": "string"},
                    "all": {"type": "boolean"},
                    "ignore_case": {"type": "boolean"},
                    "word": {"type": "boolean"}
                }
            }
        },
        {
            "name": "trash",
            "description": "Move a note to trash.",
            "readOnlyHint": false,
            "destructiveHint": true,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"}
                }
            }
        },
        {
            "name": "archive",
            "description": "Archive a note.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"}
                }
            }
        },
        {
            "name": "restore",
            "description": "Restore a note from trash or archive.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"}
                }
            }
        },
        {
            "name": "tags_list",
            "description": "List all tags (or tags for a specific note).",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        },
        {
            "name": "tags_add",
            "description": "Add tags to a note.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["tags"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}}
                }
            }
        },
        {
            "name": "tags_remove",
            "description": "Remove tags from a note.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["tags"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}}
                }
            }
        },
        {
            "name": "tags_rename",
            "description": "Rename a tag across all notes.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["old_name", "new_name"],
                "properties": {
                    "old_name": {"type": "string"},
                    "new_name": {"type": "string"},
                    "force": {"type": "boolean"}
                }
            }
        },
        {
            "name": "tags_delete",
            "description": "Delete a tag and remove it from all notes.",
            "readOnlyHint": false,
            "destructiveHint": true,
            "inputSchema": {
                "type": "object",
                "required": ["tag"],
                "properties": {
                    "tag": {"type": "string"}
                }
            }
        },
        {
            "name": "pin_list",
            "description": "List pins (for all notes or a specific note).",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        },
        {
            "name": "pin_add",
            "description": "Pin a note in one or more contexts.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["contexts"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "contexts": {"type": "array", "items": {"type": "string"}}
                }
            }
        },
        {
            "name": "pin_remove",
            "description": "Unpin a note from one or more contexts.",
            "readOnlyHint": false,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "required": ["contexts"],
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "contexts": {"type": "array", "items": {"type": "string"}}
                }
            }
        },
        {
            "name": "attachments_list",
            "description": "List attachments for a note.",
            "readOnlyHint": true,
            "destructiveHint": false,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "title": {"type": "string"},
                    "format": {"type": "string", "enum": ["text", "json"]}
                }
            }
        }
    ])
}

// ── Tool dispatch ─────────────────────────────────────────────────────────────

fn dispatch(method: &str, args: &Value) -> Result<Value> {
    let str_arg = |key: &str| -> Option<String> {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let bool_arg = |key: &str| -> bool { args.get(key).and_then(|v| v.as_bool()).unwrap_or(false) };
    let u64_arg =
        |key: &str| -> Option<usize> { args.get(key).and_then(|v| v.as_u64()).map(|n| n as usize) };
    let arr_strings = |key: &str| -> Vec<String> {
        args.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };

    match method {
        "list" => {
            let store = SqliteStore::open_ro()?;
            let sort = if let Some(s) = str_arg("sort") {
                parse_sort_str(&s)?
            } else {
                vec![]
            };
            let input = ListInput {
                tag: None, // overwritten below after tag_owned is bound
                sort,
                limit: u64_arg("n"),
                include_trashed: false,
                include_archived: false,
                include_tags: true,
            };
            // Re-do with owned tag
            let tag_owned = str_arg("tag");
            let input = ListInput {
                tag: tag_owned.as_deref(),
                ..input
            };
            let notes = store.list_notes(&input)?;
            let fields = match str_arg("fields") {
                Some(f) => parse_note_fields(&f)?,
                None => default_list_fields(),
            };
            Ok(notes_to_json(&notes, &fields))
        }

        "cat" => {
            let store = SqliteStore::open_ro()?;
            let text = store.cat_note(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                u64_arg("offset"),
                u64_arg("limit"),
            )?;
            Ok(json!({"text": text}))
        }

        "show" => {
            let store = SqliteStore::open_ro()?;
            let note = store.get_note(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                true,
                true,
            )?;
            let fields = match str_arg("fields") {
                Some(f) => parse_note_fields(&f)?,
                None => default_show_fields(),
            };
            Ok(notes_to_json(&[note], &fields))
        }

        "search" => {
            let query = str_arg("query").unwrap_or_default();
            let store = SqliteStore::open_ro()?;
            let notes = store.search_notes(&query, u64_arg("n"))?;
            let fields = match str_arg("fields") {
                Some(f) => parse_note_fields(&f)?,
                None => default_list_fields(),
            };
            Ok(notes_to_json(&notes, &fields))
        }

        "search_in" => {
            let string = str_arg("string").unwrap_or_default();
            let store = SqliteStore::open_ro()?;
            let matches = store.search_in_note(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                &string,
                false,
            )?;
            Ok(json!({"matches": matches}))
        }

        "create" => {
            let tags: Vec<String> = str_arg("tags")
                .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();
            // create_note takes full text; title is extracted from the first line
            let content = {
                let title = str_arg("title").unwrap_or_default();
                let body = str_arg("content").unwrap_or_default();
                if title.is_empty() {
                    body
                } else if body.is_empty() {
                    format!("# {title}")
                } else {
                    format!("# {title}\n\n{body}")
                }
            };
            let store = SqliteStore::open_rw()?;
            let note = store.create_note(
                &content,
                &tags.iter().map(String::as_str).collect::<Vec<_>>(),
                bool_arg("if_not_exists"),
            )?;
            Ok(json!({"id": note.id}))
        }

        "append" => {
            let content = str_arg("content").unwrap_or_default();
            let pos = str_arg("position")
                .map(|p| {
                    if p == "beginning" {
                        InsertPosition::Beginning
                    } else {
                        InsertPosition::End
                    }
                })
                .unwrap_or(InsertPosition::End);
            let tag_pos = crate::prefs::load_prefs()
                .map(|p| p.tag_position)
                .unwrap_or(crate::model::TagPosition::Bottom);
            let store = SqliteStore::open_rw()?;
            store.append_to_note(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                &content,
                pos,
                !bool_arg("no_update_modified"),
                tag_pos,
            )?;
            Ok(json!({"ok": true}))
        }

        "write" => {
            let content = str_arg("content").unwrap_or_default();
            let store = SqliteStore::open_rw()?;
            store.write_note(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                &content,
                str_arg("base").as_deref(),
            )?;
            Ok(json!({"ok": true}))
        }

        "edit" => {
            let op = EditOp {
                at: str_arg("at").unwrap_or_default(),
                replace: str_arg("replace"),
                insert: str_arg("insert"),
                all: bool_arg("all"),
                ignore_case: bool_arg("ignore_case"),
                word: bool_arg("word"),
            };
            let store = SqliteStore::open_rw()?;
            store.edit_note(str_arg("id").as_deref(), str_arg("title").as_deref(), &[op])?;
            Ok(json!({"ok": true}))
        }

        "trash" => {
            let store = SqliteStore::open_rw()?;
            store.trash_note(str_arg("id").as_deref(), str_arg("title").as_deref())?;
            Ok(json!({"ok": true}))
        }

        "archive" => {
            let store = SqliteStore::open_rw()?;
            store.archive_note(str_arg("id").as_deref(), str_arg("title").as_deref())?;
            Ok(json!({"ok": true}))
        }

        "restore" => {
            let store = SqliteStore::open_rw()?;
            store.restore_note(str_arg("id").as_deref(), str_arg("title").as_deref())?;
            Ok(json!({"ok": true}))
        }

        "tags_list" => {
            let store = SqliteStore::open_ro()?;
            let tags = store.list_tags(str_arg("id").as_deref(), str_arg("title").as_deref())?;
            Ok(json!(tags.iter().map(|t| &t.name).collect::<Vec<_>>()))
        }

        "tags_add" => {
            let tags = arr_strings("tags");
            let store = SqliteStore::open_rw()?;
            store.add_tags(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                &tags.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            Ok(json!({"ok": true}))
        }

        "tags_remove" => {
            let tags = arr_strings("tags");
            let store = SqliteStore::open_rw()?;
            store.remove_tags(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                &tags.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            Ok(json!({"ok": true}))
        }

        "tags_rename" => {
            let old = str_arg("old_name").unwrap_or_default();
            let new = str_arg("new_name").unwrap_or_default();
            let store = SqliteStore::open_rw()?;
            store.rename_tag(&old, &new, bool_arg("force"))?;
            Ok(json!({"ok": true}))
        }

        "tags_delete" => {
            let tag = str_arg("tag").unwrap_or_default();
            let store = SqliteStore::open_rw()?;
            store.delete_tag(&tag)?;
            Ok(json!({"ok": true}))
        }

        "pin_list" => {
            let store = SqliteStore::open_ro()?;
            let pins = store.list_pins(str_arg("id").as_deref(), str_arg("title").as_deref())?;
            Ok(json!(
                pins.iter()
                    .map(|p| json!({"noteId": p.note_id, "pin": p.pin}))
                    .collect::<Vec<_>>()
            ))
        }

        "pin_add" => {
            let contexts = arr_strings("contexts");
            let store = SqliteStore::open_rw()?;
            store.add_pins(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                &contexts.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            Ok(json!({"ok": true}))
        }

        "pin_remove" => {
            let contexts = arr_strings("contexts");
            let store = SqliteStore::open_rw()?;
            store.remove_pins(
                str_arg("id").as_deref(),
                str_arg("title").as_deref(),
                &contexts.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
            Ok(json!({"ok": true}))
        }

        "attachments_list" => {
            let store = SqliteStore::open_ro()?;
            let atts =
                store.list_attachments(str_arg("id").as_deref(), str_arg("title").as_deref())?;
            Ok(json!(
                atts.iter()
                    .map(|a| json!({"filename": a.filename, "size": a.size}))
                    .collect::<Vec<_>>()
            ))
        }

        _ => anyhow::bail!("unknown tool: {}", method),
    }
}

// ── Small helpers ─────────────────────────────────────────────────────────────

fn notes_to_json(notes: &[crate::model::Note], fields: &[crate::output::NoteField]) -> Value {
    // Build JSON directly with the same logic as output.rs.
    // Simpler: build JSON directly with the same logic as output.rs.
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
    Value::Array(arr)
}

fn note_field_json(
    note: &crate::model::Note,
    field: crate::output::NoteField,
) -> (&'static str, Value) {
    use crate::output::NoteField;
    use chrono::{DateTime, Utc};
    let fmt_ts = |unix: i64| {
        DateTime::<Utc>::from_timestamp(unix, 0)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| unix.to_string())
    };
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

fn parse_sort_str(s: &str) -> Result<Vec<(SortField, SortDir)>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        let (field_str, dir_str) = if let Some(pos) = part.find(':') {
            (&part[..pos], &part[pos + 1..])
        } else {
            (part, "desc")
        };
        let field = match field_str {
            "modified" => SortField::Modified,
            "created" => SortField::Created,
            "title" => SortField::Title,
            _ => anyhow::bail!("unknown sort field: {}", field_str),
        };
        let dir = match dir_str {
            "asc" => SortDir::Asc,
            "desc" => SortDir::Desc,
            _ => anyhow::bail!("unknown sort direction: {}", dir_str),
        };
        out.push((field, dir));
    }
    Ok(out)
}

// ── Main server loop ──────────────────────────────────────────────────────────

pub fn run_server() -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(e) => {
                eprintln!("mcp: read error: {e}");
                break;
            }
        };

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = err_response(&Value::Null, -32700, &format!("parse error: {e}"));
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
                continue;
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let resp = match method {
            "initialize" => ok_response(
                &id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {"name": "bear-mcp", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"tools": {}}
                }),
            ),
            "tools/list" => ok_response(&id, json!({"tools": tool_list()})),
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                if tool_name.is_empty() {
                    invalid_params(&id, "missing tool name")
                } else {
                    match dispatch(tool_name, &arguments) {
                        Ok(result) => ok_response(
                            &id,
                            json!({
                                "content": [{"type": "text", "text": result.to_string()}],
                                "isError": false
                            }),
                        ),
                        Err(e) => ok_response(
                            &id,
                            json!({
                                "content": [{"type": "text", "text": e.to_string()}],
                                "isError": true
                            }),
                        ),
                    }
                }
            }
            "notifications/initialized" | "ping" => {
                // No response needed for notifications; send empty ok for ping
                if method == "ping" {
                    ok_response(&id, json!({}))
                } else {
                    continue;
                }
            }
            _ => err_response(&id, -32601, &format!("method not found: {method}")),
        };

        let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap_or_default());
        let _ = out.flush();
    }

    Ok(())
}
