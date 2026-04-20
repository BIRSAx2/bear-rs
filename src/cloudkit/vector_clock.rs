use std::collections::BTreeMap;
use std::io::Cursor;

use anyhow::{Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use plist::Value;

/// Decode a base64-encoded binary-plist vector clock into a counter map.
pub fn decode(encoded: &str) -> Result<BTreeMap<String, u64>> {
    let bytes = BASE64.decode(encoded)?;
    let value = plist::from_reader(Cursor::new(&bytes))?;
    parse_dict(value)
}

fn parse_dict(value: Value) -> Result<BTreeMap<String, u64>> {
    match value {
        Value::Dictionary(dict) => {
            let mut map = BTreeMap::new();
            for (key, val) in dict {
                let n = match val {
                    Value::Integer(i) => i.as_unsigned().unwrap_or(0),
                    _ => bail!("vector clock value is not an integer"),
                };
                map.insert(key, n);
            }
            Ok(map)
        }
        _ => bail!("vector clock is not a plist dictionary"),
    }
}

/// Encode a counter map to base64 binary plist.
pub fn encode(clock: &BTreeMap<String, u64>) -> Result<String> {
    let dict: plist::Dictionary = clock
        .iter()
        .map(|(k, v)| (k.clone(), Value::Integer((*v).into())))
        .collect();
    let mut buf = Vec::new();
    Value::Dictionary(dict).to_writer_binary(&mut buf)?;
    Ok(BASE64.encode(&buf))
}

/// Increment this device's counter, preserving all other device entries.
/// Pass `None` for `existing` when creating a brand-new note.
pub fn increment(existing: Option<&str>, device: &str) -> Result<String> {
    let mut clock = match existing {
        Some(enc) if !enc.is_empty() => decode(enc)?,
        _ => BTreeMap::new(),
    };
    let max = clock.values().max().copied().unwrap_or(0);
    clock.insert(device.to_string(), max + 1);
    encode(&clock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_empty() {
        let enc = encode(&BTreeMap::new()).unwrap();
        let dec = decode(&enc).unwrap();
        assert!(dec.is_empty());
    }

    #[test]
    fn increment_new_device() {
        let enc = increment(None, "Bear CLI").unwrap();
        let clock = decode(&enc).unwrap();
        assert_eq!(clock["Bear CLI"], 1);
    }

    #[test]
    fn increment_preserves_existing() {
        let initial = {
            let mut m = BTreeMap::new();
            m.insert("iPhone".to_string(), 5u64);
            m.insert("Mac".to_string(), 3u64);
            encode(&m).unwrap()
        };
        let enc = increment(Some(&initial), "Bear CLI").unwrap();
        let clock = decode(&enc).unwrap();
        assert_eq!(clock["iPhone"], 5);
        assert_eq!(clock["Mac"], 3);
        assert_eq!(clock["Bear CLI"], 6); // max(5,3)+1
    }
}
