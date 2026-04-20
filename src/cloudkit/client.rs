use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use uuid::Uuid;

use super::auth::AuthConfig;
use super::models::*;
use super::vector_clock;

pub const API_TOKEN: &str = "ce59f955ec47e744f720aa1d2816a4e985e472d8b859b6c7a47b81fd36646307";
const BASE_URL: &str =
    "https://api.apple-cloudkit.com/database/1/iCloud.net.shinyfrog.bear/production/private";
const DEVICE_NAME: &str = "Bear CLI";

pub struct CloudKitClient {
    http: Client,
    auth: AuthConfig,
}

impl CloudKitClient {
    pub fn new(auth: AuthConfig) -> Result<Self> {
        let http = Client::builder()
            .user_agent("bear-cli/0.3.0")
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { http, auth })
    }

    fn url(&self, path: &str) -> String {
        // Apple CloudKit decodes '+' as space — must percent-encode it explicitly.
        let token = self.auth.ck_web_auth_token.replace('+', "%2B");
        let api = API_TOKEN.replace('+', "%2B");
        format!("{BASE_URL}{path}?ckWebAuthToken={token}&ckAPIToken={api}")
    }

    fn post<Req, Res>(&self, path: &str, body: &Req) -> Result<Res>
    where
        Req: serde::Serialize,
        Res: serde::de::DeserializeOwned,
    {
        let resp = self
            .http
            .post(self.url(path))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .with_context(|| format!("HTTP POST {path} failed"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("CloudKit {path} returned {status}: {body}");
        }

        resp.json::<Res>()
            .with_context(|| format!("failed to parse response from {path}"))
    }

    pub fn modify(&self, ops: Vec<ModifyOperation>) -> Result<Vec<CkRecord>> {
        let req = ModifyRequest {
            operations: ops,
            zone_id: ZoneId::default(),
        };
        let resp: ModifyResponse = self.post("/records/modify", &req)?;

        // Surface per-record errors
        for rec in &resp.records {
            if let Some(code) = &rec.server_error_code {
                bail!(
                    "CloudKit record error on {}: {} — {}",
                    rec.record_name,
                    code,
                    rec.reason.as_deref().unwrap_or("")
                );
            }
        }
        Ok(resp.records)
    }

    pub fn query(&self, req: QueryRequest) -> Result<QueryResponse> {
        self.post("/records/query", &req)
    }

    pub fn list_notes(
        &self,
        include_trashed: bool,
        include_archived: bool,
        limit: Option<usize>,
    ) -> Result<Vec<CkRecord>> {
        let mut filters = Vec::new();
        if !include_trashed {
            filters.push(CkFilter {
                field_name: "trashed".into(),
                comparator: "EQUALS".into(),
                field_value: CkFilterValue {
                    value: 0.into(),
                    kind: "INT64".into(),
                },
            });
        }
        if !include_archived {
            filters.push(CkFilter {
                field_name: "archived".into(),
                comparator: "EQUALS".into(),
                field_value: CkFilterValue {
                    value: 0.into(),
                    kind: "INT64".into(),
                },
            });
        }

        let mut records = Vec::new();
        let mut continuation_marker = None;

        loop {
            let remaining = limit.map(|n| n.saturating_sub(records.len()));
            if matches!(remaining, Some(0)) {
                break;
            }

            let req = QueryRequest {
                zone_id: ZoneId::default(),
                query: CkQuery {
                    record_type: "SFNote".into(),
                    filter_by: filters.clone(),
                    sort_by: vec![CkSort {
                        field_name: "sf_modificationDate".into(),
                        ascending: false,
                    }],
                },
                results_limit: Some(remaining.unwrap_or(200).min(200)),
                desired_keys: Some(vec![
                    "uniqueIdentifier".into(),
                    "title".into(),
                    "textADP".into(),
                    "subtitleADP".into(),
                    "sf_creationDate".into(),
                    "sf_modificationDate".into(),
                    "trashed".into(),
                    "archived".into(),
                    "pinned".into(),
                    "locked".into(),
                    "encrypted".into(),
                    "todoCompleted".into(),
                    "todoIncompleted".into(),
                    "tagsStrings".into(),
                    "conflictUniqueIdentifier".into(),
                ]),
                continuation_marker,
            };

            let resp = self.query(req)?;
            records.extend(resp.records);
            continuation_marker = resp.continuation_marker;

            if continuation_marker.is_none() {
                break;
            }
        }

        Ok(records)
    }

    pub fn list_tags(&self) -> Result<Vec<CkRecord>> {
        let mut records = Vec::new();
        let mut marker = None;

        loop {
            let resp = self.query(QueryRequest {
                zone_id: ZoneId::default(),
                query: CkQuery {
                    record_type: "SFNoteTag".into(),
                    filter_by: vec![],
                    sort_by: vec![CkSort {
                        field_name: "name".into(),
                        ascending: true,
                    }],
                },
                results_limit: Some(500),
                desired_keys: Some(vec!["name".into(), "sf_modificationDate".into()]),
                continuation_marker: marker,
            })?;
            records.extend(resp.records);
            marker = resp.continuation_marker;
            if marker.is_none() {
                break;
            }
        }

        Ok(records)
    }

    pub fn lookup(&self, record_names: &[&str]) -> Result<Vec<CkRecord>> {
        let req = LookupRequest {
            records: record_names
                .iter()
                .map(|n| LookupRecord {
                    record_name: n.to_string(),
                })
                .collect(),
            zone_id: ZoneId::default(),
        };
        let resp: LookupResponse = self.post("/records/lookup", &req)?;
        Ok(resp.records)
    }

    pub fn fetch_note(&self, record_name: &str) -> Result<CkRecord> {
        let records = self.lookup(&[record_name])?;
        records
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("note not found: {record_name}"))
    }

    /// Upload a file to CloudKit asset storage. Returns the receipt to embed in a record field.
    pub fn upload_asset(
        &self,
        record_name: &str,
        record_type: &str,
        data: &[u8],
        mime_type: &str,
    ) -> Result<AssetReceipt> {
        // Phase 1: request a signed upload URL
        let req = AssetUploadRequest {
            zone_id: ZoneId::default(),
            tokens: vec![AssetToken {
                record_type: record_type.to_string(),
                record_name: record_name.to_string(),
                field_name: "file".to_string(),
            }],
        };
        let resp: AssetUploadResponse = self.post("/assets/upload", &req)?;
        let token = resp
            .tokens
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no upload token returned"))?;

        // Phase 2: upload raw bytes to the signed URL
        let upload_resp = self
            .http
            .post(&token.url)
            .header("Content-Type", mime_type)
            .body(data.to_vec())
            .send()
            .context("asset upload POST failed")?;

        let status = upload_resp.status();
        if !status.is_success() {
            let body = upload_resp.text().unwrap_or_default();
            bail!("asset upload returned {status}: {body}");
        }

        let result: AssetUploadResult = upload_resp
            .json()
            .context("failed to parse upload receipt")?;
        Ok(result.single_file)
    }

    /// Create a brand-new note. Returns the created record.
    pub fn create_note(
        &self,
        text: &str,
        tag_uuids: Vec<String>,
        tag_names: Vec<String>,
    ) -> Result<CkRecord> {
        let now_ms = now_ms();
        let note_uuid = Uuid::new_v4().to_string().to_uppercase();
        let title = extract_title(text);
        let subtitle = extract_subtitle(text);
        let clock = vector_clock::increment(None, DEVICE_NAME)?;

        let mut fields: Fields = HashMap::new();
        fields.insert("uniqueIdentifier".into(), CkField::string(&note_uuid));
        fields.insert("title".into(), CkField::string(&title));
        fields.insert("subtitle".into(), CkField::string_null());
        fields.insert("subtitleADP".into(), CkField::string_encrypted(&subtitle));
        fields.insert("textADP".into(), CkField::string_encrypted(text));
        fields.insert("text".into(), CkField::string_null());
        fields.insert("tags".into(), CkField::string_list(tag_uuids));
        fields.insert("tagsStrings".into(), CkField::string_list(tag_names));
        fields.insert("files".into(), CkField::string_list(vec![]));
        fields.insert("linkedBy".into(), CkField::string_list(vec![]));
        fields.insert("linkingTo".into(), CkField::string_list(vec![]));
        fields.insert("pinnedInTagsStrings".into(), CkField::string_list_null());
        fields.insert("vectorClock".into(), CkField::bytes(&clock));
        fields.insert("lastEditingDevice".into(), CkField::string(DEVICE_NAME));
        fields.insert("version".into(), CkField::int64(3));
        fields.insert("encrypted".into(), CkField::int64(0));
        fields.insert("locked".into(), CkField::int64(0));
        fields.insert("trashed".into(), CkField::int64(0));
        fields.insert("archived".into(), CkField::int64(0));
        fields.insert("pinned".into(), CkField::int64(0));
        fields.insert("hasImages".into(), CkField::int64(0));
        fields.insert("hasFiles".into(), CkField::int64(0));
        fields.insert("hasSourceCode".into(), CkField::int64(0));
        fields.insert("todoCompleted".into(), CkField::int64(0));
        fields.insert("todoIncompleted".into(), CkField::int64(0));
        fields.insert("sf_creationDate".into(), CkField::timestamp(now_ms));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms + 1));
        fields.insert("trashedDate".into(), CkField::timestamp_null());
        fields.insert("pinnedDate".into(), CkField::timestamp_null());
        fields.insert("archivedDate".into(), CkField::timestamp_null());
        fields.insert("lockedDate".into(), CkField::timestamp_null());
        fields.insert("conflictUniqueIdentifier".into(), CkField::string_null());
        fields.insert(
            "conflictUniqueIdentifierDate".into(),
            CkField::timestamp_null(),
        );
        fields.insert("encryptedData".into(), CkField::string_null());

        let op = ModifyOperation {
            operation_type: "create".into(),
            record: CkRecord {
                record_name: note_uuid.clone(),
                record_type: "SFNote".into(),
                fields,
                record_change_tag: None,
                deleted: false,
                server_error_code: None,
                reason: None,
            },
        };
        let records = self.modify(vec![op])?;
        records
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no record returned from create"))
    }

    /// Update a note's text. Fetches the current record first to obtain the recordChangeTag
    /// and existing vector clock, then writes back the updated content.
    pub fn update_note_text(&self, record_name: &str, new_text: &str) -> Result<CkRecord> {
        let current = self.fetch_note(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note {record_name} has no recordChangeTag"))?;
        let existing_clock = current.str_field("vectorClock");
        let clock = vector_clock::increment(existing_clock, DEVICE_NAME)?;

        let title = extract_title(new_text);
        let subtitle = extract_subtitle(new_text);
        let todo_counts = count_todos(new_text);
        let now_ms = now_ms();

        let mut fields: Fields = HashMap::new();
        fields.insert("textADP".into(), CkField::string_encrypted(new_text));
        fields.insert("text".into(), CkField::string_null());
        fields.insert("title".into(), CkField::string(&title));
        fields.insert("subtitleADP".into(), CkField::string_encrypted(&subtitle));
        fields.insert("subtitle".into(), CkField::string_null());
        fields.insert("vectorClock".into(), CkField::bytes(&clock));
        fields.insert("lastEditingDevice".into(), CkField::string(DEVICE_NAME));
        fields.insert("version".into(), CkField::int64(3));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms));
        fields.insert("todoCompleted".into(), CkField::int64(todo_counts.0));
        fields.insert("todoIncompleted".into(), CkField::int64(todo_counts.1));
        fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(current.str_field("uniqueIdentifier").unwrap_or(record_name)),
        );

        let op = ModifyOperation {
            operation_type: "update".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNote".into(),
                fields,
                record_change_tag: Some(change_tag),
                deleted: false,
                server_error_code: None,
                reason: None,
            },
        };
        let records = self.modify(vec![op])?;
        records
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no record returned from update"))
    }

    /// Attach a file to a note. Uploads the asset, creates the file record, and
    /// updates the note's markdown — all in one atomic `records/modify` call.
    pub fn attach_file(
        &self,
        note_record_name: &str,
        filename: &str,
        data: &[u8],
        position: AttachPosition,
    ) -> Result<()> {
        // Determine record type and mime type from extension
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_image = matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "tiff"
        );
        let record_type = if is_image {
            "SFNoteImage"
        } else {
            "SFNoteGenericFile"
        };
        let mime_type = mime_for_ext(&ext);

        // Upload asset (2-phase)
        let file_record_uuid = Uuid::new_v4().to_string().to_uppercase();
        let receipt = self.upload_asset(&file_record_uuid, record_type, data, &mime_type)?;
        let file_size = receipt.size;

        // Fetch current note to get change tag and existing content
        let note = self.fetch_note(note_record_name)?;
        let change_tag = note
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note has no recordChangeTag"))?;
        let existing_clock = note.str_field("vectorClock");
        let clock = vector_clock::increment(existing_clock, DEVICE_NAME)?;

        // Build updated note text with file embedded
        let current_text = note.str_field("textADP").unwrap_or("").to_string();
        let embed = if is_image {
            format!("![{filename}]({filename})<!-- {{\"preview\":\"true\",\"embed\":\"true\"}} -->")
        } else {
            format!("[{filename}]({filename})<!-- {{\"preview\":\"true\",\"embed\":\"true\"}} -->")
        };
        let new_text = match position {
            AttachPosition::Append => format!("{current_text}\n{embed}"),
            AttachPosition::Prepend => {
                // Insert after the first heading line if present
                let mut lines = current_text.lines();
                let first = lines.next().unwrap_or("").to_string();
                let rest: String = lines.collect::<Vec<_>>().join("\n");
                if first.starts_with('#') {
                    format!("{first}\n{embed}\n{rest}")
                } else {
                    format!("{embed}\n{current_text}")
                }
            }
        };

        // Update files list on the note
        let mut files_list: Vec<String> = note
            .fields
            .get("files")
            .and_then(|f| f.value.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        files_list.push(file_record_uuid.clone());

        let has_images = note.i64_field("hasImages").unwrap_or(0) + if is_image { 1 } else { 0 };
        let has_files = note.i64_field("hasFiles").unwrap_or(0) + if is_image { 0 } else { 1 };
        let now_ms = now_ms();
        let title = extract_title(&new_text);
        let subtitle = extract_subtitle(&new_text);
        let todo_counts = count_todos(&new_text);

        // Build file record fields
        let mut file_fields: Fields = std::collections::HashMap::new();
        file_fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(&file_record_uuid),
        );
        file_fields.insert("filenameADP".into(), CkField::string_encrypted(filename));
        file_fields.insert("normalizedFileExtension".into(), CkField::string(&ext));
        file_fields.insert("fileSize".into(), CkField::int64(file_size));
        file_fields.insert("file".into(), CkField::asset_id(receipt));
        file_fields.insert(
            "noteUniqueIdentifier".into(),
            CkField::string(
                note.str_field("uniqueIdentifier")
                    .unwrap_or(note_record_name),
            ),
        );
        file_fields.insert("index".into(), CkField::int64(0));
        file_fields.insert("unused".into(), CkField::int64(0));
        file_fields.insert("uploaded".into(), CkField::int64(1));
        file_fields.insert("uploadedDate".into(), CkField::timestamp(now_ms));
        file_fields.insert("insertionDate".into(), CkField::timestamp(now_ms));
        file_fields.insert("encrypted".into(), CkField::int64(0));
        file_fields.insert(
            "animated".into(),
            CkField::int64(if ext == "gif" { 1 } else { 0 }),
        );
        file_fields.insert("version".into(), CkField::int64(3));
        file_fields.insert("sf_creationDate".into(), CkField::timestamp(now_ms));
        file_fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms + 1));

        // Build updated note fields
        let mut note_fields: Fields = std::collections::HashMap::new();
        note_fields.insert("textADP".into(), CkField::string_encrypted(&new_text));
        note_fields.insert("text".into(), CkField::string_null());
        note_fields.insert("title".into(), CkField::string(&title));
        note_fields.insert("subtitleADP".into(), CkField::string_encrypted(&subtitle));
        note_fields.insert("subtitle".into(), CkField::string_null());
        note_fields.insert("files".into(), CkField::string_list(files_list));
        note_fields.insert("hasImages".into(), CkField::int64(has_images));
        note_fields.insert("hasFiles".into(), CkField::int64(has_files));
        note_fields.insert("vectorClock".into(), CkField::bytes(&clock));
        note_fields.insert("lastEditingDevice".into(), CkField::string(DEVICE_NAME));
        note_fields.insert("version".into(), CkField::int64(3));
        note_fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms + 2));
        note_fields.insert("todoCompleted".into(), CkField::int64(todo_counts.0));
        note_fields.insert("todoIncompleted".into(), CkField::int64(todo_counts.1));
        note_fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(
                note.str_field("uniqueIdentifier")
                    .unwrap_or(note_record_name),
            ),
        );

        // Single atomic modify call: file record + note update
        self.modify(vec![
            ModifyOperation {
                operation_type: "create".into(),
                record: CkRecord {
                    record_name: file_record_uuid,
                    record_type: record_type.to_string(),
                    fields: file_fields,
                    record_change_tag: None,
                    deleted: false,
                    server_error_code: None,
                    reason: None,
                },
            },
            ModifyOperation {
                operation_type: "update".into(),
                record: CkRecord {
                    record_name: note_record_name.to_string(),
                    record_type: "SFNote".into(),
                    fields: note_fields,
                    record_change_tag: Some(change_tag),
                    deleted: false,
                    server_error_code: None,
                    reason: None,
                },
            },
        ])?;

        Ok(())
    }

    /// Move a note to trash (sets trashed=1, trashedDate=now, increments vector clock).
    pub fn trash_note(&self, record_name: &str) -> Result<()> {
        let current = self.fetch_note(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note has no recordChangeTag"))?;
        let clock = vector_clock::increment(current.str_field("vectorClock"), DEVICE_NAME)?;
        let now_ms = now_ms();

        let mut fields: Fields = HashMap::new();
        fields.insert("trashed".into(), CkField::int64(1));
        fields.insert("trashedDate".into(), CkField::timestamp(now_ms));
        fields.insert("vectorClock".into(), CkField::bytes(&clock));
        fields.insert("lastEditingDevice".into(), CkField::string(DEVICE_NAME));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms + 1));
        fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(current.str_field("uniqueIdentifier").unwrap_or(record_name)),
        );

        self.modify(vec![ModifyOperation {
            operation_type: "update".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNote".into(),
                fields,
                record_change_tag: Some(change_tag),
                deleted: false,
                server_error_code: None,
                reason: None,
            },
        }])?;
        Ok(())
    }

    /// Archive a note.
    pub fn archive_note(&self, record_name: &str) -> Result<()> {
        let current = self.fetch_note(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note has no recordChangeTag"))?;
        let clock = vector_clock::increment(current.str_field("vectorClock"), DEVICE_NAME)?;
        let now_ms = now_ms();

        let mut fields: Fields = HashMap::new();
        fields.insert("archived".into(), CkField::int64(1));
        fields.insert("archivedDate".into(), CkField::timestamp(now_ms));
        fields.insert("vectorClock".into(), CkField::bytes(&clock));
        fields.insert("lastEditingDevice".into(), CkField::string(DEVICE_NAME));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms + 1));
        fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(current.str_field("uniqueIdentifier").unwrap_or(record_name)),
        );

        self.modify(vec![ModifyOperation {
            operation_type: "update".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNote".into(),
                fields,
                record_change_tag: Some(change_tag),
                deleted: false,
                server_error_code: None,
                reason: None,
            },
        }])?;
        Ok(())
    }
}

pub enum AttachPosition {
    Append,
    Prepend,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// First `# Heading` or first non-empty line.
pub fn extract_title(text: &str) -> String {
    for line in text.lines() {
        let t = line.trim();
        if let Some(stripped) = t.strip_prefix("# ") {
            return stripped.to_string();
        }
        if !t.is_empty() {
            return t.to_string();
        }
    }
    String::new()
}

/// First body line (skipping the title line).
pub fn extract_subtitle(text: &str) -> String {
    let mut past_title = false;
    for line in text.lines() {
        let t = line.trim();
        if !past_title {
            past_title = !t.is_empty();
            continue;
        }
        if !t.is_empty() {
            return t.to_string();
        }
    }
    String::new()
}

fn count_todos(text: &str) -> (i64, i64) {
    let mut done = 0i64;
    let mut todo = 0i64;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("- [x]") || t.starts_with("- [X]") {
            done += 1;
        } else if t.starts_with("- [ ]") {
            todo += 1;
        }
    }
    (done, todo)
}

fn mime_for_ext(ext: &str) -> String {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "tiff" | "tif" => "image/tiff",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}
