use serde::{Deserialize, Serialize};

/// A Bear note, populated from ZSFNOTE + related tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    /// ZUNIQUEIDENTIFIER: external ID used in CLI commands.
    pub id: String,
    /// Z_PK: internal SQLite primary key, used only for joins.
    #[serde(skip)]
    pub pk: i64,
    /// ZTITLE: empty string when the column is NULL.
    pub title: String,
    /// ZTEXT: empty string when the note is encrypted or the column is NULL.
    pub text: String,
    /// Tag names, sorted alphabetically, from the Z_5TAGS join.
    pub tags: Vec<String>,
    /// ZCREATIONDATE converted to Unix timestamp (seconds).
    pub created: i64,
    /// ZMODIFICATIONDATE converted to Unix timestamp (seconds).
    pub modified: i64,
    pub trashed: bool,
    pub archived: bool,
    pub pinned: bool,
    pub locked: bool,
    pub encrypted: bool,
    pub has_images: bool,
    pub has_files: bool,
    pub has_source_code: bool,
    pub todo_completed: i64,
    pub todo_incompleted: i64,
    /// Populated on demand (e.g. for `show --fields attachments`).
    pub attachments: Vec<Attachment>,
    /// Pin contexts: "global" for the All Notes pin, or a tag name.
    /// Populated on demand.
    pub pinned_in_tags: Vec<String>,
}

impl Note {
    /// SHA-256 hex digest of the note text.
    pub fn hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.text.as_bytes());
        format!("{digest:x}")
    }

    /// Byte length of the note text.
    pub fn length(&self) -> i64 {
        self.text.len() as i64
    }
}

/// A Bear tag, from ZSFNOTETAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    /// ZTITLE: full tag path, e.g. "work/meetings".
    pub name: String,
    /// Z_PK: internal primary key.
    #[serde(skip)]
    pub pk: i64,
}

/// A Bear note attachment, from ZSFNOTEFILE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// ZFILENAME.
    pub filename: String,
    /// ZFILESIZE in bytes.
    pub size: i64,
    /// ZUNIQUEIDENTIFIER: used to locate the file on disk.
    pub uuid: String,
}

/// A pin record: a note pinned in a specific context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinRecord {
    /// The note's ZUNIQUEIDENTIFIER.
    pub note_id: String,
    /// "global" for All Notes pin; tag name for tag-scoped pin.
    pub pin: String,
}

/// Where to insert content relative to existing body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPosition {
    Beginning,
    End,
}

/// Tag position preference from Bear settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TagPosition {
    Top,
    #[default]
    Bottom,
}

/// Sort field for note lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Pinned,
    Modified,
    Created,
    Title,
}

impl SortField {
    pub fn sql_column(&self) -> &'static str {
        match self {
            SortField::Pinned => "n.ZPINNED",
            SortField::Modified => "n.ZMODIFICATIONDATE",
            SortField::Created => "n.ZCREATIONDATE",
            SortField::Title => "n.ZTITLE",
        }
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}
