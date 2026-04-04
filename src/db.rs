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
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{BearDb, NoteListItem};

    fn test_db() -> BearDb {
        let connection = Connection::open_in_memory().expect("in-memory db");
        connection
            .execute_batch(
                "
                create table ZSFNOTE (
                    Z_PK integer primary key,
                    ZTRASHED integer,
                    ZARCHIVED integer,
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
                    (1, 0, 0, 0, 0, 0, 1, 1, 10, 'Alpha', 'alpha body', 'NOTE-1'),
                    (2, 0, 0, 0, 0, 0, 0, 0, 20, 'Beta', 'beta body', 'NOTE-2'),
                    (3, 1, 0, 0, 0, 0, 0, 0, 30, 'Trash', 'trashed', 'NOTE-3');
                insert into ZSFNOTETAG values
                    (10, 0, 'work'),
                    (11, 0, 'misc');
                insert into Z_5TAGS values
                    (1, 10),
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
        assert_eq!(note.text, "alpha body");
    }

    #[test]
    fn searches_non_trashed_notes() {
        let db = test_db();
        let notes = db
            .search(Some("body"), None, false)
            .expect("search should work");
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
    fn lists_notes_for_tag_without_trashed_entries() {
        let db = test_db();
        let notes = db
            .notes_for_tags(&["work".into(), "misc".into()], false)
            .expect("tag lookup should work");
        assert_eq!(
            notes,
            vec![NoteListItem {
                identifier: "NOTE-1".into(),
                title: "Alpha".into()
            }]
        );
    }
}
