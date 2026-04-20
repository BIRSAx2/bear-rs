use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ZoneId {
    pub zone_name: String,
}

impl Default for ZoneId {
    fn default() -> Self {
        Self {
            zone_name: "Notes".into(),
        }
    }
}

/// A CloudKit field value with type tag and optional encryption flag.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CkField {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_encrypted: Option<bool>,
}

impl CkField {
    pub fn string(v: impl Into<String>) -> Self {
        Self {
            kind: "STRING".into(),
            value: JsonValue::String(v.into()),
            is_encrypted: None,
        }
    }

    pub fn string_encrypted(v: impl Into<String>) -> Self {
        Self {
            kind: "STRING".into(),
            value: JsonValue::String(v.into()),
            is_encrypted: Some(true),
        }
    }

    pub fn string_null() -> Self {
        Self {
            kind: "STRING".into(),
            value: JsonValue::Null,
            is_encrypted: None,
        }
    }

    pub fn string_list(v: Vec<String>) -> Self {
        Self {
            kind: "STRING_LIST".into(),
            value: serde_json::to_value(v).unwrap(),
            is_encrypted: None,
        }
    }

    pub fn string_list_null() -> Self {
        Self {
            kind: "STRING_LIST".into(),
            value: JsonValue::Null,
            is_encrypted: None,
        }
    }

    pub fn int64(v: i64) -> Self {
        Self {
            kind: "INT64".into(),
            value: JsonValue::Number(v.into()),
            is_encrypted: None,
        }
    }

    pub fn timestamp(ms: i64) -> Self {
        Self {
            kind: "TIMESTAMP".into(),
            value: JsonValue::Number(ms.into()),
            is_encrypted: None,
        }
    }

    pub fn timestamp_null() -> Self {
        Self {
            kind: "TIMESTAMP".into(),
            value: JsonValue::Null,
            is_encrypted: None,
        }
    }

    pub fn bytes(b64: impl Into<String>) -> Self {
        Self {
            kind: "BYTES".into(),
            value: JsonValue::String(b64.into()),
            is_encrypted: None,
        }
    }

    pub fn asset_id(receipt: AssetReceipt) -> Self {
        Self {
            kind: "ASSETID".into(),
            value: serde_json::to_value(receipt).unwrap(),
            is_encrypted: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssetToken {
    pub record_type: String,
    pub record_name: String,
    pub field_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetUploadRequest {
    pub zone_id: ZoneId,
    pub tokens: Vec<AssetToken>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetUploadResponse {
    pub tokens: Vec<AssetUploadToken>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetUploadToken {
    pub record_name: String,
    pub field_name: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssetReceipt {
    pub file_checksum: String,
    pub receipt: String,
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wrapping_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_checksum: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetUploadResult {
    pub single_file: AssetReceipt,
}

pub type Fields = HashMap<String, CkField>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CkRecord {
    pub record_name: String,
    pub record_type: String,
    pub fields: Fields,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_change_tag: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    // Present on error responses:
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyOperation {
    pub operation_type: String, // "create" | "update" | "delete"
    pub record: CkRecord,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyRequest {
    pub operations: Vec<ModifyOperation>,
    pub zone_id: ZoneId,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyResponse {
    pub records: Vec<CkRecord>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CkQuery {
    pub record_type: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filter_by: Vec<CkFilter>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sort_by: Vec<CkSort>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CkFilter {
    pub field_name: String,
    pub comparator: String,
    pub field_value: CkFilterValue,
}

#[derive(Debug, Serialize, Clone)]
pub struct CkFilterValue {
    pub value: JsonValue,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CkSort {
    pub field_name: String,
    pub ascending: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    pub zone_id: ZoneId,
    pub query: CkQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired_keys: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_marker: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResponse {
    pub records: Vec<CkRecord>,
    #[serde(default)]
    pub continuation_marker: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupRequest {
    pub records: Vec<LookupRecord>,
    pub zone_id: ZoneId,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupRecord {
    pub record_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupResponse {
    pub records: Vec<CkRecord>,
}

impl CkRecord {
    pub fn str_field(&self, name: &str) -> Option<&str> {
        self.fields.get(name)?.value.as_str()
    }

    pub fn i64_field(&self, name: &str) -> Option<i64> {
        self.fields.get(name)?.value.as_i64()
    }

    pub fn bool_field(&self, name: &str) -> Option<bool> {
        self.i64_field(name).map(|v| v != 0)
    }

    pub fn string_list_field(&self, name: &str) -> Vec<String> {
        self.fields
            .get(name)
            .and_then(|f| f.value.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}
