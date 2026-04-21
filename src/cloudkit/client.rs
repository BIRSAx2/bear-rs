use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::blocking::Client;
use uuid::Uuid;

use super::auth::AuthConfig;
use super::models::*;
use super::vector_clock;
use crate::verbose;

pub const API_TOKEN: &str = "ce59f955ec47e744f720aa1d2816a4e985e472d8b859b6c7a47b81fd36646307";
const BASE_URL: &str =
    "https://api.apple-cloudkit.com/database/1/iCloud.net.shinyfrog.bear/production/private";
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

    fn device_name(&self) -> &'static str {
        "Bear CLI"
    }

    fn vector_clock_device(&self) -> &'static str {
        "Bear CLI"
    }

    fn url(&self, path: &str) -> String {
        let token = self.auth.ck_web_auth_token.replace('+', "%2B");
        let api_token = API_TOKEN.replace('+', "%2B");
        format!("{BASE_URL}{path}?ckWebAuthToken={token}&ckAPIToken={api_token}")
    }

    fn post<Req, Res>(&self, path: &str, body: &Req) -> Result<Res>
    where
        Req: serde::Serialize,
        Res: serde::de::DeserializeOwned,
    {
        let url = self.url(path);
        if verbose::enabled(1) {
            verbose::eprintln(1, format!("[cloudkit] POST {path}"));
        }
        if verbose::enabled(2) {
            let body_json = serde_json::to_string_pretty(body)
                .unwrap_or_else(|_| "<failed to serialize request body>".to_string());
            verbose::eprintln(2, format!("[cloudkit] url: {}", redact_cloudkit_url(&url)));
            verbose::eprintln(2, format!("[cloudkit] request body:\n{body_json}"));
        }
        let resp = self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .with_context(|| format!("HTTP POST {path} failed"))?;

        let status = resp.status();
        if verbose::enabled(1) {
            verbose::eprintln(1, format!("[cloudkit] {path} -> {status}"));
        }
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            if verbose::enabled(2) {
                verbose::eprintln(2, format!("[cloudkit] error body:\n{body}"));
            }
            bail!("CloudKit {path} returned {status}: {body}");
        }
        let text = resp
            .text()
            .with_context(|| format!("failed reading response from {path}"))?;
        if verbose::enabled(2) {
            verbose::eprintln(2, format!("[cloudkit] response body:\n{text}"));
        }
        serde_json::from_str::<Res>(&text)
            .with_context(|| format!("failed to parse response from {path}"))
    }

    pub fn modify(&self, ops: Vec<ModifyOperation>) -> Result<Vec<CkRecord>> {
        self.modify_in_zone(ZoneId::notes(), ops)
    }

    pub fn modify_in_zone(
        &self,
        zone_id: ZoneId,
        ops: Vec<ModifyOperation>,
    ) -> Result<Vec<CkRecord>> {
        if verbose::enabled(1) {
            let summary = ops
                .iter()
                .map(|op| format!("{}:{}", op.operation_type, op.record_type))
                .collect::<Vec<_>>()
                .join(", ");
            verbose::eprintln(
                1,
                format!(
                    "[cloudkit] modify zone={} ops={} [{}]",
                    zone_id.zone_name,
                    ops.len(),
                    summary
                ),
            );
        }
        let req = ModifyRequest {
            operations: ops,
            zone_id,
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
        verbose::eprintln(
            1,
            format!(
                "[cloudkit] list_notes include_trashed={} include_archived={} limit={limit:?}",
                include_trashed, include_archived
            ),
        );
        self.list_notes_in_zone(ZoneId::notes(), include_trashed, include_archived, limit)
    }

    pub fn list_phantom_notes(&self, limit: Option<usize>) -> Result<Vec<CkRecord>> {
        Ok(self
            .list_notes_in_zone(ZoneId::default_zone(), true, true, limit)?
            .into_iter()
            .filter(|record| {
                record
                    .zone_id
                    .as_ref()
                    .is_some_and(|zone| zone.zone_name == "_defaultZone")
            })
            .collect())
    }

    fn list_notes_in_zone(
        &self,
        zone_id: ZoneId,
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
        let mut page = 0usize;

        loop {
            page += 1;
            let remaining = limit.map(|n| n.saturating_sub(records.len()));
            if matches!(remaining, Some(0)) {
                break;
            }
            verbose::eprintln(
                2,
                format!(
                    "[cloudkit] list_notes page={} zone={} remaining={remaining:?}",
                    page, zone_id.zone_name
                ),
            );

            let req = QueryRequest {
                zone_id: zone_id.clone(),
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
            verbose::eprintln(
                1,
                format!(
                    "[cloudkit] list_notes page={} returned {} record(s)",
                    page,
                    resp.records.len()
                ),
            );
            records.extend(resp.records);
            continuation_marker = resp.continuation_marker;

            if continuation_marker.is_none() {
                break;
            }
        }

        Ok(records)
    }

    pub fn delete_phantom_notes(&self, records: &[CkRecord]) -> Result<Vec<CkRecord>> {
        let ops = records
            .iter()
            .map(|record| {
                let change_tag = record.record_change_tag.clone().ok_or_else(|| {
                    anyhow!("phantom note {} has no recordChangeTag", record.record_name)
                })?;
                Ok(ModifyOperation {
                    operation_type: "delete".into(),
                    record_type: "SFNote".into(),
                    record: CkRecord {
                        record_name: record.record_name.clone(),
                        record_type: "SFNote".into(),
                        zone_id: None,
                        fields: HashMap::new(),
                        plugin_fields: HashMap::new(),
                        record_change_tag: Some(change_tag),
                        created: record.created.clone(),
                        modified: record.modified.clone(),
                        deleted: true,
                        server_error_code: None,
                        reason: None,
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        self.modify_in_zone(ZoneId::default_zone(), ops)
    }

    pub fn list_tags(&self) -> Result<Vec<CkRecord>> {
        verbose::eprintln(1, "[cloudkit] list_tags");
        let mut records = Vec::new();
        let mut marker = None;
        let mut page = 0usize;

        loop {
            page += 1;
            let resp = self.query(QueryRequest {
                zone_id: ZoneId::default(),
                query: CkQuery {
                    record_type: "SFNoteTag".into(),
                    filter_by: vec![],
                    sort_by: vec![CkSort {
                        field_name: "title".into(),
                        ascending: true,
                    }],
                },
                results_limit: Some(500),
                desired_keys: Some(vec!["title".into(), "sf_modificationDate".into()]),
                continuation_marker: marker,
            })?;
            verbose::eprintln(
                1,
                format!(
                    "[cloudkit] list_tags page={} returned {} record(s)",
                    page,
                    resp.records.len()
                ),
            );
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
                .map(|name| LookupRecord {
                    record_name: (*name).to_string(),
                })
                .collect(),
            zone_id: ZoneId::default(),
        };
        let resp: LookupResponse = self.post("/records/lookup", &req)?;
        Ok(resp.records)
    }

    /// Fetch a single SFNote by its uniqueIdentifier (which equals its CloudKit recordName).
    pub fn fetch_note(&self, record_name: &str) -> Result<CkRecord> {
        verbose::eprintln(1, format!("[cloudkit] fetch_note record={record_name}"));
        self.lookup(&[record_name])?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("note not found: {record_name}"))
    }

    pub fn fetch_note_by_title(
        &self,
        title: &str,
        include_trashed: bool,
        include_archived: bool,
    ) -> Result<CkRecord> {
        verbose::eprintln(
            1,
            format!(
                "[cloudkit] fetch_note_by_title title={title:?} include_trashed={} include_archived={}",
                include_trashed, include_archived
            ),
        );
        let mut filter_by = vec![CkFilter {
            field_name: "title".into(),
            comparator: "EQUALS".into(),
            field_value: CkFilterValue {
                value: title.to_string().into(),
                kind: "STRING".into(),
            },
        }];
        if !include_trashed {
            filter_by.push(CkFilter {
                field_name: "trashed".into(),
                comparator: "EQUALS".into(),
                field_value: CkFilterValue {
                    value: 0.into(),
                    kind: "INT64".into(),
                },
            });
        }
        if !include_archived {
            filter_by.push(CkFilter {
                field_name: "archived".into(),
                comparator: "EQUALS".into(),
                field_value: CkFilterValue {
                    value: 0.into(),
                    kind: "INT64".into(),
                },
            });
        }

        let resp = self.query(QueryRequest {
            zone_id: ZoneId::notes(),
            query: CkQuery {
                record_type: "SFNote".into(),
                filter_by,
                sort_by: vec![CkSort {
                    field_name: "sf_modificationDate".into(),
                    ascending: false,
                }],
            },
            results_limit: Some(1),
            desired_keys: None,
            continuation_marker: None,
        })?;

        resp.records
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("note not found: {title}"))
    }

    /// Fetch a single SFNoteTag by its recordName.
    pub fn fetch_tag(&self, record_name: &str) -> Result<CkRecord> {
        verbose::eprintln(1, format!("[cloudkit] fetch_tag record={record_name}"));
        let resp = self.query(QueryRequest {
            zone_id: ZoneId::default(),
            query: CkQuery {
                record_type: "SFNoteTag".into(),
                filter_by: vec![CkFilter {
                    field_name: "uniqueIdentifier".into(),
                    comparator: "EQUALS".into(),
                    field_value: CkFilterValue {
                        value: record_name.to_string().into(),
                        kind: "STRING".into(),
                    },
                }],
                sort_by: vec![],
            },
            results_limit: Some(1),
            desired_keys: None,
            continuation_marker: None,
        })?;
        resp.records
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("tag not found: {record_name}"))
    }

    /// Upload a file to CloudKit asset storage. Returns the receipt to embed in a record field.
    pub fn upload_asset(
        &self,
        record_name: &str,
        record_type: &str,
        data: &[u8],
        mime_type: &str,
    ) -> Result<AssetReceipt> {
        verbose::eprintln(
            1,
            format!(
                "[cloudkit] upload_asset record={} type={} bytes={} mime={}",
                record_name,
                record_type,
                data.len(),
                mime_type
            ),
        );
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
        verbose::eprintln(1, format!("[cloudkit] asset upload -> {status}"));
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
        mut tag_uuids: Vec<String>,
        tag_names: Vec<String>,
    ) -> Result<CkRecord> {
        let title = extract_title(text);
        verbose::eprintln(
            1,
            format!(
                "[cloudkit] create_note title={:?} tag_names={:?}",
                title, tag_names
            ),
        );
        let device_name = self.device_name();
        let now_ms = now_ms();
        let note_uuid = Uuid::new_v4().to_string().to_uppercase();
        let subtitle = extract_subtitle(text);
        let clock = vector_clock::increment(None, self.vector_clock_device())?;
        if !tag_names.is_empty() && tag_uuids.len() != tag_names.len() {
            tag_uuids = self.resolve_tag_record_names(&tag_names, true)?;
        }

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
        fields.insert("lastEditingDevice".into(), CkField::string(device_name));
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
            record_type: "SFNote".into(),
            record: CkRecord {
                record_name: note_uuid.clone(),
                record_type: "SFNote".into(),
                zone_id: None,
                fields,
                plugin_fields: HashMap::new(),
                record_change_tag: None,
                created: None,
                modified: None,
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

    pub fn ensure_tag(&self, title: &str) -> Result<String> {
        verbose::eprintln(1, format!("[cloudkit] ensure_tag title={title:?}"));
        if let Some(existing) = self.find_tag_record_name(title)? {
            verbose::eprintln(
                2,
                format!(
                    "[cloudkit] ensure_tag reusing record={}",
                    existing.record_name
                ),
            );
            return Ok(existing.record_name);
        }

        let now_ms = now_ms();
        let tag_uuid = Uuid::new_v4().to_string().to_uppercase();
        let mut fields: Fields = HashMap::new();
        fields.insert("tagcon".into(), CkField::string_null());
        fields.insert("pinnedDate".into(), CkField::timestamp_null());
        fields.insert("pinned".into(), CkField::int64(0));
        fields.insert("pinnedNotes".into(), CkField::string_list_null());
        fields.insert("title".into(), CkField::string(title));
        fields.insert("notesCount".into(), CkField::int64(1));
        fields.insert("tagconDate".into(), CkField::timestamp_null());
        fields.insert("pinnedNotesDate".into(), CkField::timestamp_null());
        fields.insert(
            "isRoot".into(),
            CkField::int64(if title.contains('/') { 0 } else { 1 }),
        );
        fields.insert("sortingDate".into(), CkField::timestamp_null());
        fields.insert("sorting".into(), CkField::int64(0));
        fields.insert("version".into(), CkField::int64(3));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms));
        fields.insert("uniqueIdentifier".into(), CkField::string(&tag_uuid));

        self.modify(vec![ModifyOperation {
            operation_type: "create".into(),
            record_type: "SFNoteTag".into(),
            record: CkRecord {
                record_name: tag_uuid.clone(),
                record_type: "SFNoteTag".into(),
                zone_id: None,
                fields,
                plugin_fields: HashMap::new(),
                record_change_tag: None,
                created: None,
                modified: None,
                deleted: false,
                server_error_code: None,
                reason: None,
            },
        }])?;
        verbose::eprintln(
            1,
            format!("[cloudkit] ensure_tag created record={tag_uuid}"),
        );

        Ok(tag_uuid)
    }

    pub fn find_tag_record_name(&self, title: &str) -> Result<Option<CkRecord>> {
        Ok(self
            .list_tags()?
            .into_iter()
            .find(|tag| tag.str_field("title") == Some(title)))
    }

    pub fn resolve_tag_record_names(
        &self,
        tag_names: &[String],
        create_missing: bool,
    ) -> Result<Vec<String>> {
        verbose::eprintln(
            2,
            format!(
                "[cloudkit] resolve_tag_record_names names={tag_names:?} create_missing={create_missing}"
            ),
        );
        let mut uuids = Vec::with_capacity(tag_names.len());
        for tag_name in tag_names {
            let tag_uuid = match self.find_tag_record_name(tag_name)? {
                Some(existing) => existing.record_name,
                None if create_missing => self.ensure_tag(tag_name)?,
                None => continue,
            };
            uuids.push(tag_uuid);
        }
        Ok(uuids)
    }

    /// Update a note's text. Fetches the current record first to obtain the recordChangeTag
    /// and existing vector clock, then writes back the updated content.
    pub fn update_note_text(&self, record_name: &str, new_text: &str) -> Result<CkRecord> {
        self.update_note(record_name, new_text, None, None)
    }

    pub fn update_note(
        &self,
        record_name: &str,
        new_text: &str,
        tag_uuids: Option<Vec<String>>,
        tag_names: Option<Vec<String>>,
    ) -> Result<CkRecord> {
        verbose::eprintln(
            1,
            format!(
                "[cloudkit] update_note record={} len={} tags_supplied={} names_supplied={}",
                record_name,
                new_text.len(),
                tag_uuids.as_ref().map(|v| v.len()).unwrap_or(0),
                tag_names.as_ref().map(|v| v.len()).unwrap_or(0)
            ),
        );
        let device_name = self.device_name();
        let current = self.fetch_note(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note {record_name} has no recordChangeTag"))?;
        let existing_clock = current.str_field("vectorClock");
        let clock = vector_clock::increment(existing_clock, self.vector_clock_device())?;

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
        fields.insert("lastEditingDevice".into(), CkField::string(device_name));
        fields.insert("version".into(), CkField::int64(3));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms));
        fields.insert("todoCompleted".into(), CkField::int64(todo_counts.0));
        fields.insert("todoIncompleted".into(), CkField::int64(todo_counts.1));
        fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(current.str_field("uniqueIdentifier").unwrap_or(record_name)),
        );
        if let Some(tag_uuids) = tag_uuids {
            fields.insert("tags".into(), CkField::string_list(tag_uuids));
        }
        if let Some(tag_names) = tag_names {
            fields.insert("tagsStrings".into(), CkField::string_list(tag_names));
        }

        let op = ModifyOperation {
            operation_type: "update".into(),
            record_type: "SFNote".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNote".into(),
                zone_id: None,
                fields,
                plugin_fields: HashMap::new(),
                record_change_tag: Some(change_tag),
                created: current.created.clone(),
                modified: current.modified.clone(),
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
        verbose::eprintln(
            1,
            format!(
                "[cloudkit] attach_file note={} filename={} bytes={} position={}",
                note_record_name,
                filename,
                data.len(),
                match position {
                    AttachPosition::Append => "append",
                    AttachPosition::Prepend => "prepend",
                }
            ),
        );
        let device_name = self.device_name();
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

        // Fetch current note to get change tag and existing content
        let note = self.fetch_note(note_record_name)?;
        let change_tag = note
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note has no recordChangeTag"))?;
        let existing_clock = note.str_field("vectorClock");
        let clock = vector_clock::increment(existing_clock, self.vector_clock_device())?;

        // Build updated note text with file embedded
        let current_text = note.str_field("textADP").unwrap_or("").to_string();
        let encoded_name = encode_markdown_path(filename);
        let embed = if is_image {
            format!(
                "![{filename}]({encoded_name})<!-- {{\"preview\":\"true\",\"embed\":\"true\"}} -->"
            )
        } else {
            format!(
                "[{encoded_name}]({encoded_name})<!-- {{\"preview\":\"true\",\"embed\":\"true\"}} -->"
            )
        };
        let new_text = match position {
            AttachPosition::Append => {
                if current_text.ends_with('\n') {
                    format!("{current_text}\n{embed}")
                } else {
                    format!("{current_text}\n\n{embed}")
                }
            }
            AttachPosition::Prepend => {
                let mut lines = current_text.lines().map(str::to_string).collect::<Vec<_>>();
                if lines.len() > 1 {
                    lines.insert(1, String::new());
                    lines.insert(2, embed);
                } else {
                    lines.push(String::new());
                    lines.push(embed);
                }
                lines.join("\n")
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

        let has_images = if is_image {
            1
        } else {
            note.i64_field("hasImages").unwrap_or(0)
        };
        let has_files = if is_image {
            note.i64_field("hasFiles").unwrap_or(0)
        } else {
            1
        };
        let now_ms = now_ms();
        let title = extract_title(&new_text);

        // Build file record fields
        let mut file_fields: Fields = std::collections::HashMap::new();
        file_fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(&file_record_uuid),
        );
        file_fields.insert("filenameADP".into(), CkField::string_encrypted(filename));
        file_fields.insert("normalizedFileExtension".into(), CkField::string(&ext));
        file_fields.insert("fileSize".into(), CkField::int64(data.len() as i64));
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
        if is_image {
            file_fields.insert(
                "animated".into(),
                CkField::int64(if ext == "gif" { 1 } else { 0 }),
            );
        }
        file_fields.insert("version".into(), CkField::int64(3));
        file_fields.insert("sf_creationDate".into(), CkField::timestamp(now_ms));
        file_fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms));

        // Build updated note fields
        let mut note_fields: Fields = std::collections::HashMap::new();
        note_fields.insert("textADP".into(), CkField::string_encrypted(&new_text));
        note_fields.insert("text".into(), CkField::string_null());
        note_fields.insert("title".into(), CkField::string(&title));
        note_fields.insert("files".into(), CkField::string_list(files_list));
        note_fields.insert("hasImages".into(), CkField::int64(has_images));
        note_fields.insert("hasFiles".into(), CkField::int64(has_files));
        note_fields.insert("vectorClock".into(), CkField::bytes(&clock));
        note_fields.insert("lastEditingDevice".into(), CkField::string(device_name));
        note_fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms));

        // Single atomic modify call: file record + note update
        self.modify(vec![
            ModifyOperation {
                operation_type: "create".into(),
                record_type: record_type.to_string(),
                record: CkRecord {
                    record_name: file_record_uuid,
                    record_type: record_type.to_string(),
                    zone_id: None,
                    fields: file_fields,
                    plugin_fields: HashMap::new(),
                    record_change_tag: None,
                    created: None,
                    modified: None,
                    deleted: false,
                    server_error_code: None,
                    reason: None,
                },
            },
            ModifyOperation {
                operation_type: "update".into(),
                record_type: "SFNote".into(),
                record: CkRecord {
                    record_name: note_record_name.to_string(),
                    record_type: "SFNote".into(),
                    zone_id: None,
                    fields: note_fields,
                    plugin_fields: HashMap::new(),
                    record_change_tag: Some(change_tag),
                    created: note.created.clone(),
                    modified: note.modified.clone(),
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
        verbose::eprintln(1, format!("[cloudkit] trash_note record={record_name}"));
        let device_name = self.device_name();
        let current = self.fetch_note(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note has no recordChangeTag"))?;
        let clock =
            vector_clock::increment(current.str_field("vectorClock"), self.vector_clock_device())?;
        let now_ms = now_ms();

        let mut fields: Fields = HashMap::new();
        fields.insert("trashed".into(), CkField::int64(1));
        fields.insert("trashedDate".into(), CkField::timestamp(now_ms));
        fields.insert("vectorClock".into(), CkField::bytes(&clock));
        fields.insert("lastEditingDevice".into(), CkField::string(device_name));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms + 1));
        fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(current.str_field("uniqueIdentifier").unwrap_or(record_name)),
        );

        self.modify(vec![ModifyOperation {
            operation_type: "update".into(),
            record_type: "SFNote".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNote".into(),
                zone_id: None,
                fields,
                plugin_fields: HashMap::new(),
                record_change_tag: Some(change_tag),
                created: current.created.clone(),
                modified: current.modified.clone(),
                deleted: false,
                server_error_code: None,
                reason: None,
            },
        }])?;
        Ok(())
    }

    /// Archive a note.
    pub fn archive_note(&self, record_name: &str) -> Result<()> {
        verbose::eprintln(1, format!("[cloudkit] archive_note record={record_name}"));
        let device_name = self.device_name();
        let current = self.fetch_note(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note has no recordChangeTag"))?;
        let clock =
            vector_clock::increment(current.str_field("vectorClock"), self.vector_clock_device())?;
        let now_ms = now_ms();

        let mut fields: Fields = HashMap::new();
        fields.insert("archived".into(), CkField::int64(1));
        fields.insert("archivedDate".into(), CkField::timestamp(now_ms));
        fields.insert("vectorClock".into(), CkField::bytes(&clock));
        fields.insert("lastEditingDevice".into(), CkField::string(device_name));
        fields.insert("sf_modificationDate".into(), CkField::timestamp(now_ms + 1));
        fields.insert(
            "uniqueIdentifier".into(),
            CkField::string(current.str_field("uniqueIdentifier").unwrap_or(record_name)),
        );

        self.modify(vec![ModifyOperation {
            operation_type: "update".into(),
            record_type: "SFNote".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNote".into(),
                zone_id: None,
                fields,
                plugin_fields: HashMap::new(),
                record_change_tag: Some(change_tag),
                created: current.created.clone(),
                modified: current.modified.clone(),
                deleted: false,
                server_error_code: None,
                reason: None,
            },
        }])?;
        Ok(())
    }

    pub fn delete_note(&self, record_name: &str) -> Result<()> {
        verbose::eprintln(1, format!("[cloudkit] delete_note record={record_name}"));
        let current = self.fetch_note(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("note has no recordChangeTag"))?;

        self.modify(vec![ModifyOperation {
            operation_type: "delete".into(),
            record_type: "SFNote".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNote".into(),
                zone_id: None,
                fields: HashMap::new(),
                plugin_fields: HashMap::new(),
                record_change_tag: Some(change_tag),
                created: current.created.clone(),
                modified: current.modified.clone(),
                deleted: true,
                server_error_code: None,
                reason: None,
            },
        }])?;
        Ok(())
    }

    pub fn delete_tag(&self, record_name: &str) -> Result<()> {
        verbose::eprintln(1, format!("[cloudkit] delete_tag record={record_name}"));
        let current = self.fetch_tag(record_name)?;
        let change_tag = current
            .record_change_tag
            .clone()
            .ok_or_else(|| anyhow!("tag has no recordChangeTag"))?;

        self.modify(vec![ModifyOperation {
            operation_type: "delete".into(),
            record_type: "SFNoteTag".into(),
            record: CkRecord {
                record_name: record_name.to_string(),
                record_type: "SFNoteTag".into(),
                zone_id: None,
                fields: HashMap::new(),
                plugin_fields: HashMap::new(),
                record_change_tag: Some(change_tag),
                created: current.created.clone(),
                modified: current.modified.clone(),
                deleted: true,
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

fn encode_markdown_path(value: &str) -> String {
    value.replace(' ', "%20")
}

fn redact_cloudkit_url(url: &str) -> String {
    let mut redacted = url.to_string();
    for key in ["ckWebAuthToken=", "ckAPIToken="] {
        redacted = redact_query_value(&redacted, key);
    }
    redacted
}

fn redact_query_value(url: &str, key: &str) -> String {
    let Some(start) = url.find(key) else {
        return url.to_string();
    };
    let value_start = start + key.len();
    let value_end = url[value_start..]
        .find('&')
        .map(|offset| value_start + offset)
        .unwrap_or(url.len());
    let value = &url[value_start..value_end];
    let replacement = redact_secret(value);

    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..value_start]);
    out.push_str(&replacement);
    out.push_str(&url[value_end..]);
    out
}

fn redact_secret(value: &str) -> String {
    if value.len() <= 8 {
        "***".to_string()
    } else {
        format!("{}...{}", &value[..4], &value[value.len() - 4..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encodes_plus_in_cloudkit_tokens() {
        let client = CloudKitClient::new(AuthConfig {
            ck_web_auth_token: "abc+123/xyz".into(),
        })
        .unwrap();

        let url = client.url("/records/query");
        assert!(url.contains("ckWebAuthToken=abc%2B123/xyz"));
        assert!(!url.contains("ckWebAuthToken=abc+123/xyz"));
    }
}
