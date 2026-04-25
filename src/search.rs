//! Bear search-query parser. Converts a query string into a SQL WHERE
//! clause fragment and bound parameters.
//!
//! Supported tokens:
//!   bare term          -> ZTEXT/ZTITLE LIKE '%term%'
//!   "exact phrase"     -> ZTEXT/ZTITLE LIKE '%exact phrase%'
//!   -negation          -> NOT (ZTEXT/ZTITLE LIKE '%negation%')
//!   #tag               -> note has tag
//!   !#tag              -> note has tag (exact, alias)
//!   @today             -> modified today (local midnight)
//!   @yesterday         -> modified yesterday
//!   @lastNdays         -> modified in last N days
//!   @date(YYYY-MM-DD)  -> modified on that date
//!   @ctoday            -> created today
//!   @createdNdays      -> created in last N days
//!   @cdate(YYYY-MM-DD) -> created on that date
//!   @todo              -> ZTODOINCOMPLETED > 0
//!   @done              -> ZTODOCOMPLETED > 0
//!   @task              -> alias for @todo
//!   @tagged            -> has at least one tag
//!   @untagged          -> has no tags
//!   @pinned            -> ZPINNED = 1
//!   @images            -> ZHASIMAGES = 1
//!   @files             -> ZHASFILES = 1
//!   @attachments       -> ZHASIMAGES = 1 OR ZHASFILES = 1
//!   @code              -> ZHASSOURCECODE = 1
//!   @locked            -> ZLOCKED = 1
//!   @title <term>      -> next bare term matched against ZTITLE only
//!   @untitled          -> ZTITLE IS NULL OR ZTITLE = ''
//!   @empty             -> ZTEXT IS NULL OR ZTEXT = ''
//!
//! Unsupported tokens (@ocr, @wikilinks, @backlinks, @readonly) are
//! silently skipped with a warning to stderr.

use chrono::{Duration, Local, NaiveDate, TimeZone};

/// Result of parsing a Bear query string.
pub struct ParsedQuery {
    /// SQL fragments joined with AND, ready to embed in a WHERE clause.
    /// Each `?` placeholder corresponds to an entry in `params`.
    pub clauses: Vec<String>,
    /// Bound parameter values (all strings for rusqlite).
    pub params: Vec<String>,
    /// Extra JOIN clauses required (e.g. for tag filters).
    pub joins: Vec<String>,
}

impl ParsedQuery {
    fn new() -> Self {
        ParsedQuery {
            clauses: Vec::new(),
            params: Vec::new(),
            joins: Vec::new(),
        }
    }

    fn push_like(&mut self, col: &str, value: &str) {
        self.clauses.push(format!("{col} LIKE ? ESCAPE '\\'"));
        self.params.push(format!("%{}%", escape_like(value)));
    }

    fn push_not_like(&mut self, value: &str) {
        self.clauses.push(
            "(n.ZTEXT NOT LIKE ? ESCAPE '\\' AND n.ZTITLE NOT LIKE ? ESCAPE '\\')".to_string(),
        );
        let pat = format!("%{}%", escape_like(value));
        self.params.push(pat.clone());
        self.params.push(pat);
    }

    fn push_text_or_title_like(&mut self, value: &str) {
        self.clauses
            .push("(n.ZTEXT LIKE ? ESCAPE '\\' OR n.ZTITLE LIKE ? ESCAPE '\\')".to_string());
        let pat = format!("%{}%", escape_like(value));
        self.params.push(pat.clone());
        self.params.push(pat);
    }

