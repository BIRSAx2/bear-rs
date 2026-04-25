use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::frontmatter::{FrontMatter, parse_front_matter};
use crate::model::Note;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportNote {
    pub identifier: String,
    pub title: String,
    pub text: String,
    pub pinned: bool,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub tags: Vec<String>,
}

impl From<Note> for ExportNote {
    fn from(n: Note) -> Self {
        ExportNote {
            identifier: n.id,
            title: n.title,
            text: n.text,
            pinned: n.pinned,
            created_at: Some(n.created),
            modified_at: Some(n.modified),
            tags: n.tags,
        }
    }
}

impl From<&Note> for ExportNote {
    fn from(n: &Note) -> Self {
        ExportNote {
            identifier: n.id.clone(),
            title: n.title.clone(),
            text: n.text.clone(),
            pinned: n.pinned,
            created_at: Some(n.created),
            modified_at: Some(n.modified),
            tags: n.tags.clone(),
        }
    }
}

pub fn export_notes(
    output_dir: &Path,
    notes: &[ExportNote],
    include_frontmatter: bool,
    by_tag: bool,
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let mut written = Vec::new();
    for note in notes {
        let target = output_dir.join(export_path_for(note, by_tag));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let contents = render_exported_note(note, include_frontmatter);
        fs::write(&target, contents)
            .with_context(|| format!("failed to write {}", target.display()))?;
        written.push(target);
    }

    Ok(written)
}

pub fn export_path_for(note: &ExportNote, by_tag: bool) -> PathBuf {
    let filename = format!("{}.md", sanitize_filename(&display_title(note)));
    if by_tag {
        if let Some(tag) = note.tags.first() {
            return PathBuf::from(sanitize_path_segment(tag)).join(filename);
        }
    }
    PathBuf::from(filename)
}

pub fn render_exported_note(note: &ExportNote, include_frontmatter: bool) -> String {
    if !include_frontmatter {
        return note.text.clone();
    }

    let (frontmatter, body) = parse_front_matter(&note.text);
    let mut merged = frontmatter.unwrap_or_else(|| FrontMatter::new(Vec::new()));
    merged.merge_missing_from(&generated_frontmatter(note));
    merged.to_note_text(&body)
}

fn generated_frontmatter(note: &ExportNote) -> FrontMatter {
    let mut fields = vec![
        ("title".to_string(), display_title(note)),
        ("id".to_string(), note.identifier.clone()),
        (
            "tags".to_string(),
            format!(
                "[{}]",
                note.tags
                    .iter()
                    .map(|tag| format!("\"{}\"", tag.replace('"', "\\\"")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        ("pinned".to_string(), note.pinned.to_string()),
    ];

    if let Some(created) = note.created_at {
        fields.push(("created".to_string(), created.to_string()));
    }
    if let Some(modified) = note.modified_at {
        fields.push(("modified".to_string(), modified.to_string()));
    }

    FrontMatter::new(fields)
}

fn display_title(note: &ExportNote) -> String {
    let title = note.title.trim();
    if title.is_empty() {
        note.identifier.clone()
    } else {
        title.to_string()
    }
}

pub fn sanitize_filename(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ if ch.is_control() => ' ',
            _ => ch,
        })
        .collect::<String>();
    let collapsed = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}

fn sanitize_path_segment(value: &str) -> String {
    sanitize_filename(&value.replace('/', "-"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ExportNote, export_path_for, render_exported_note, sanitize_filename};

    fn sample_note() -> ExportNote {
        ExportNote {
            identifier: "NOTE-1".into(),
            title: "Hello / Rust".into(),
            text: "# Hello\n\nBody".into(),
            pinned: true,
            created_at: Some(10),
            modified_at: Some(20),
            tags: vec!["work/project".into(), "rust".into()],
        }
    }

    #[test]
    fn sanitizes_filenames() {
        assert_eq!(sanitize_filename(" Hello:/Rust? "), "Hello--Rust-");
    }

    #[test]
    fn merges_generated_frontmatter_without_overwriting_user_fields() {
        let mut note = sample_note();
        note.text = "---\ntitle: Custom\ntags: [\"mine\"]\n---\n# Hello\n\nBody".into();

        let rendered = render_exported_note(&note, true);

        assert!(rendered.contains("title: Custom"));
        assert!(rendered.contains("tags: [\"mine\"]"));
        assert!(rendered.contains("id: NOTE-1"));
        assert!(rendered.contains("pinned: true"));
    }

    #[test]
    fn exports_by_first_tag_path() {
        let note = sample_note();
        assert_eq!(
            export_path_for(&note, true),
            PathBuf::from("work-project/Hello - Rust.md")
        );
    }
}
