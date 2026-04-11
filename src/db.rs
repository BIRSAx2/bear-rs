use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

#[derive(Debug)]
pub struct NoteRecord {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteListItem {
    pub identifier: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateNote {
    pub identifier: String,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub title: String,
    pub notes: Vec<DuplicateNote>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsSummary {
    pub total_notes: usize,
    pub pinned_notes: usize,
    pub tagged_notes: usize,
    pub archived_notes: usize,
    pub trashed_notes: usize,
    pub unique_tags: usize,
    pub total_words: usize,
    pub notes_with_todos: usize,
    pub oldest_modified: Option<i64>,
    pub newest_modified: Option<i64>,
    pub top_tags: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthNoteIssue {
    pub identifier: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LargeNoteIssue {
    pub identifier: String,
    pub title: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthSummary {
    pub total_notes: usize,
    pub duplicate_groups: usize,
    pub duplicate_notes: usize,
    pub empty_notes: Vec<HealthNoteIssue>,
    pub untagged_notes: usize,
    pub old_trashed_notes: Vec<HealthNoteIssue>,
    pub large_notes: Vec<LargeNoteIssue>,
    pub conflict_notes: Vec<HealthNoteIssue>,
}

pub struct BearDb {
    connection: Connection,
}

impl BearDb {
    pub fn open(path: PathBuf) -> Result<Self> {
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|err| anyhow!("failed to open Bear database at {}: {err}", path.display()))?;
        Ok(Self { connection })
    }

    #[cfg(test)]
    fn from_connection(connection: Connection) -> Self {
        Self { connection }
    }

    pub fn find_note(
        &self,
        id: Option<&str>,
        title: Option<&str>,
        exclude_trashed: bool,
    ) -> Result<NoteRecord> {
        if id.is_none() && title.is_none() {
            bail!("provide either --id or --title");
        }

        let sql = if id.is_some() {
            if exclude_trashed {
                "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, ''), coalesce(ZTEXT, '')
                 from ZSFNOTE
                 where ZUNIQUEIDENTIFIER = ?1 and ZTRASHED = 0
                 limit 1"
            } else {
                "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, ''), coalesce(ZTEXT, '')
                 from ZSFNOTE
                 where ZUNIQUEIDENTIFIER = ?1
                 limit 1"
            }
        } else if exclude_trashed {
            "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, ''), coalesce(ZTEXT, '')
             from ZSFNOTE
             where ZTITLE = ?1 and ZTRASHED = 0
             order by ZMODIFICATIONDATE desc
             limit 1"
        } else {
            "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, ''), coalesce(ZTEXT, '')
             from ZSFNOTE
             where ZTITLE = ?1
             order by ZMODIFICATIONDATE desc
             limit 1"
        };

        let needle = id.or(title).unwrap_or_default();
        self.connection
            .query_row(sql, [needle], |row| Ok(NoteRecord { text: row.get(2)? }))
            .optional()?
            .ok_or_else(|| anyhow!("note not found"))
    }

    pub fn tags(&self) -> Result<Vec<String>> {
        let mut stmt = self.connection.prepare(
            "select ZTITLE
             from ZSFNOTETAG
             where ZTITLE is not null and ZENCRYPTED = 0
             order by lower(ZTITLE) asc",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn notes_for_tags(
        &self,
        tags: &[String],
        include_trashed: bool,
    ) -> Result<Vec<NoteListItem>> {
        if tags.is_empty() {
            bail!("at least one tag is required");
        }

        let placeholders = (0..tags.len()).map(|_| "?").collect::<Vec<_>>().join(", ");
        let trashed_filter = if include_trashed {
            ""
        } else {
            "and n.ZTRASHED = 0"
        };
        let sql = format!(
            "select distinct n.ZUNIQUEIDENTIFIER, coalesce(n.ZTITLE, '')
             from ZSFNOTE n
             join Z_5TAGS nt on nt.Z_5NOTES = n.Z_PK
             join ZSFNOTETAG t on t.Z_PK = nt.Z_13TAGS
             where t.ZTITLE in ({placeholders})
               and n.ZENCRYPTED = 0
               and n.ZLOCKED = 0
               and n.ZPERMANENTLYDELETED = 0
               {trashed_filter}
             order by lower(coalesce(n.ZTITLE, '')) asc"
        );
        let mut stmt = self.connection.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(tags.iter()), |row| {
            Ok(NoteListItem {
                identifier: row.get(0)?,
                title: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn search(
        &self,
        term: Option<&str>,
        tag: Option<&str>,
        include_trashed: bool,
    ) -> Result<Vec<NoteListItem>> {
        let term = term.unwrap_or_default();
        let like = format!("%{term}%");

        if let Some(tag) = tag {
            let sql = if include_trashed {
                "select distinct n.ZUNIQUEIDENTIFIER, coalesce(n.ZTITLE, '')
                 from ZSFNOTE n
                 join Z_5TAGS nt on nt.Z_5NOTES = n.Z_PK
                 join ZSFNOTETAG t on t.Z_PK = nt.Z_13TAGS
                 where t.ZTITLE = ?1
                   and n.ZENCRYPTED = 0
                   and n.ZLOCKED = 0
                   and n.ZPERMANENTLYDELETED = 0
                   and (coalesce(n.ZTITLE, '') like ?2 or coalesce(n.ZTEXT, '') like ?2)
                 order by lower(coalesce(n.ZTITLE, '')) asc"
            } else {
                "select distinct n.ZUNIQUEIDENTIFIER, coalesce(n.ZTITLE, '')
                 from ZSFNOTE n
                 join Z_5TAGS nt on nt.Z_5NOTES = n.Z_PK
                 join ZSFNOTETAG t on t.Z_PK = nt.Z_13TAGS
                 where t.ZTITLE = ?1
                   and n.ZTRASHED = 0
                   and n.ZARCHIVED = 0
                   and n.ZENCRYPTED = 0
                   and n.ZLOCKED = 0
                   and n.ZPERMANENTLYDELETED = 0
                   and (coalesce(n.ZTITLE, '') like ?2 or coalesce(n.ZTEXT, '') like ?2)
                 order by lower(coalesce(n.ZTITLE, '')) asc"
            };
            let mut stmt = self.connection.prepare(sql)?;
            let rows = stmt.query_map(params![tag, like], |row| {
                Ok(NoteListItem {
                    identifier: row.get(0)?,
                    title: row.get(1)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        } else {
            let sql = if include_trashed {
                "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, '')
                 from ZSFNOTE
                 where ZENCRYPTED = 0
                   and ZLOCKED = 0
                   and ZPERMANENTLYDELETED = 0
                   and (coalesce(ZTITLE, '') like ?1 or coalesce(ZTEXT, '') like ?1)
                 order by lower(coalesce(ZTITLE, '')) asc"
            } else {
                "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, '')
                 from ZSFNOTE
                 where ZTRASHED = 0
                   and ZARCHIVED = 0
                   and ZENCRYPTED = 0
                   and ZLOCKED = 0
                   and ZPERMANENTLYDELETED = 0
                   and (coalesce(ZTITLE, '') like ?1 or coalesce(ZTEXT, '') like ?1)
                 order by lower(coalesce(ZTITLE, '')) asc"
            };
            let mut stmt = self.connection.prepare(sql)?;
            let rows = stmt.query_map([like], |row| {
                Ok(NoteListItem {
                    identifier: row.get(0)?,
                    title: row.get(1)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        }
    }

    pub fn duplicate_titles(&self) -> Result<Vec<DuplicateGroup>> {
        let mut stmt = self.connection.prepare(
            "select coalesce(ZTITLE, ''), ZUNIQUEIDENTIFIER, ZMODIFICATIONDATE
             from ZSFNOTE
             where ZTRASHED = 0
               and ZPERMANENTLYDELETED = 0
               and trim(coalesce(ZTITLE, '')) <> ''
             order by lower(trim(coalesce(ZTITLE, ''))) asc, ZMODIFICATIONDATE desc",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<f64>>(2)?,
            ))
        })?;

        let mut groups = std::collections::BTreeMap::<String, Vec<DuplicateNote>>::new();

        for row in rows {
            let (title, identifier, modified_at) = row?;
            let trimmed_title = title.trim().to_string();
            groups
                .entry(trimmed_title)
                .or_default()
                .push(DuplicateNote {
                    identifier,
                    modified_at: modified_at.map(|value| value.to_string()),
                });
        }

        Ok(groups
            .into_iter()
            .filter_map(|(title, notes)| {
                if notes.len() > 1 {
                    Some(DuplicateGroup { title, notes })
                } else {
                    None
                }
            })
            .collect())
    }

    pub fn stats_summary(&self) -> Result<StatsSummary> {
        let mut stmt = self.connection.prepare(
            "select coalesce(ZTITLE, ''), coalesce(ZTEXT, ''), ZTRASHED, ZARCHIVED, ZPINNED, ZMODIFICATIONDATE
             from ZSFNOTE
             where ZPERMANENTLYDELETED = 0",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<f64>>(5)?,
            ))
        })?;

        let tags = self.tags()?;
        let mut total_notes = 0usize;
        let mut pinned_notes = 0usize;
        let mut tagged_notes = 0usize;
        let mut archived_notes = 0usize;
        let mut trashed_notes = 0usize;
        let mut total_words = 0usize;
        let mut notes_with_todos = 0usize;
        let mut oldest_modified: Option<i64> = None;
        let mut newest_modified: Option<i64> = None;

        let mut tag_counts = std::collections::BTreeMap::<String, usize>::new();

        let note_tags = self.note_tag_map()?;

        for row in rows {
            let (_title, text, trashed, archived, pinned, modified_at) = row?;

            if trashed == 1 {
                trashed_notes += 1;
                continue;
            }

            total_notes += 1;
            if pinned == 1 {
                pinned_notes += 1;
            }
            if archived == 1 {
                archived_notes += 1;
            }
            if text.contains("- [ ]") {
                notes_with_todos += 1;
            }
            total_words += text
                .split_whitespace()
                .filter(|part| !part.is_empty())
                .count();

            if let Some(modified_at) = modified_at.map(|value| value as i64) {
                oldest_modified = Some(match oldest_modified {
                    Some(current) => current.min(modified_at),
                    None => modified_at,
                });
                newest_modified = Some(match newest_modified {
                    Some(current) => current.max(modified_at),
                    None => modified_at,
                });
            }
        }

        for (note_id, tags) in note_tags {
            if self.is_trashed(&note_id)? {
                continue;
            }
            if !tags.is_empty() {
                tagged_notes += 1;
            }
            for tag in tags {
                *tag_counts.entry(tag).or_default() += 1;
            }
        }

        let mut top_tags = tag_counts.into_iter().collect::<Vec<_>>();
        top_tags.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        top_tags.truncate(10);

        Ok(StatsSummary {
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
        })
    }

    pub fn health_summary(&self) -> Result<HealthSummary> {
        const OLD_TRASH_THRESHOLD: i64 = 30;
        const LARGE_NOTE_THRESHOLD_BYTES: usize = 100_000;

        let mut stmt = self.connection.prepare(
            "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, ''), coalesce(ZTEXT, ''), ZTRASHED, ZARCHIVED, ZMODIFICATIONDATE
             from ZSFNOTE
             where ZPERMANENTLYDELETED = 0
               and ZENCRYPTED = 0
               and ZLOCKED = 0",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<f64>>(5)?,
            ))
        })?;

        let duplicate_groups = self.duplicate_titles()?;
        let note_tags = self.note_tag_map()?;

        let mut total_notes = 0usize;
        let mut empty_notes = Vec::new();
        let mut untagged_notes = 0usize;
        let mut old_trashed_notes = Vec::new();
        let mut large_notes = Vec::new();
        let mut conflict_notes = Vec::new();
        let mut max_modified = 0i64;

        let mut rows_cache = Vec::new();
        for row in rows {
            let row = row?;
            if let Some(modified_at) = row.5.map(|value| value as i64) {
                max_modified = max_modified.max(modified_at);
            }
            rows_cache.push(row);
        }

        for (identifier, title, text, trashed, archived, modified_at) in rows_cache {
            let display_title = if title.trim().is_empty() {
                "(untitled)".to_string()
            } else {
                title.trim().to_string()
            };

            if trashed == 0 {
                total_notes += 1;

                if text.trim().is_empty() {
                    empty_notes.push(HealthNoteIssue {
                        identifier: identifier.clone(),
                        title: display_title.clone(),
                    });
                }

                if !note_tags
                    .get(&identifier)
                    .map(|tags| !tags.is_empty())
                    .unwrap_or(false)
                {
                    untagged_notes += 1;
                }

                let size_bytes = text.len();
                if size_bytes >= LARGE_NOTE_THRESHOLD_BYTES {
                    large_notes.push(LargeNoteIssue {
                        identifier: identifier.clone(),
                        title: display_title.clone(),
                        size_bytes,
                    });
                }

                let lower_title = display_title.to_lowercase();
                if lower_title.contains("conflict") || lower_title.contains("copy") {
                    conflict_notes.push(HealthNoteIssue {
                        identifier,
                        title: display_title,
                    });
                }
            } else if archived == 0 {
                let modified_at = modified_at.map(|value| value as i64).unwrap_or_default();
                if max_modified.saturating_sub(modified_at) >= OLD_TRASH_THRESHOLD {
                    old_trashed_notes.push(HealthNoteIssue {
                        identifier,
                        title: display_title,
                    });
                }
            }
        }

        Ok(HealthSummary {
            total_notes,
            duplicate_groups: duplicate_groups.len(),
            duplicate_notes: duplicate_groups.iter().map(|group| group.notes.len()).sum(),
            empty_notes,
            untagged_notes,
            old_trashed_notes,
            large_notes,
            conflict_notes,
        })
    }

    pub fn untagged(&self, search: Option<&str>) -> Result<Vec<NoteListItem>> {
        let like = format!("%{}%", search.unwrap_or_default());
        let mut stmt = self.connection.prepare(
            "select n.ZUNIQUEIDENTIFIER, coalesce(n.ZTITLE, '')
             from ZSFNOTE n
             where n.ZTRASHED = 0
               and n.ZARCHIVED = 0
               and n.ZENCRYPTED = 0
               and n.ZLOCKED = 0
               and n.ZPERMANENTLYDELETED = 0
               and not exists (
                   select 1
                   from Z_5TAGS nt
                   where nt.Z_5NOTES = n.Z_PK
               )
               and (coalesce(n.ZTITLE, '') like ?1 or coalesce(n.ZTEXT, '') like ?1)
             order by lower(coalesce(n.ZTITLE, '')) asc",
        )?;
        let rows = stmt.query_map([like], |row| {
            Ok(NoteListItem {
                identifier: row.get(0)?,
                title: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn todo(&self, search: Option<&str>) -> Result<Vec<NoteListItem>> {
        self.simple_filtered_list("ZTODOINCOMPLETED > 0", search)
    }

    pub fn today(&self, search: Option<&str>) -> Result<Vec<NoteListItem>> {
        self.simple_filtered_list("ZSHOWNINTODAYWIDGET > 0", search)
    }

    pub fn locked(&self, search: Option<&str>) -> Result<Vec<NoteListItem>> {
        let like = format!("%{}%", search.unwrap_or_default());
        let mut stmt = self.connection.prepare(
            "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, '')
             from ZSFNOTE
             where ZLOCKED > 0
               and ZPERMANENTLYDELETED = 0
               and (coalesce(ZTITLE, '') like ?1 or coalesce(ZTEXT, '') like ?1)
             order by lower(coalesce(ZTITLE, '')) asc",
        )?;
        let rows = stmt.query_map([like], |row| {
            Ok(NoteListItem {
                identifier: row.get(0)?,
                title: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn simple_filtered_list(
        &self,
        predicate: &str,
        search: Option<&str>,
    ) -> Result<Vec<NoteListItem>> {
        let like = format!("%{}%", search.unwrap_or_default());
        let sql = format!(
            "select ZUNIQUEIDENTIFIER, coalesce(ZTITLE, '')
             from ZSFNOTE
             where {predicate}
               and ZTRASHED = 0
               and ZARCHIVED = 0
               and ZENCRYPTED = 0
               and ZLOCKED = 0
               and ZPERMANENTLYDELETED = 0
               and (coalesce(ZTITLE, '') like ?1 or coalesce(ZTEXT, '') like ?1)
             order by lower(coalesce(ZTITLE, '')) asc"
        );
        let mut stmt = self.connection.prepare(&sql)?;
        let rows = stmt.query_map([like], |row| {
            Ok(NoteListItem {
                identifier: row.get(0)?,
                title: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn note_tag_map(&self) -> Result<std::collections::BTreeMap<String, Vec<String>>> {
        let mut stmt = self.connection.prepare(
            "select n.ZUNIQUEIDENTIFIER, t.ZTITLE
             from ZSFNOTE n
             left join Z_5TAGS nt on nt.Z_5NOTES = n.Z_PK
             left join ZSFNOTETAG t on t.Z_PK = nt.Z_13TAGS
             where n.ZPERMANENTLYDELETED = 0",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;

        let mut map = std::collections::BTreeMap::<String, Vec<String>>::new();
        for row in rows {
            let (note_id, tag) = row?;
            let entry = map.entry(note_id).or_default();
            if let Some(tag) = tag {
                entry.push(tag);
            }
        }
        Ok(map)
    }

    fn is_trashed(&self, note_id: &str) -> Result<bool> {
        let mut stmt = self
            .connection
            .prepare("select ZTRASHED from ZSFNOTE where ZUNIQUEIDENTIFIER = ?1 limit 1")?;
        let trashed = stmt.query_row([note_id], |row| row.get::<_, i64>(0))?;
        Ok(trashed == 1)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{
        BearDb, DuplicateGroup, DuplicateNote, HealthNoteIssue, HealthSummary, NoteListItem,
        StatsSummary,
    };

    fn test_db() -> BearDb {
        let connection = Connection::open_in_memory().expect("in-memory db");
        connection
            .execute_batch(
                "
                create table ZSFNOTE (
                    Z_PK integer primary key,
                    ZTRASHED integer,
                    ZARCHIVED integer,
                    ZPINNED integer,
                    ZENCRYPTED integer,
                    ZLOCKED integer,
                    ZPERMANENTLYDELETED integer,
                    ZTODOINCOMPLETED integer,
                    ZSHOWNINTODAYWIDGET integer,
                    ZMODIFICATIONDATE integer,
                    ZTITLE text,
                    ZTEXT text,
                    ZUNIQUEIDENTIFIER text
                );
                create table ZSFNOTETAG (
                    Z_PK integer primary key,
                    ZENCRYPTED integer,
                    ZTITLE text
                );
                create table Z_5TAGS (
                    Z_5NOTES integer,
                    Z_13TAGS integer
                );
                insert into ZSFNOTE values
                    (1, 0, 0, 1, 0, 0, 0, 1, 1, 10, 'Alpha', 'alpha body - [ ]', 'NOTE-1'),
                    (2, 0, 1, 0, 0, 0, 0, 0, 0, 20, 'Beta', 'beta body', 'NOTE-2'),
                    (3, 1, 0, 0, 0, 0, 0, 0, 0, 1, 'Trash', 'trashed', 'NOTE-3'),
                    (4, 0, 0, 0, 0, 0, 0, 0, 0, 40, 'Alpha', 'another alpha', 'NOTE-4'),
                    (5, 0, 0, 0, 0, 0, 0, 0, 0, 50, '  ', 'blank title', 'NOTE-5'),
                    (6, 0, 0, 0, 0, 0, 0, 0, 0, 60, 'Conflict copy', '', 'NOTE-6');
                insert into ZSFNOTETAG values
                    (10, 0, 'work'),
                    (11, 0, 'misc');
                insert into Z_5TAGS values
                    (1, 10),
                    (2, 10),
                    (3, 11);
                ",
            )
            .expect("schema should be created");

        BearDb::from_connection(connection)
    }

    #[test]
    fn finds_note_by_title() {
        let db = test_db();
        let note = db
            .find_note(None, Some("Alpha"), false)
            .expect("note should exist");
        assert_eq!(note.text, "another alpha");
    }

    #[test]
    fn searches_non_trashed_notes() {
        let db = test_db();
        let notes = db
            .search(Some("body"), None, false)
            .expect("search should work");
        assert_eq!(
            notes,
            vec![NoteListItem {
                identifier: "NOTE-1".into(),
                title: "Alpha".into()
            }]
        );
    }

    #[test]
    fn lists_notes_for_tag_without_trashed_entries() {
        let db = test_db();
        let notes = db
            .notes_for_tags(&["work".into(), "misc".into()], false)
            .expect("tag lookup should work");
        assert_eq!(
            notes,
            vec![
                NoteListItem {
                    identifier: "NOTE-1".into(),
                    title: "Alpha".into()
                },
                NoteListItem {
                    identifier: "NOTE-2".into(),
                    title: "Beta".into()
                }
            ]
        );
    }

    #[test]
    fn finds_duplicate_titles() {
        let db = test_db();
        let groups = db
            .duplicate_titles()
            .expect("duplicate detection should work");

        assert_eq!(
            groups,
            vec![DuplicateGroup {
                title: "Alpha".into(),
                notes: vec![
                    DuplicateNote {
                        identifier: "NOTE-4".into(),
                        modified_at: Some("40".into()),
                    },
                    DuplicateNote {
                        identifier: "NOTE-1".into(),
                        modified_at: Some("10".into()),
                    },
                ],
            }]
        );
    }

    #[test]
    fn computes_stats_summary() {
        let db = test_db();
        let summary = db.stats_summary().expect("stats should compute");

        assert_eq!(
            summary,
            StatsSummary {
                total_notes: 5,
                pinned_notes: 1,
                tagged_notes: 2,
                archived_notes: 1,
                trashed_notes: 1,
                unique_tags: 2,
                total_words: 11,
                notes_with_todos: 1,
                oldest_modified: Some(10),
                newest_modified: Some(60),
                top_tags: vec![("work".into(), 2)],
            }
        );
    }

    #[test]
    fn computes_health_summary() {
        let db = test_db();
        let summary = db.health_summary().expect("health should compute");

        assert_eq!(
            summary,
            HealthSummary {
                total_notes: 5,
                duplicate_groups: 1,
                duplicate_notes: 2,
                empty_notes: vec![HealthNoteIssue {
                    identifier: "NOTE-6".into(),
                    title: "Conflict copy".into(),
                }],
                untagged_notes: 3,
                old_trashed_notes: vec![HealthNoteIssue {
                    identifier: "NOTE-3".into(),
                    title: "Trash".into(),
                }],
                large_notes: vec![],
                conflict_notes: vec![HealthNoteIssue {
                    identifier: "NOTE-6".into(),
                    title: "Conflict copy".into(),
                }],
            }
        );
    }
}
