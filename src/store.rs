use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::db::{coredata_to_unix, now_coredata, open_ro, open_rw};
use crate::model::{
    Attachment, InsertPosition, Note, PinRecord, SortDir, SortField, Tag, TagPosition,
};
use crate::notify::request_app_refresh;
use crate::prefs::check_app_lock;
use crate::search::parse_query;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn col_bool(v: Option<i64>) -> bool {
    v.unwrap_or(0) != 0
}

fn col_i64(v: Option<i64>) -> i64 {
    v.unwrap_or(0)
}

fn col_ts(v: Option<f64>) -> i64 {
    v.map(coredata_to_unix).unwrap_or(0)
}

fn col_str(v: Option<String>) -> String {
    v.unwrap_or_default()
}

// ── Core note row → Note struct ───────────────────────────────────────────────

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        pk: row.get(0)?,
        id: col_str(row.get(1)?),
        title: col_str(row.get(2)?),
        text: col_str(row.get(3)?),
        created: col_ts(row.get(4)?),
        modified: col_ts(row.get(5)?),
        trashed: col_bool(row.get(6)?),
        archived: col_bool(row.get(7)?),
        pinned: col_bool(row.get(8)?),
        locked: col_bool(row.get(9)?),
        encrypted: col_bool(row.get(10)?),
        has_images: col_bool(row.get(11)?),
        has_files: col_bool(row.get(12)?),
        has_source_code: col_bool(row.get(13)?),
        todo_completed: col_i64(row.get(14)?),
        todo_incompleted: col_i64(row.get(15)?),
        tags: Vec::new(),
        attachments: Vec::new(),
        pinned_in_tags: Vec::new(),
    })
}

const NOTE_COLS: &str = "n.Z_PK, n.ZUNIQUEIDENTIFIER, n.ZTITLE, n.ZTEXT,
    n.ZCREATIONDATE, n.ZMODIFICATIONDATE,
    n.ZTRASHED, n.ZARCHIVED, n.ZPINNED, n.ZLOCKED, n.ZENCRYPTED,
    n.ZHASIMAGES, n.ZHASFILES, n.ZHASSOURCECODE,
    n.ZTODOCOMPLETED, n.ZTODOINCOMPLETED";

// ── SqliteStore ───────────────────────────────────────────────────────────────

pub struct ListInput<'a> {
    pub tag: Option<&'a str>,
    pub sort: Vec<(SortField, SortDir)>,
    pub limit: Option<usize>,
    pub include_trashed: bool,
    pub include_archived: bool,
    pub include_tags: bool,
}

