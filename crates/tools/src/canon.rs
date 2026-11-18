//! Canonical JSON and length-prefixed hashing.
//!
//! Canonical form: object keys sorted by their UTF-8 bytes, no whitespace,
//! strings and numbers as `serde_json` prints them. The sort is done here so
//! the result does not depend on `serde_json`'s map ordering feature.

use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 over the fields, each prefixed with its length as 8 little-endian
/// bytes, so that ("ab","c") and ("a","bc") hash differently.
pub fn hash_fields(fields: &[&str]) -> String {
    let mut h = Sha256::new();
    for f in fields {
        h.update((f.len() as u64).to_le_bytes());
        h.update(f.as_bytes());
    }
    hex(&h.finalize())
}

/// `n` random bytes from the operating system, as hex.
pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("operating system random source failed");
    hex(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_at_every_level() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":{"z":[1,{"y":2,"x":3}],"c":"s"}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"c":"s","z":[1,{"x":3,"y":2}]},"b":1}"#).unwrap();
        assert_eq!(canonical_json(&a), r#"{"a":{"c":"s","z":[1,{"x":3,"y":2}]},"b":1}"#);
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(canonical_json(&json!({"k\"": "line\nbreak"})), r#"{"k\"":"line\nbreak"}"#);
    }

    #[test]
    fn length_prefix_separates_fields() {
        assert_ne!(hash_fields(&["ab", "c"]), hash_fields(&["a", "bc"]));
        assert_eq!(hash_fields(&["ab", "c"]), hash_fields(&["ab", "c"]));
        assert_eq!(hash_fields(&[]).len(), 64);
    }
}
