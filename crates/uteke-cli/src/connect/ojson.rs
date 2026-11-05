//! A JSON value that keeps the key order of the file it was read from.
//!
//! `serde_json::Value` sorts object keys unless the `preserve_order` feature
//! is on, and turning that on would change the key order of every JSON the
//! workspace prints. `connect` must not reorder a user's settings file, so it
//! reads and writes through this type instead.

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<OJson>),
    Object(Vec<(String, OJson)>),
}

impl OJson {
    pub(crate) fn empty_object() -> Self {
        OJson::Object(Vec::new())
    }

    pub(crate) fn get(&self, key: &str) -> Option<&OJson> {
        match self {
            OJson::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut OJson> {
        match self {
            OJson::Object(entries) => entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Set `key`, keeping its position when it exists and appending it when not.
    pub(crate) fn insert(&mut self, key: &str, value: OJson) {
        if let OJson::Object(entries) = self {
            match entries.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => entries.push((key.to_string(), value)),
            }
        }
    }

    pub(crate) fn remove(&mut self, key: &str) {
        if let OJson::Object(entries) = self {
            entries.retain(|(k, _)| k != key);
        }
    }

    pub(crate) fn is_empty_container(&self) -> bool {
        match self {
            OJson::Object(e) => e.is_empty(),
            OJson::Array(a) => a.is_empty(),
            _ => false,
        }
    }

    /// The same value as a `serde_json::Value`, to compare entries without
    /// caring about key order.
    pub(crate) fn to_value(&self) -> serde_json::Value {
        match self {
            OJson::Null => serde_json::Value::Null,
            OJson::Bool(b) => serde_json::Value::Bool(*b),
            OJson::Number(n) => serde_json::Value::Number(n.clone()),
            OJson::String(s) => serde_json::Value::String(s.clone()),
            OJson::Array(a) => serde_json::Value::Array(a.iter().map(OJson::to_value).collect()),
            OJson::Object(e) => serde_json::Value::Object(
                e.iter().map(|(k, v)| (k.clone(), v.to_value())).collect(),
            ),
        }
    }

    pub(crate) fn from_value(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => OJson::Null,
            serde_json::Value::Bool(b) => OJson::Bool(*b),
            serde_json::Value::Number(n) => OJson::Number(n.clone()),
            serde_json::Value::String(s) => OJson::String(s.clone()),
            serde_json::Value::Array(a) => OJson::Array(a.iter().map(OJson::from_value).collect()),
            serde_json::Value::Object(m) => OJson::Object(
                m.iter()
                    .map(|(k, v)| (k.clone(), OJson::from_value(v)))
                    .collect(),
            ),
        }
    }
}

impl Serialize for OJson {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            OJson::Null => s.serialize_unit(),
            OJson::Bool(b) => s.serialize_bool(*b),
            OJson::Number(n) => n.serialize(s),
            OJson::String(v) => s.serialize_str(v),
            OJson::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            OJson::Object(entries) => {
                let mut map = s.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for OJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = OJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_unit<E>(self) -> Result<OJson, E> {
                Ok(OJson::Null)
            }
            fn visit_none<E>(self) -> Result<OJson, E> {
                Ok(OJson::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<OJson, E> {
                Ok(OJson::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<OJson, E> {
                Ok(OJson::Number(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<OJson, E> {
                Ok(OJson::Number(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<OJson, E> {
                serde_json::Number::from_f64(v)
                    .map(OJson::Number)
                    .ok_or_else(|| E::custom("a number JSON cannot hold"))
            }
            fn visit_str<E>(self, v: &str) -> Result<OJson, E> {
                Ok(OJson::String(v.to_string()))
            }
            fn visit_string<E>(self, v: String) -> Result<OJson, E> {
                Ok(OJson::String(v))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<OJson, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = seq.next_element()? {
                    out.push(v);
                }
                Ok(OJson::Array(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<OJson, A::Error> {
                let mut out: Vec<(String, OJson)> = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, OJson>()? {
                    // A repeated key keeps the last value, as serde_json does.
                    match out.iter_mut().find(|(ek, _)| *ek == k) {
                        Some((_, ev)) => *ev = v,
                        None => out.push((k, v)),
                    }
                }
                Ok(OJson::Object(out))
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_keep_the_order_of_the_file() {
        let text = "{\n  \"zeta\": 1,\n  \"alpha\": {\n    \"y\": [\n      true,\n      null,\n      2.5\n    ],\n    \"b\": \"x\"\n  }\n}";
        let v: OJson = serde_json::from_str(text).unwrap();
        assert_eq!(serde_json::to_string_pretty(&v).unwrap(), text);
    }
}