    fn push_tag(&mut self, tag: &str) {
        let alias = format!("tag_{}", self.joins.len());
        self.joins.push(format!(
            "JOIN Z_5TAGS {a}t ON {a}t.Z_5NOTES = n.Z_PK \
             JOIN ZSFNOTETAG {a}n ON {a}n.Z_PK = {a}t.Z_13TAGS AND {a}n.ZTITLE = ?",
            a = alias
        ));
        self.params.push(tag.to_string());
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Parse a Bear query string into SQL fragments.
pub fn parse_query(query: &str) -> ParsedQuery {
    let mut pq = ParsedQuery::new();
    let mut chars = query.chars().peekable();
    let mut title_only = false;

    while chars.peek().is_some() {
        // skip leading whitespace
        while chars.peek().map(|c| c.is_whitespace()) == Some(true) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let ch = *chars.peek().unwrap();

        if ch == '"' {
            // quoted phrase
            chars.next();
            let phrase: String = chars.by_ref().take_while(|&c| c != '"').collect();
            if !phrase.is_empty() {
                if title_only {
                    pq.push_like("n.ZTITLE", &phrase);
                    title_only = false;
                } else {
                    pq.push_text_or_title_like(&phrase);
                }
            }
        } else if ch == '-' {
            // negation: -term or -"phrase"
            chars.next();
            let term = read_token(&mut chars);
            let term = term.trim_matches('"');
            if !term.is_empty() {
                pq.push_not_like(term);
            }
        } else if ch == '#' || (ch == '!' && chars.clone().nth(1) == Some('#')) {
            // tag filter: #tag or !#tag
            if ch == '!' {
                chars.next(); // consume '!'
            }
            chars.next(); // consume '#'
            let tag: String = chars.by_ref().take_while(|&c| !c.is_whitespace()).collect();
            if !tag.is_empty() {
                pq.push_tag(&tag);
            }
        } else if ch == '@' {
            chars.next(); // consume '@'
            let token: String = chars.by_ref().take_while(|&c| !c.is_whitespace()).collect();
            match token.to_lowercase().as_str() {
                "today" => {
                    let ts = local_midnight_coredata(0);
                    pq.clauses.push("n.ZMODIFICATIONDATE >= ?".to_string());
                    pq.params.push(ts.to_string());
                }
                "yesterday" => {
                    let start = local_midnight_coredata(-1);
                    let end = local_midnight_coredata(0);
                    pq.clauses
                        .push("(n.ZMODIFICATIONDATE >= ? AND n.ZMODIFICATIONDATE < ?)".to_string());
                    pq.params.push(start.to_string());
                    pq.params.push(end.to_string());
                }
                "ctoday" => {
                    let ts = local_midnight_coredata(0);
                    pq.clauses.push("n.ZCREATIONDATE >= ?".to_string());
                    pq.params.push(ts.to_string());
                }
                "untitled" => {
                    pq.clauses
                        .push("(n.ZTITLE IS NULL OR n.ZTITLE = '')".to_string());
                }
                "empty" => {
                    pq.clauses
                        .push("(n.ZTEXT IS NULL OR n.ZTEXT = '')".to_string());
                }
                "todo" | "task" => {
                    pq.clauses.push("n.ZTODOINCOMPLETED > 0".to_string());
                }
                "done" => {
                    pq.clauses.push("n.ZTODOCOMPLETED > 0".to_string());
                }
                "tagged" => {
                    pq.clauses
                        .push("EXISTS (SELECT 1 FROM Z_5TAGS WHERE Z_5NOTES = n.Z_PK)".to_string());
                }
                "untagged" => {
                    pq.clauses.push(
                        "NOT EXISTS (SELECT 1 FROM Z_5TAGS WHERE Z_5NOTES = n.Z_PK)".to_string(),
                    );
                }
                "pinned" => {
                    pq.clauses.push("n.ZPINNED = 1".to_string());
                }
                "images" => {
                    pq.clauses.push("n.ZHASIMAGES = 1".to_string());
                }
                "files" => {
                    pq.clauses.push("n.ZHASFILES = 1".to_string());
                }
                "attachments" => {
                    pq.clauses
                        .push("(n.ZHASIMAGES = 1 OR n.ZHASFILES = 1)".to_string());
                }
                "code" => {
                    pq.clauses.push("n.ZHASSOURCECODE = 1".to_string());
                }
                "locked" => {
                    pq.clauses.push("n.ZLOCKED = 1".to_string());
                }
                "title" => {
                    title_only = true;
                }
                t if t.starts_with("last") && t.ends_with("days") => {
                    if let Ok(n) = t[4..t.len() - 4].parse::<i64>() {
                        let ts = local_midnight_coredata(-n);
                        pq.clauses.push("n.ZMODIFICATIONDATE >= ?".to_string());
                        pq.params.push(ts.to_string());
                    }
                }
                t if t.starts_with("created") && t.ends_with("days") => {
                    if let Ok(n) = t[7..t.len() - 4].parse::<i64>() {
                        let ts = local_midnight_coredata(-n);
                        pq.clauses.push("n.ZCREATIONDATE >= ?".to_string());
                        pq.params.push(ts.to_string());
                    }
                }
                t if t.starts_with("date(") && t.ends_with(')') => {
                    let date_str = &t[5..t.len() - 1];
                    if let Some((start, end)) = parse_date_range_coredata(date_str) {
                        pq.clauses.push(
                            "(n.ZMODIFICATIONDATE >= ? AND n.ZMODIFICATIONDATE < ?)".to_string(),
                        );
                        pq.params.push(start.to_string());
                        pq.params.push(end.to_string());
                    }
                }
                t if t.starts_with("cdate(") && t.ends_with(')') => {
                    let date_str = &t[6..t.len() - 1];
                    if let Some((start, end)) = parse_date_range_coredata(date_str) {
                        pq.clauses
                            .push("(n.ZCREATIONDATE >= ? AND n.ZCREATIONDATE < ?)".to_string());
                        pq.params.push(start.to_string());
                        pq.params.push(end.to_string());
                    }
                }
                // unsupported: silently skip
                "ocr" | "wikilinks" | "backlinks" | "readonly" => {
                    eprintln!("warning: @{token} is not supported, skipping");
                }
                _ => {
                    eprintln!("warning: unknown token @{token}, skipping");
                }
            }
        } else {
            // bare term
            let term = read_token(&mut chars);
            if !term.is_empty() {
                if title_only {
                    pq.push_like("n.ZTITLE", &term);
                    title_only = false;
                } else {
                    pq.push_text_or_title_like(&term);
                }
            }
        }
    }

    pq
}

fn read_token(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        s.push(c);
        chars.next();
    }
    s
}

/// CoreData timestamp for local midnight N days from today (negative = past).
fn local_midnight_coredata(days_offset: i64) -> f64 {
    let today = Local::now().date_naive();
    let target = today + Duration::days(days_offset);
    let midnight = Local
        .from_local_datetime(&target.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    crate::db::unix_to_coredata(midnight)
}

/// Parse "YYYY-MM-DD" into a [start, end) CoreData range (one full day).
fn parse_date_range_coredata(s: &str) -> Option<(f64, f64)> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let start_unix = Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .single()?
        .timestamp();
    let end_unix = start_unix + 86_400;
    Some((
        crate::db::unix_to_coredata(start_unix),
        crate::db::unix_to_coredata(end_unix),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_query() {
        let pq = parse_query("");
        assert!(pq.clauses.is_empty());
        assert!(pq.params.is_empty());
    }

    #[test]
    fn parse_bare_term() {
        let pq = parse_query("meeting");
        assert_eq!(pq.clauses.len(), 1);
        assert!(pq.clauses[0].contains("LIKE"));
        assert_eq!(pq.params.len(), 2); // text + title
    }

    #[test]
    fn parse_negation() {
        let pq = parse_query("-draft");
        assert_eq!(pq.clauses.len(), 1);
        assert!(pq.clauses[0].contains("NOT LIKE"));
    }

    #[test]
    fn parse_at_todo() {
        let pq = parse_query("@todo");
        assert_eq!(pq.clauses.len(), 1);
        assert_eq!(pq.clauses[0], "n.ZTODOINCOMPLETED > 0");
        assert!(pq.params.is_empty());
    }

    #[test]
    fn parse_tag() {
        let pq = parse_query("#work");
        assert!(pq.joins.len() == 1);
        assert!(pq.joins[0].contains("ZSFNOTETAG"));
        assert_eq!(pq.params[0], "work");
    }

    #[test]
    fn parse_combined() {
        let pq = parse_query("meeting #work @today");
        assert_eq!(pq.joins.len(), 1);
        assert_eq!(pq.clauses.len(), 2); // text/title LIKE + ZMODIFICATIONDATE >=
    }

    #[test]
    fn escape_like_special_chars() {
        assert_eq!(escape_like("50%"), "50\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
    }
}
