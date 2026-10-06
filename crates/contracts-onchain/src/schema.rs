//! Strict check of a workflow record against
//! `schema/workflow-record.v1.schema.json`. Supports exactly the JSON Schema
//! keywords that file uses; any other keyword is an error, so the schema and
//! this checker cannot drift silently. Objects may not carry properties the
//! schema does not declare unless it gives `additionalProperties`.
use serde_json::Value;

const SCHEMA: &str = include_str!("../schema/workflow-record.v1.schema.json");

/// The parsed workflow-record schema.
pub fn schema() -> Value {
    serde_json::from_str(SCHEMA).expect("schema JSON")
}

/// Validate `record` against the workflow-record schema.
pub fn validate(record: &Value) -> Result<(), String> {
    let root = schema();
    check(&root, &root, record, "$")
}

fn pattern_matches(pattern: &str, s: &str) -> Result<bool, String> {
    let hex = |t: &str, upper: bool| t.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c) || (upper && ('A'..='F').contains(&c)));
    Ok(match pattern {
        "^[0-9]+$" => !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()),
        "^0x[0-9a-f]{64}$" => s.len() == 66 && s.starts_with("0x") && hex(&s[2..], false),
        "^0x[0-9a-fA-F]{40}$" => s.len() == 42 && s.starts_with("0x") && hex(&s[2..], true),
        "^[0-9a-f]{64}$" => s.len() == 64 && hex(s, false),
        other => return Err(format!("unsupported pattern {other}")),
    })
}

fn resolve<'a>(root: &'a Value, reference: &str) -> Result<&'a Value, String> {
    let name = reference.strip_prefix("#/$defs/").ok_or_else(|| format!("unsupported $ref {reference}"))?;
    root["$defs"].get(name).ok_or_else(|| format!("missing $defs/{name}"))
}

fn type_ok(t: &str, v: &Value) -> Result<bool, String> {
    Ok(match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "integer" => v.is_u64() || v.is_i64(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        other => return Err(format!("unsupported type {other}")),
    })
}

fn check(root: &Value, schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    let s = schema.as_object().ok_or_else(|| format!("{at}: schema is not an object"))?;
    for (key, rule) in s {
        match key.as_str() {
            "$schema" | "$id" | "title" | "description" | "$defs" | "properties" | "additionalProperties" => {}
            "$ref" => check(root, resolve(root, rule.as_str().unwrap_or_default())?, v, at)?,
            "type" => {
                if !type_ok(rule.as_str().unwrap_or_default(), v)? {
                    return Err(format!("{at}: expected {rule}, got {v}"));
                }
            }
            "const" => {
                if v != rule {
                    return Err(format!("{at}: expected const {rule}"));
                }
            }
            "enum" => {
                if !rule.as_array().is_some_and(|a| a.contains(v)) {
                    return Err(format!("{at}: {v} not in {rule}"));
                }
            }
            "minimum" => {
                if v.as_f64().is_none_or(|n| n < rule.as_f64().unwrap_or(0.0)) {
                    return Err(format!("{at}: below minimum"));
                }
            }
            "pattern" => {
                let text = v.as_str().ok_or_else(|| format!("{at}: pattern on non-string"))?;
                if !pattern_matches(rule.as_str().unwrap_or_default(), text)? {
                    return Err(format!("{at}: {text:?} does not match {rule}"));
                }
            }
            "required" => {
                for name in rule.as_array().into_iter().flatten() {
                    let name = name.as_str().unwrap_or_default();
                    if v.get(name).is_none() {
                        return Err(format!("{at}: missing required {name}"));
                    }
                }
            }
            "items" => {
                for (i, item) in v.as_array().into_iter().flatten().enumerate() {
                    check(root, rule, item, &format!("{at}[{i}]"))?;
                }
            }
            "minItems" | "maxItems" => {
                let n = v.as_array().map_or(0, Vec::len) as u64;
                let bound = rule.as_u64().unwrap_or(0);
                if (key == "minItems" && n < bound) || (key == "maxItems" && n > bound) {
                    return Err(format!("{at}: {n} items violates {key} {bound}"));
                }
            }
            "propertyNames" => {
                for name in v.as_object().into_iter().flatten().map(|(k, _)| k) {
                    check(root, rule, &Value::String(name.clone()), &format!("{at}.<{name}>"))?;
                }
            }
            "oneOf" => {
                let matches = rule
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|alt| check(root, alt, v, at).is_ok())
                    .count();
                if matches != 1 {
                    return Err(format!("{at}: {matches} oneOf alternatives match"));
                }
            }
            other => return Err(format!("{at}: unsupported keyword {other}")),
        }
    }
    if let Some(object) = v.as_object() {
        let properties = s.get("properties").and_then(Value::as_object);
        let additional = s.get("additionalProperties");
        if properties.is_some() || additional.is_some() {
            for (name, value) in object {
                let path = format!("{at}.{name}");
                match (properties.and_then(|p| p.get(name)), additional) {
                    (Some(sub), _) => check(root, sub, value, &path)?,
                    (None, Some(sub)) => check(root, sub, value, &path)?,
                    (None, None) => return Err(format!("{path}: not declared in the schema")),
                }
            }
        }
    }
    Ok(())
}
