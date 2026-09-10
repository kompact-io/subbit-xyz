use std::collections::BTreeMap;

use serde_json::{Value, json};

pub fn map_labels(value: Value, lookup: &BTreeMap<String, String>) -> Value {
    let subs: Vec<(Value, Value)> = lookup.iter().map(|(k, v)| (json!(k), json!(v))).collect();
    map_values(value, &subs)
}

pub fn map_values(
    v: serde_json::Value,
    subs: &[(serde_json::Value, serde_json::Value)],
) -> serde_json::Value {
    if let Some((_, r)) = subs.iter().find(|(f, _)| *f == v) {
        return r.clone();
    }
    match v {
        serde_json::Value::Object(m) => serde_json::Value::Object(
            m.into_iter()
                .map(|(k, v)| (k, map_values(v, subs)))
                .collect(),
        ),
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.into_iter().map(|v| map_values(v, subs)).collect())
        }
        other => other,
    }
}