pub struct EditOp {
    pub at: String,
    pub replace: Option<String>,
    pub insert: Option<String>,
    pub all: bool,
    pub ignore_case: bool,
    pub word: bool,
}

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Open read-only (sufficient for all read commands).
    pub fn open_ro() -> Result<Self> {
        Ok(SqliteStore { conn: open_ro()? })
    }

    /// Open read-write (required for any mutation).
    pub fn open_rw() -> Result<Self> {
        Ok(SqliteStore { conn: open_rw()? })
    }

    // ── Tag helpers ───────────────────────────────────────────────────────────

    fn tags_for_note(&self, note_pk: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.ZTITLE FROM ZSFNOTETAG t
             JOIN Z_5TAGS jt ON jt.Z_13TAGS = t.Z_PK
             WHERE jt.Z_5NOTES = ?
             ORDER BY t.ZTITLE",
        )?;
        let tags: Result<Vec<String>> = stmt
            .query_map(params![note_pk], |row| row.get(0))?
            .map(|r| r.map_err(Into::into))
            .collect();
        tags
    }

    fn attachments_for_note(&self, note_pk: i64) -> Result<Vec<Attachment>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT ZFILENAME, ZFILESIZE, ZUNIQUEIDENTIFIER FROM ZSFNOTEFILE
             WHERE ZNOTE = ?
               AND (ZUNUSED IS NULL OR ZUNUSED = 0)
               AND (ZPERMANENTLYDELETED IS NULL OR ZPERMANENTLYDELETED = 0)
             ORDER BY ZINSERTIONDATE",
        )?;
        let rows: Result<Vec<Attachment>> = stmt
            .query_map(params![note_pk], |row| {
                Ok(Attachment {
                    filename: col_str(row.get(0)?),
                    size: col_i64(row.get(1)?),
                    uuid: col_str(row.get(2)?),
                })
            })?
            .map(|r| r.map_err(Into::into))
            .collect();
        rows
    }

    fn pin_contexts_for_note(&self, note_pk: i64, globally_pinned: bool) -> Result<Vec<String>> {
        let mut contexts = Vec::new();
        if globally_pinned {
            contexts.push("global".to_string());
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.ZTITLE FROM ZSFNOTETAG t
             JOIN Z_5PINNEDINTAGS jp ON jp.Z_13PINNEDINTAGS = t.Z_PK
             WHERE jp.Z_5PINNEDNOTES = ?
             ORDER BY t.ZTITLE",
        )?;
        let tag_pins: Result<Vec<String>> = stmt
            .query_map(params![note_pk], |row| row.get(0))?
            .map(|r| r.map_err(Into::into))
            .collect();
        contexts.extend(tag_pins?);
        Ok(contexts)
    }

    fn populate_note(
        &self,
        mut note: Note,
        include_tags: bool,
        include_attachments: bool,
        include_pins: bool,
    ) -> Result<Note> {
        if include_tags {
            note.tags = self.tags_for_note(note.pk)?;
        }
        if include_attachments {
            note.attachments = self.attachments_for_note(note.pk)?;
        }
        if include_pins {
            note.pinned_in_tags = self.pin_contexts_for_note(note.pk, note.pinned)?;
        }
        Ok(note)
    }

    // ── Note resolution ───────────────────────────────────────────────────────

    /// Resolve a note by ZUNIQUEIDENTIFIER or case-insensitive title.
    /// If title matches multiple notes, picks the most recently modified.
    pub fn resolve_note(
        &self,
        id: Option<&str>,
        title: Option<&str>,
        include_trashed: bool,
        include_archived: bool,
    ) -> Result<Note> {
        let trashed_clause = if include_trashed {
            ""
        } else {
            "AND (n.ZTRASHED IS NULL OR n.ZTRASHED = 0)"
        };
        let archived_clause = if include_archived {
            ""
        } else {
            "AND (n.ZARCHIVED IS NULL OR n.ZARCHIVED = 0)"
        };
        let base = format!(
            "FROM ZSFNOTE n
             WHERE (n.ZPERMANENTLYDELETED IS NULL OR n.ZPERMANENTLYDELETED = 0)
               {trashed_clause}
               {archived_clause}"
        );

        if let Some(uid) = id {
            let sql = format!("SELECT {NOTE_COLS} {base} AND n.ZUNIQUEIDENTIFIER = ?");
            let note = self
                .conn
                .query_row(&sql, params![uid], row_to_note)
                .with_context(|| format!("Note not found: {uid}"))?;
            return self.populate_note(note, true, false, false);
        }

        if let Some(t) = title {
            // Exact case-insensitive title match; most-recently-modified wins on ties.
            let sql_exact = format!(
                "SELECT {NOTE_COLS} {base}
                 AND n.ZTITLE = ? COLLATE NOCASE
                 ORDER BY n.ZMODIFICATIONDATE DESC
                 LIMIT 1"
            );
            let result = self.conn.query_row(&sql_exact, params![t], row_to_note);
            if let Ok(note) = result {
                return self.populate_note(note, true, false, false);
            }
            bail!("Note not found: {t}");
        }

        bail!("provide an id or --title to identify the note")
    }

    // ── List notes ────────────────────────────────────────────────────────────

    pub fn list_notes(&self, input: &ListInput<'_>) -> Result<Vec<Note>> {
        let tag_join = if let Some(tag) = input.tag {
            format!(
                "JOIN Z_5TAGS jt ON jt.Z_5NOTES = n.Z_PK \
                 JOIN ZSFNOTETAG ft ON ft.Z_PK = jt.Z_13TAGS AND ft.ZTITLE = '{}'",
                tag.replace('\'', "''")
            )
        } else {
            String::new()
        };

        let trashed_clause = if input.include_trashed {
            ""
        } else {
            "AND (n.ZTRASHED IS NULL OR n.ZTRASHED = 0)"
        };
        let archived_clause = if input.include_archived {
            ""
        } else {
            "AND (n.ZARCHIVED IS NULL OR n.ZARCHIVED = 0)"
        };

        let order_clause = if input.sort.is_empty() {
            // Default: pinned DESC, modified DESC
            "ORDER BY n.ZPINNED DESC NULLS LAST, n.ZMODIFICATIONDATE DESC".to_string()
        } else {
            let parts: Vec<String> = input
                .sort
                .iter()
                .map(|(field, dir)| {
                    let dir_str = match dir {
                        SortDir::Asc => "ASC",
                        SortDir::Desc => "DESC",
                    };
                    format!("{} {}", field.sql_column(), dir_str)
                })
                .collect();
            format!("ORDER BY {}", parts.join(", "))
        };

        let limit_clause = input
            .limit
            .map(|n| format!("LIMIT {n}"))
            .unwrap_or_default();

        let sql = format!(
            "SELECT {NOTE_COLS} FROM ZSFNOTE n
             {tag_join}
             WHERE (n.ZPERMANENTLYDELETED IS NULL OR n.ZPERMANENTLYDELETED = 0)
               {trashed_clause}
               {archived_clause}
             {order_clause}
             {limit_clause}"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let notes: Result<Vec<Note>> = stmt
            .query_map([], row_to_note)?
            .map(|r| r.map_err(Into::into))
            .collect();
        let mut notes = notes?;

        if input.include_tags {
            for note in &mut notes {
                note.tags = self.tags_for_note(note.pk)?;
            }
        }
        Ok(notes)
    }

    // ── Get single note (show) ────────────────────────────────────────────────

    pub fn get_note(
        &self,
        id: Option<&str>,
        title: Option<&str>,
        include_attachments: bool,
        include_pins: bool,
    ) -> Result<Note> {
        let mut note = self.resolve_note(id, title, false, false)?;
        if include_attachments {
            note.attachments = self.attachments_for_note(note.pk)?;
        }
        if include_pins {
            note.pinned_in_tags = self.pin_contexts_for_note(note.pk, note.pinned)?;
        }
        Ok(note)
    }

    // ── Cat (raw content) ─────────────────────────────────────────────────────

    pub fn cat_note(
        &self,
        id: Option<&str>,
        title: Option<&str>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<String> {
        let note = self.resolve_note(id, title, false, false)?;
        let text = &note.text;
        let start = offset.unwrap_or(0).min(text.len());
        let end = limit
            .map(|l| (start + l).min(text.len()))
            .unwrap_or(text.len());
        Ok(text[start..end].to_string())
    }

    // ── Search ────────────────────────────────────────────────────────────────

    pub fn search_notes(&self, query: &str, limit: Option<usize>) -> Result<Vec<Note>> {
        let pq = parse_query(query);

        let join_str = pq.joins.join("\n");
        let where_extra = if pq.clauses.is_empty() {
            String::new()
        } else {
            format!("AND {}", pq.clauses.join(" AND "))
        };
        let limit_clause = limit.map(|n| format!("LIMIT {n}")).unwrap_or_default();

        let sql = format!(
            "SELECT DISTINCT {NOTE_COLS} FROM ZSFNOTE n
             {join_str}
             WHERE (n.ZPERMANENTLYDELETED IS NULL OR n.ZPERMANENTLYDELETED = 0)
               AND (n.ZTRASHED IS NULL OR n.ZTRASHED = 0)
               AND (n.ZARCHIVED IS NULL OR n.ZARCHIVED = 0)
               {where_extra}
             ORDER BY n.ZMODIFICATIONDATE DESC
             {limit_clause}"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = pq
            .params
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();

        let notes: Result<Vec<Note>> = stmt
            .query_map(param_refs.as_slice(), row_to_note)?
            .map(|r| r.map_err(Into::into))
            .collect();
        let mut notes = notes?;
        for note in &mut notes {
            note.tags = self.tags_for_note(note.pk)?;
        }
        Ok(notes)
    }

    // ── Search-in ─────────────────────────────────────────────────────────────

    /// Returns (line_number, line_text) pairs for lines containing `string`.
    pub fn search_in_note(
        &self,
        id: Option<&str>,
        title: Option<&str>,
        string: &str,
        ignore_case: bool,
    ) -> Result<Vec<(usize, String)>> {
        let note = self.resolve_note(id, title, false, false)?;
        let needle = if ignore_case {
            string.to_lowercase()
        } else {
            string.to_string()
        };
        let mut matches = Vec::new();
        for (i, line) in note.text.lines().enumerate() {
            let hay = if ignore_case {
                line.to_lowercase()
            } else {
                line.to_string()
            };
            if hay.contains(&needle) {
                matches.push((i + 1, line.to_string()));
            }
        }
        Ok(matches)
    }

    // ── List tags ─────────────────────────────────────────────────────────────

    pub fn list_tags(&self, note_id: Option<&str>, note_title: Option<&str>) -> Result<Vec<Tag>> {
        if note_id.is_some() || note_title.is_some() {
            let note = self.resolve_note(note_id, note_title, false, false)?;
            let names = self.tags_for_note(note.pk)?;
            return Ok(names
                .into_iter()
                .enumerate()
                .map(|(i, name)| Tag { name, pk: i as i64 })
                .collect());
        }
        let mut stmt = self
            .conn
            .prepare_cached("SELECT Z_PK, ZTITLE FROM ZSFNOTETAG ORDER BY ZTITLE")?;
        let tags: Result<Vec<Tag>> = stmt
            .query_map([], |row| {
                Ok(Tag {
                    pk: row.get(0)?,
                    name: col_str(row.get(1)?),
                })
            })?
            .map(|r| r.map_err(Into::into))
            .collect();
        tags
    }

    // ── List pins ─────────────────────────────────────────────────────────────

    pub fn list_pins(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
    ) -> Result<Vec<PinRecord>> {
        if note_id.is_some() || note_title.is_some() {
            let note = self.resolve_note(note_id, note_title, false, false)?;
            let contexts = self.pin_contexts_for_note(note.pk, note.pinned)?;
            return Ok(contexts
                .into_iter()
                .map(|pin| PinRecord {
                    note_id: note.id.clone(),
                    pin,
                })
                .collect());
        }

        // All pins across the database
        let mut pins = Vec::new();

        // Global pins (ZPINNED = 1)
        let mut stmt = self.conn.prepare_cached(
            "SELECT ZUNIQUEIDENTIFIER FROM ZSFNOTE
             WHERE ZPINNED = 1
               AND (ZPERMANENTLYDELETED IS NULL OR ZPERMANENTLYDELETED = 0)
               AND (ZTRASHED IS NULL OR ZTRASHED = 0)",
        )?;
        let global: Result<Vec<String>> = stmt
            .query_map([], |row| row.get(0))?
            .map(|r| r.map_err(Into::into))
            .collect();
        for note_id in global? {
            pins.push(PinRecord {
                note_id,
                pin: "global".to_string(),
            });
        }

        // Tag-scoped pins
        let mut stmt = self.conn.prepare_cached(
            "SELECT n.ZUNIQUEIDENTIFIER, t.ZTITLE
             FROM ZSFNOTE n
             JOIN Z_5PINNEDINTAGS jp ON jp.Z_5PINNEDNOTES = n.Z_PK
             JOIN ZSFNOTETAG t ON t.Z_PK = jp.Z_13PINNEDINTAGS
             WHERE (n.ZPERMANENTLYDELETED IS NULL OR n.ZPERMANENTLYDELETED = 0)
               AND (n.ZTRASHED IS NULL OR n.ZTRASHED = 0)
             ORDER BY t.ZTITLE, n.ZUNIQUEIDENTIFIER",
        )?;
        let tag_pins: Result<Vec<PinRecord>> = stmt
            .query_map([], |row| {
                Ok(PinRecord {
                    note_id: col_str(row.get(0)?),
                    pin: col_str(row.get(1)?),
                })
            })?
            .map(|r| r.map_err(Into::into))
            .collect();
        pins.extend(tag_pins?);
        Ok(pins)
    }

    // ── List attachments ──────────────────────────────────────────────────────

    pub fn list_attachments(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
    ) -> Result<Vec<Attachment>> {
        let note = self.resolve_note(note_id, note_title, false, false)?;
        self.attachments_for_note(note.pk)
    }

    // ── Read attachment bytes ─────────────────────────────────────────────────

    pub fn read_attachment(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        filename: &str,
    ) -> Result<Vec<u8>> {
        let note = self.resolve_note(note_id, note_title, false, false)?;
        let attachments = self.attachments_for_note(note.pk)?;
        let att = attachments
            .iter()
            .find(|a| a.filename == filename)
            .with_context(|| format!("Attachment not found: {filename}"))?;

        let note_uuid = &note.id;
        let container = crate::db::group_container_path()?;
        let file_path = container
            .join("Application Data")
            .join("Local Files")
            .join("Note Files")
            .join(&att.uuid)
            .join(filename);

        if file_path.exists() {
            return std::fs::read(&file_path)
                .with_context(|| format!("File not found on disk: {}", file_path.display()));
        }

        // Fallback: search by note UUID subdirectory
        let alt_path = container
            .join("Application Data")
            .join("Local Files")
            .join("Note Files")
            .join(note_uuid)
            .join(filename);

        if alt_path.exists() {
            return std::fs::read(&alt_path).context("Attachment file not found on disk");
        }

        bail!("Attachment file not found on disk: {filename}")
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Write operations
    // All writes: check app lock → begin txn → mutate → commit → notify.
    // ─────────────────────────────────────────────────────────────────────────

    fn get_or_create_tag_pk(&self, name: &str) -> Result<i64> {
        // Lookup existing
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT Z_PK FROM ZSFNOTETAG WHERE ZTITLE = ?",
                params![name],
                |row| row.get(0),
            )
            .ok();

        if let Some(pk) = existing {
            return Ok(pk);
        }

        // Get next Z_PK from Z_PRIMARYKEY metadata
        let next_pk: i64 = self
            .conn
            .query_row(
                "SELECT Z_MAX FROM Z_PRIMARYKEY WHERE Z_NAME = 'SFNoteTag'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            + 1;

        // Update Z_PRIMARYKEY
        self.conn.execute(
            "UPDATE Z_PRIMARYKEY SET Z_MAX = ? WHERE Z_NAME = 'SFNoteTag'",
            params![next_pk],
        )?;

        let ent: i64 = self
            .conn
            .query_row(
                "SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = 'SFNoteTag'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(crate::db::SFNOTETAG_ENT);

        let uuid = Uuid::new_v4().to_string().to_uppercase();
        let now = now_coredata();
        self.conn.execute(
            "INSERT INTO ZSFNOTETAG (Z_PK, Z_ENT, Z_OPT, ZTITLE, ZUNIQUEIDENTIFIER,
             ZSORTING, ZSORTINGDIRECTION, ZPINNED, ZHIDESUBTAGSNOTES, ZISROOT, ZVERSION,
             ZMODIFICATIONDATE)
             VALUES (?, ?, 1, ?, ?, 0, 0, 0, 0, 0, 1, ?)",
            params![next_pk, ent, name, uuid, now],
        )?;
        Ok(next_pk)
    }

    fn add_tags_to_note(&self, note_pk: i64, tags: &[&str]) -> Result<()> {
        for &tag in tags {
            let tag_pk = self.get_or_create_tag_pk(tag)?;
            // INSERT OR IGNORE to skip if already linked
            self.conn.execute(
                "INSERT OR IGNORE INTO Z_5TAGS (Z_5NOTES, Z_13TAGS) VALUES (?, ?)",
                params![note_pk, tag_pk],
            )?;
        }
        Ok(())
    }

    fn next_note_pk(&self) -> Result<i64> {
        let max: i64 = self
            .conn
            .query_row(
                "SELECT Z_MAX FROM Z_PRIMARYKEY WHERE Z_NAME = 'SFNote'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            + 1;
        self.conn.execute(
            "UPDATE Z_PRIMARYKEY SET Z_MAX = ? WHERE Z_NAME = 'SFNote'",
            params![max],
        )?;
        Ok(max)
    }

    fn note_ent(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = 'SFNote'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(crate::db::SFNOTE_ENT)
    }

    // ── Create ────────────────────────────────────────────────────────────────

    pub fn create_note(&self, text: &str, tags: &[&str], if_not_exists: bool) -> Result<Note> {
        check_app_lock()?;

        // Extract title from first heading or first line
        let title = extract_title(text);

        // if_not_exists: check for existing note with same title
        if if_not_exists && !title.is_empty() {
            let existing = self.conn.query_row(
                "SELECT ZUNIQUEIDENTIFIER FROM ZSFNOTE
                 WHERE ZTITLE = ? COLLATE NOCASE
                   AND (ZTRASHED IS NULL OR ZTRASHED = 0)
                   AND (ZPERMANENTLYDELETED IS NULL OR ZPERMANENTLYDELETED = 0)
                 LIMIT 1",
                params![title],
                |row| row.get::<_, String>(0),
            );
            if let Ok(id) = existing {
                return self.resolve_note(Some(&id), None, false, false);
            }
        }

        let pk = self.next_note_pk()?;
        let ent = self.note_ent();
        let id = Uuid::new_v4().to_string().to_uppercase();
        let now = now_coredata();

        self.conn.execute(
            "INSERT INTO ZSFNOTE (Z_PK, Z_ENT, Z_OPT, ZUNIQUEIDENTIFIER, ZTITLE, ZTEXT,
             ZCREATIONDATE, ZMODIFICATIONDATE,
             ZTRASHED, ZARCHIVED, ZPINNED, ZLOCKED, ZENCRYPTED,
             ZHASIMAGES, ZHASFILES, ZHASSOURCECODE,
             ZTODOCOMPLETED, ZTODOINCOMPLETED, ZVERSION,
             ZPERMANENTLYDELETED)
             VALUES (?,?,1,?,?,?,?,?,0,0,0,0,0,0,0,0,0,0,1,0)",
            params![pk, ent, id, title, text, now, now],
        )?;

        if !tags.is_empty() {
            self.add_tags_to_note(pk, tags)?;
        }

        request_app_refresh();
        self.resolve_note(Some(&id), None, false, false)
    }

    // ── Append ────────────────────────────────────────────────────────────────

    pub fn append_to_note(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        content: &str,
        position: InsertPosition,
        update_modified: bool,
        tag_position: TagPosition,
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;
        let current = &note.text;

        let new_text = match position {
            InsertPosition::End => {
                // Insert before bottom-placed tags if tag_position is Bottom
                if tag_position == TagPosition::Bottom {
                    insert_before_bottom_tags(current, content)
                } else {
                    format!("{current}\n{content}")
                }
            }
            InsertPosition::Beginning => {
                // Insert after title (and any top-placed tags)
                if tag_position == TagPosition::Top {
                    insert_after_title_block(current, content)
                } else {
                    // Insert after title line only
                    insert_after_first_line(current, content)
                }
            }
        };

        let now = if update_modified {
            now_coredata()
        } else {
            // preserve existing timestamp
            self.conn
                .query_row(
                    "SELECT ZMODIFICATIONDATE FROM ZSFNOTE WHERE Z_PK = ?",
                    params![note.pk],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| now_coredata())
        };

        self.conn.execute(
            "UPDATE ZSFNOTE SET ZTEXT = ?, ZMODIFICATIONDATE = ? WHERE Z_PK = ?",
            params![new_text, now, note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    // ── Write (overwrite) ─────────────────────────────────────────────────────

    pub fn write_note(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        content: &str,
        base_hash: Option<&str>,
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;

        if let Some(expected) = base_hash {
            let actual = note.hash();
            if actual != expected {
                bail!(
                    "hashMismatch: base hash does not match current note content \
                     (expected {expected}, got {actual})"
                );
            }
        }

        let title = extract_title(content);
        let now = now_coredata();
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZTEXT = ?, ZTITLE = ?, ZMODIFICATIONDATE = ? WHERE Z_PK = ?",
            params![content, title, now, note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    // ── Edit (find/replace) ───────────────────────────────────────────────────

    pub fn edit_note(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        ops: &[EditOp],
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;
        let mut text = note.text.clone();
        let mut any_match = false;

        for op in ops {
            let result = apply_edit_op(&text, op);
            if result.matched {
                any_match = true;
                text = result.text;
            } else {
                bail!("String not found in note: {}", op.at);
            }
        }

        if !any_match {
            return Ok(());
        }

        let now = now_coredata();
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZTEXT = ?, ZMODIFICATIONDATE = ? WHERE Z_PK = ?",
            params![text, now, note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    // ── Trash / Archive / Restore ─────────────────────────────────────────────

    pub fn trash_note(&self, note_id: Option<&str>, note_title: Option<&str>) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, true, true)?;
        let now = now_coredata();
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZTRASHED = 1, ZTRASHEDDATE = ? WHERE Z_PK = ?",
            params![now, note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    pub fn archive_note(&self, note_id: Option<&str>, note_title: Option<&str>) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, true)?;
        let now = now_coredata();
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZARCHIVED = 1, ZARCHIVEDDATE = ? WHERE Z_PK = ?",
            params![now, note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    pub fn restore_note(&self, note_id: Option<&str>, note_title: Option<&str>) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, true, true)?;
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZTRASHED = 0, ZTRASHEDDATE = NULL,
                                ZARCHIVED = 0, ZARCHIVEDDATE = NULL
             WHERE Z_PK = ?",
            params![note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    // ── Tags add / remove / rename / delete ───────────────────────────────────

    pub fn add_tags(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        tags: &[&str],
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;
        self.add_tags_to_note(note.pk, tags)?;
        // Bear rewrites inline #tag markers itself on next open;
        // we only update the join tables here.
        let now = now_coredata();
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZMODIFICATIONDATE = ? WHERE Z_PK = ?",
            params![now, note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    pub fn remove_tags(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        tags: &[&str],
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;
        for &tag in tags {
            let tag_pk: Option<i64> = self
                .conn
                .query_row(
                    "SELECT Z_PK FROM ZSFNOTETAG WHERE ZTITLE = ?",
                    params![tag],
                    |row| row.get(0),
                )
                .ok();
            if let Some(tpk) = tag_pk {
                self.conn.execute(
                    "DELETE FROM Z_5TAGS WHERE Z_5NOTES = ? AND Z_13TAGS = ?",
                    params![note.pk, tpk],
                )?;
            } else {
                bail!("Tags not found: {tag}");
            }
        }
        let now = now_coredata();
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZMODIFICATIONDATE = ? WHERE Z_PK = ?",
            params![now, note.pk],
        )?;
        request_app_refresh();
        Ok(())
    }

    pub fn rename_tag(&self, old_name: &str, new_name: &str, force: bool) -> Result<()> {
        check_app_lock()?;

        // Check if new name already exists
        let new_exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM ZSFNOTETAG WHERE ZTITLE = ?",
                params![new_name],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);

        if new_exists && !force {
            bail!("Tag '{new_name}' already exists. Use --force to merge.");
        }

        let old_pk: i64 = self
            .conn
            .query_row(
                "SELECT Z_PK FROM ZSFNOTETAG WHERE ZTITLE = ?",
                params![old_name],
                |row| row.get(0),
            )
            .with_context(|| format!("Tags not found: {old_name}"))?;

        if new_exists {
            // Merge: re-point Z_5TAGS rows, delete old tag
            let new_pk: i64 = self
                .conn
                .query_row(
                    "SELECT Z_PK FROM ZSFNOTETAG WHERE ZTITLE = ?",
                    params![new_name],
                    |row| row.get(0),
                )
                .unwrap();
            self.conn.execute(
                "INSERT OR IGNORE INTO Z_5TAGS (Z_5NOTES, Z_13TAGS)
                 SELECT Z_5NOTES, ? FROM Z_5TAGS WHERE Z_13TAGS = ?",
                params![new_pk, old_pk],
            )?;
            self.conn
                .execute("DELETE FROM Z_5TAGS WHERE Z_13TAGS = ?", params![old_pk])?;
            self.conn
                .execute("DELETE FROM ZSFNOTETAG WHERE Z_PK = ?", params![old_pk])?;
        } else {
            self.conn.execute(
                "UPDATE ZSFNOTETAG SET ZTITLE = ? WHERE Z_PK = ?",
                params![new_name, old_pk],
            )?;
        }

        // Rewrite #tag markers in all note bodies
        self.rewrite_tag_in_notes(old_name, Some(new_name))?;
        request_app_refresh();
        Ok(())
    }

    pub fn delete_tag(&self, name: &str) -> Result<()> {
        check_app_lock()?;
        let tag_pk: i64 = self
            .conn
            .query_row(
                "SELECT Z_PK FROM ZSFNOTETAG WHERE ZTITLE = ?",
                params![name],
                |row| row.get(0),
            )
            .with_context(|| format!("Tags not found: {name}"))?;

        self.conn
            .execute("DELETE FROM Z_5TAGS WHERE Z_13TAGS = ?", params![tag_pk])?;
        self.conn
            .execute("DELETE FROM ZSFNOTETAG WHERE Z_PK = ?", params![tag_pk])?;

        self.rewrite_tag_in_notes(name, None)?;
        request_app_refresh();
        Ok(())
    }

    /// Rewrite `#tag` occurrences in all note bodies.
    /// `replacement = None` → remove the tag marker.
    fn rewrite_tag_in_notes(&self, old_name: &str, replacement: Option<&str>) -> Result<()> {
        // Find notes containing this tag in their text
        let pattern = format!("%#{}%", old_name.replace('%', "\\%"));
        let mut stmt = self.conn.prepare(
            "SELECT Z_PK, ZTEXT FROM ZSFNOTE
             WHERE ZTEXT LIKE ? ESCAPE '\\'
               AND (ZPERMANENTLYDELETED IS NULL OR ZPERMANENTLYDELETED = 0)",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![pattern], |row| {
                Ok((col_i64(row.get(0)?), col_str(row.get(1)?)))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let now = now_coredata();
        for (pk, text) in rows {
            let new_text = rewrite_tag_in_text(&text, old_name, replacement);
            if new_text != text {
                self.conn.execute(
                    "UPDATE ZSFNOTE SET ZTEXT = ?, ZMODIFICATIONDATE = ? WHERE Z_PK = ?",
                    params![new_text, now, pk],
                )?;
            }
        }
        Ok(())
    }

    // ── Pin add / remove ──────────────────────────────────────────────────────

    pub fn add_pins(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        contexts: &[&str],
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;
        let now = now_coredata();

        for &ctx in contexts {
            if ctx == "global" {
                self.conn.execute(
                    "UPDATE ZSFNOTE SET ZPINNED = 1, ZPINNEDDATE = ? WHERE Z_PK = ?",
                    params![now, note.pk],
                )?;
            } else {
                let tag_pk = self.get_or_create_tag_pk(ctx)?;
                self.conn.execute(
                    "INSERT OR IGNORE INTO Z_5PINNEDINTAGS (Z_5PINNEDNOTES, Z_13PINNEDINTAGS)
                     VALUES (?, ?)",
                    params![note.pk, tag_pk],
                )?;
            }
        }
        request_app_refresh();
        Ok(())
    }

    pub fn remove_pins(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        contexts: &[&str],
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;

        // Atomic: verify all targets exist first
        for &ctx in contexts {
            if ctx != "global" {
                let exists: bool = self
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM Z_5PINNEDINTAGS jp
                         JOIN ZSFNOTETAG t ON t.Z_PK = jp.Z_13PINNEDINTAGS
                         WHERE jp.Z_5PINNEDNOTES = ? AND t.ZTITLE = ?",
                        params![note.pk, ctx],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|n| n > 0)
                    .unwrap_or(false);
                if !exists {
                    bail!("Tags not found: {ctx}");
                }
            }
        }

        for &ctx in contexts {
            if ctx == "global" {
                self.conn.execute(
                    "UPDATE ZSFNOTE SET ZPINNED = 0, ZPINNEDDATE = NULL WHERE Z_PK = ?",
                    params![note.pk],
                )?;
            } else {
                self.conn.execute(
                    "DELETE FROM Z_5PINNEDINTAGS WHERE Z_5PINNEDNOTES = ?
                     AND Z_13PINNEDINTAGS = (SELECT Z_PK FROM ZSFNOTETAG WHERE ZTITLE = ?)",
                    params![note.pk, ctx],
                )?;
            }
        }
        request_app_refresh();
        Ok(())
    }

    // ── Attachment add / delete ───────────────────────────────────────────────

    pub fn add_attachment(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        filename: &str,
        data: &[u8],
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;

        let file_uuid = Uuid::new_v4().to_string().to_uppercase();
        let container = crate::db::group_container_path()?;
        let dir = container
            .join("Application Data")
            .join("Local Files")
            .join("Note Files")
            .join(&file_uuid);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create attachment directory {}", dir.display()))?;
        let file_path = dir.join(filename);
        std::fs::write(&file_path, data)
            .with_context(|| format!("cannot write attachment {}", file_path.display()))?;

        // Get next file PK
        let next_pk: i64 = self
            .conn
            .query_row(
                "SELECT Z_MAX FROM Z_PRIMARYKEY WHERE Z_NAME = 'SFNoteFile'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            + 1;
        self.conn.execute(
            "UPDATE Z_PRIMARYKEY SET Z_MAX = ? WHERE Z_NAME = 'SFNoteFile'",
            params![next_pk],
        )?;

        let ent: i64 = self
            .conn
            .query_row(
                "SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = 'SFNoteFile'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let ext = std::path::Path::new(filename)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let now = now_coredata();

        self.conn.execute(
            "INSERT INTO ZSFNOTEFILE (Z_PK, Z_ENT, Z_OPT, ZNOTE, ZUNIQUEIDENTIFIER,
             ZFILENAME, ZFILESIZE, ZNORMALIZEDFILEEXTENSION,
             ZDOWNLOADED, ZUPLOADED, ZUNUSED, ZPERMANENTLYDELETED,
             ZINSERTIONDATE, ZMODIFICATIONDATE, ZCREATIONDATE, ZVERSION)
             VALUES (?,?,1,?,?,?,?,?,1,0,0,0,?,?,?,1)",
            params![
                next_pk,
                ent,
                note.pk,
                file_uuid,
                filename,
                data.len() as i64,
                ext,
                now,
                now,
                now
            ],
        )?;

        let now_mod = now_coredata();
        self.conn.execute(
            "UPDATE ZSFNOTE SET ZHASFILES = 1, ZMODIFICATIONDATE = ? WHERE Z_PK = ?",
            params![now_mod, note.pk],
        )?;

        request_app_refresh();
        Ok(())
    }

    pub fn delete_attachment(
        &self,
        note_id: Option<&str>,
        note_title: Option<&str>,
        filename: &str,
    ) -> Result<()> {
        check_app_lock()?;
        let note = self.resolve_note(note_id, note_title, false, false)?;

        let rows = self.conn.execute(
            "UPDATE ZSFNOTEFILE SET ZUNUSED = 1
             WHERE ZNOTE = ? AND ZFILENAME = ?
               AND (ZUNUSED IS NULL OR ZUNUSED = 0)",
            params![note.pk, filename],
        )?;

        if rows == 0 {
            bail!("Attachment not found: {filename}");
        }
        request_app_refresh();
        Ok(())
    }
}

// ── Text manipulation helpers ─────────────────────────────────────────────────

/// Extract Bear-style title from note text (first # heading or first line).
pub fn extract_title(text: &str) -> String {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    String::new()
}

fn insert_before_bottom_tags(text: &str, content: &str) -> String {
    // Find the last block of lines that are pure #tag lines (bottom tag area).
    let lines: Vec<&str> = text.lines().collect();
    let mut split_at = lines.len();
    for i in (0..lines.len()).rev() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_tag_line(trimmed) {
            split_at = i;
        } else {
            break;
        }
    }
    if split_at == lines.len() {
        // No bottom tag block found, just append
        format!("{text}\n{content}")
    } else {
        let before = lines[..split_at].join("\n");
        let after = lines[split_at..].join("\n");
        format!("{before}\n{content}\n{after}")
    }
}

fn insert_after_title_block(text: &str, content: &str) -> String {
    // Skip leading # heading and any immediately following #tag lines
    let lines: Vec<&str> = text.lines().collect();
    let mut insert_after = 0;
    let mut past_title = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if !past_title {
            if trimmed.starts_with("# ") || trimmed.is_empty() {
                insert_after = i + 1;
                if trimmed.starts_with("# ") {
                    past_title = true;
                }
            } else {
                break;
            }
        } else if is_tag_line(trimmed) || trimmed.is_empty() {
            insert_after = i + 1;
        } else {
            break;
        }
    }
    let before = lines[..insert_after].join("\n");
    let after = lines[insert_after..].join("\n");
    if after.is_empty() {
        format!("{before}\n{content}")
    } else {
        format!("{before}\n{content}\n{after}")
    }
}

fn insert_after_first_line(text: &str, content: &str) -> String {
    if let Some(pos) = text.find('\n') {
        format!("{}\n{content}{}", &text[..pos], &text[pos..])
    } else {
        format!("{text}\n{content}")
    }
}

fn is_tag_line(line: &str) -> bool {
    // A "tag line" is a line consisting only of #tag tokens
    line.split_whitespace()
        .all(|tok| tok.starts_with('#') && tok.len() > 1)
}

/// Result of applying a single edit operation.
struct EditResult {
    text: String,
    matched: bool,
}

fn apply_edit_op(text: &str, op: &EditOp) -> EditResult {
    let needle = if op.ignore_case {
        op.at.to_lowercase()
    } else {
        op.at.clone()
    };

    // Determine replacement string
    let replacement: String = if let Some(r) = &op.replace {
        r.clone()
    } else if let Some(ins) = &op.insert {
        // insert = needle + ins
        format!("{}{}", op.at, ins)
    } else {
        op.at.clone()
    };

    let hay = if op.ignore_case {
        text.to_lowercase()
    } else {
        text.to_string()
    };

    if !hay.contains(&needle) {
        return EditResult {
            text: text.to_string(),
            matched: false,
        };
    }

    let result = if op.all {
        // Replace all occurrences
        replace_all(text, &hay, &needle, &op.at, &replacement, op.word)
    } else {
        // Replace first occurrence
        replace_first(text, &hay, &needle, &op.at, &replacement, op.word)
    };

    EditResult {
        matched: true,
        text: result,
    }
}

fn replace_first(
    original: &str,
    hay: &str,
    needle: &str,
    original_needle: &str,
    replacement: &str,
    word: bool,
) -> String {
    if let Some(pos) = find_match(hay, needle, word) {
        let end = pos + original_needle.len();
        format!("{}{}{}", &original[..pos], replacement, &original[end..])
    } else {
        original.to_string()
    }
}

fn replace_all(
    original: &str,
    hay: &str,
    needle: &str,
    original_needle: &str,
    replacement: &str,
    word: bool,
) -> String {
    let mut result = String::new();
    let mut last = 0usize;
    let mut search_from = 0usize;

    while let Some(pos) = find_match(&hay[search_from..], needle, word) {
        let abs_pos = search_from + pos;
        result.push_str(&original[last..abs_pos]);
        result.push_str(replacement);
        last = abs_pos + original_needle.len();
        search_from = last;
        if search_from >= hay.len() {
            break;
        }
    }
    result.push_str(&original[last..]);
    result
}

fn find_match(hay: &str, needle: &str, word: bool) -> Option<usize> {
    let pos = hay.find(needle)?;
    if !word {
        return Some(pos);
    }
    // Word boundary check: chars before and after must be non-alphanumeric
    let before_ok = pos == 0
        || hay[..pos]
            .chars()
            .last()
            .map(|c| !c.is_alphanumeric() && c != '_')
            .unwrap_or(true);
    let after_ok = (pos + needle.len()) >= hay.len()
        || hay[pos + needle.len()..]
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric() && c != '_')
            .unwrap_or(true);
    if before_ok && after_ok {
        Some(pos)
    } else {
        None
    }
}

/// Rewrite `#tag_name` occurrences in text.
/// `replacement = None` removes the marker; `Some(new_name)` replaces it.
fn rewrite_tag_in_text(text: &str, old_name: &str, replacement: Option<&str>) -> String {
    let marker = format!("#{old_name}");
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'#' {
            // Check if this is exactly our tag (followed by whitespace, newline, or end)
            let rest = &text[i..];
            if rest.starts_with(&marker) {
                let after = i + marker.len();
                let boundary = after >= bytes.len()
                    || bytes[after].is_ascii_whitespace()
                    || bytes[after] == b'#';
                if boundary {
                    if let Some(rep) = replacement {
                        result.push('#');
                        result.push_str(rep);
                    }
                    i += marker.len();
                    continue;
                }
            }
        }
        result.push(text[i..].chars().next().unwrap());
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_title_heading() {
        assert_eq!(extract_title("# My Note\n\nBody"), "My Note");
    }

    #[test]
    fn extract_title_first_line() {
        assert_eq!(extract_title("Quick note\nBody"), "Quick note");
    }

    #[test]
    fn extract_title_empty() {
        assert_eq!(extract_title(""), "");
    }

    #[test]
    fn rewrite_tag_rename() {
        let text = "Some text #old and #other";
        let result = rewrite_tag_in_text(text, "old", Some("new"));
        assert_eq!(result, "Some text #new and #other");
    }

    #[test]
    fn rewrite_tag_remove() {
        let text = "Text #remove keep";
        let result = rewrite_tag_in_text(text, "remove", None);
        assert_eq!(result, "Text  keep");
    }

    #[test]
    fn edit_replace_first() {
        let op = EditOp {
            at: "foo".into(),
            replace: Some("bar".into()),
            insert: None,
            all: false,
            ignore_case: false,
            word: false,
        };
        let result = apply_edit_op("foo baz foo", &op);
        assert!(result.matched);
        assert_eq!(result.text, "bar baz foo");
    }

    #[test]
    fn edit_replace_all() {
        let op = EditOp {
            at: "x".into(),
            replace: Some("Y".into()),
            insert: None,
            all: true,
            ignore_case: false,
            word: false,
        };
        let result = apply_edit_op("x and x", &op);
        assert_eq!(result.text, "Y and Y");
    }
}
