//! Upstream `yt_dlp.utils.jslib.devalue.parse_iter` (pinned f264a6c): the
//! flat index-referencing array format of Rich Harris's `devalue`, as
//! Nuxt's `__NUXT_DATA__` payloads use it.
//!
//! Upstream resolves with an explicit stack; this resolves recursively in
//! the same order (a container's children last to first, so object keys are
//! inserted in reverse, as upstream's dicts are). Recoverable problems are
//! collected in order as upstream yields them; the cases where upstream's
//! generator raises instead are `Err`.
//!
//! Python values without a JSON equivalent are mapped: `NaN` and the
//! infinities become `null` (serde_json has no non-finite numbers), a
//! `Date` stays its ISO 8601 string (validated as Python's
//! `datetime.fromisoformat` does), a `RegExp` its pattern, and `bytes`
//! (`ArrayBuffer`, `DataView`) an array of byte values. A reference back to
//! a container still being resolved (a cycle, which Python represents by
//! object identity) resolves to `null`.
use base64::Engine;
use serde_json::{Map, Number, Value};
use std::collections::HashMap;

/// An upstream reviver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reviver {
    /// `lambda data: data`.
    Indirect,
    /// `json.loads`.
    Json,
}

/// The resolved value and the messages upstream's generator yielded.
#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub value: Value,
    pub errors: Vec<String>,
}

const HOLE: i64 = -2;

/// The special negative indices (`-2`, a hole, is `None` when referenced).
fn special(index: i64) -> Option<Value> {
    match index {
        // undefined, hole, NaN, Infinity, -Infinity.
        -5..=-1 => Some(Value::Null),
        -6 => Some(Number::from_f64(-0.0).map_or(Value::Null, Value::Number)),
        _ => None,
    }
}

fn python_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Python's `repr` of a JSON value (enough for error messages).
fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(text) => crate::batch::py_repr(text),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(python_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    crate::batch::py_repr(key),
                    python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A source index as Python would use it: `Ok(Some(index))`, `Ok(None)`
/// for a `resolved` hit, or the `TypeError` upstream raises.
fn source_index(source: &Value) -> Result<i64, String> {
    match source {
        Value::Bool(value) => Ok(i64::from(*value)),
        Value::Number(number) => {
            if let Some(index) = number.as_i64() {
                return Ok(index);
            }
            let value = number.as_f64().unwrap_or(f64::NAN);
            // `-1.0 in resolved` is a hit (equal hashes).
            if (-6.0..=-1.0).contains(&value) && value.fract() == 0.0 {
                return Ok(value as i64);
            }
            if value < 0.0 {
                return Ok(-7);
            }
            Err("list indices must be integers or slices, not float".into())
        }
        other => Err(format!(
            "'<' not supported between instances of '{}' and 'int'",
            python_type(other)
        )),
    }
}

/// Python's `base64.b64decode(text)` (non-alphabet characters discarded).
fn b64decode(text: &str) -> Result<Vec<u8>, String> {
    let filtered: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(filtered.trim_end_matches('=').to_owned() + pad(&filtered))
        .map_err(|_| "Incorrect padding".to_owned())
}

fn pad(filtered: &str) -> &'static str {
    let data = filtered.trim_end_matches('=').len();
    let pads = filtered.len() - data;
    match (data % 4, pads) {
        (0, _) => "",
        (2, 2..) => "==",
        (3, 1..) => "=",
        // Missing padding: Python fails; force a decode error.
        _ => "!",
    }
}

fn bytes_value(bytes: &[u8]) -> Value {
    Value::Array(bytes.iter().map(|byte| Value::from(*byte)).collect())
}

fn float_value(value: f64) -> Value {
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

/// IEEE 754 half precision.
fn f16_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f64::from(bits & 0x3ff);
    match exponent {
        0 => sign * fraction * 2f64.powi(-24),
        0x1f if fraction == 0.0 => sign * f64::INFINITY,
        0x1f => f64::NAN,
        _ => sign * (1.0 + fraction / 1024.0) * 2f64.powi(exponent - 15),
    }
}

/// `struct.calcsize('=' + typecode)` for each typed array.
fn item_size(type_name: &str) -> Option<usize> {
    Some(match type_name {
        "Int8Array" | "Uint8Array" | "Uint8ClampedArray" => 1,
        "Int16Array" | "Uint16Array" | "Float16Array" => 2,
        "Int32Array" | "Uint32Array" | "Float32Array" => 4,
        "Float64Array" | "BigInt64Array" | "BigUint64Array" => 8,
        _ => return None,
    })
}

/// `struct.unpack('=' + typecode, item)` (native order is little endian
/// on every supported platform).
fn unpack(type_name: &str, item: &[u8]) -> Value {
    let array = |n: usize| -> [u8; 8] {
        let mut out = [0; 8];
        out[..n].copy_from_slice(item);
        out
    };
    match type_name {
        "Int8Array" => Value::from(item[0] as i8),
        "Uint8Array" | "Uint8ClampedArray" => Value::from(item[0]),
        "Int16Array" => Value::from(i16::from_le_bytes([item[0], item[1]])),
        "Uint16Array" => Value::from(u16::from_le_bytes([item[0], item[1]])),
        "Float16Array" => float_value(f16_to_f64(u16::from_le_bytes([item[0], item[1]]))),
        "Int32Array" => Value::from(i32::from_le_bytes([item[0], item[1], item[2], item[3]])),
        "Uint32Array" => Value::from(u32::from_le_bytes([item[0], item[1], item[2], item[3]])),
        "Float32Array" => float_value(f64::from(f32::from_le_bytes([
            item[0], item[1], item[2], item[3],
        ]))),
        "Float64Array" => float_value(f64::from_le_bytes(array(8))),
        "BigInt64Array" => Value::from(i64::from_le_bytes(array(8))),
        _ => Value::from(u64::from_le_bytes(array(8))),
    }
}

fn non_negative_int(value: Option<&Value>) -> Option<Option<usize>> {
    match value {
        None => Some(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some),
        _ => None,
    }
}

struct State<'a> {
    parsed: &'a [Value],
    revivers: &'a [(&'a str, Reviver)],
    resolved: HashMap<i64, Value>,
    pending: Vec<i64>,
    errors: Vec<String>,
}

impl State<'_> {
    fn resolve(&mut self, source: &Value) -> Result<Value, String> {
        let index = source_index(source)?;
        if let Some(value) = special(index) {
            return Ok(value);
        }
        if let Some(value) = self.resolved.get(&index) {
            return Ok(value.clone());
        }
        if index < 0 {
            self.errors.push(format!("invalid index: {index}"));
            return Ok(Value::Null);
        }
        if self.pending.contains(&index) {
            // A cycle: Python shares the object; JSON cannot.
            return Ok(Value::Null);
        }
        let Some(value) = usize::try_from(index)
            .ok()
            .and_then(|at| self.parsed.get(at))
        else {
            self.errors.push("list index out of range".into());
            return Ok(Value::Null);
        };
        self.pending.push(index);
        let result = self.resolve_value(index, value);
        self.pending.pop();
        let result = result?;
        if let Some(result) = result {
            self.resolved.insert(index, result.clone());
            return Ok(result);
        }
        // A revived value is not cached.
        self.revive(index, value)
    }

    fn revive(&mut self, index: i64, value: &Value) -> Result<Value, String> {
        let items = value.as_array().expect("revivers are arrays");
        let name = items[0].as_str().expect("typed");
        let reviver = self
            .revivers
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, reviver)| *reviver)
            .expect("reviver");
        let inner = items
            .get(1)
            .ok_or_else(|| "list index out of range".to_owned())?;
        if source_index(inner).ok() == Some(index) {
            self.errors.push(format!(
                "{} cannot point to itself (index: {index})",
                crate::batch::py_repr(name)
            ));
            return Ok(Value::Null);
        }
        self.pending.push(index);
        let data = self.resolve(inner);
        self.pending.pop();
        let data = data?;
        match reviver {
            Reviver::Indirect => Ok(data),
            Reviver::Json => {
                let failure = match &data {
                    Value::String(text) => match crate::json::from_str::<Value>(text) {
                        Ok(value) => return Ok(value),
                        Err(error) => format!(
                            "Expecting value: line {} column {} (char {})",
                            error.line(),
                            error.column(),
                            error.column().saturating_sub(1)
                        ),
                    },
                    other => format!(
                        "the JSON object must be str, bytes or bytearray, not {}",
                        python_type(other)
                    ),
                };
                self.errors.push(format!(
                    "failed to parse {} as {}: {failure}",
                    python_repr(inner),
                    crate::batch::py_repr(name)
                ));
                Ok(Value::Null)
            }
        }
    }

    /// `None` when the entry is revived instead.
    fn resolve_value(&mut self, index: i64, value: &Value) -> Result<Option<Value>, String> {
        let items = match value {
            Value::Array(items) => items,
            Value::Object(map) => {
                let mut result = Map::new();
                for (key, source) in map.iter().rev() {
                    let resolved = self.resolve(source)?;
                    result.insert(key.clone(), resolved);
                }
                return Ok(Some(Value::Object(result)));
            }
            other => return Ok(Some(other.clone())),
        };
        let Some(Value::String(type_name)) = items.first() else {
            let mut result = vec![Value::Null; items.len()];
            for (offset, source) in items.iter().enumerate().rev() {
                result[offset] = self.resolve(source)?;
            }
            return Ok(Some(Value::Array(result)));
        };
        if self.revivers.iter().any(|(name, _)| name == type_name) {
            return Ok(None);
        }
        let second = items.get(1);
        let missing = || "list index out of range".to_owned();
        let result = match type_name.as_str() {
            "Date" => {
                let parsed = second
                    .and_then(Value::as_str)
                    .filter(|text| crate::dates::is_py_isoformat_datetime(text));
                match parsed {
                    Some(_) => second.cloned().unwrap_or(Value::Null),
                    None => {
                        let repr = python_repr(second.ok_or_else(missing)?);
                        self.errors.push(format!("invalid date: {repr}"));
                        Value::Null
                    }
                }
            }
            "Set" => {
                let mut result = vec![Value::Null; items.len() - 1];
                for (offset, source) in items[1..].iter().enumerate().rev() {
                    result[offset] = self.resolve(source)?;
                }
                Value::Array(result)
            }
            "Map" => {
                let pairs = &items[1..];
                if pairs.len() % 2 != 0 {
                    return Err("zip() argument 2 is shorter than argument 1".into());
                }
                let mut result = vec![Value::Null; pairs.len() / 2];
                for (offset, pair) in pairs.chunks(2).enumerate().rev() {
                    let value = self.resolve(&pair[1])?;
                    let key = self.resolve(&pair[0])?;
                    result[offset] = Value::Array(vec![key, value]);
                }
                Value::Array(result)
            }
            "RegExp" => second.cloned().ok_or_else(missing)?,
            "Object" => second.cloned().ok_or_else(missing)?,
            "BigInt" => {
                let value = second.ok_or_else(missing)?;
                match value {
                    Value::Number(number) if number.is_i64() || number.is_u64() => value.clone(),
                    Value::String(text) => {
                        let text = text.trim().replace('_', "");
                        text.parse::<i64>()
                            .map(Value::from)
                            .or_else(|_| text.parse::<u64>().map(Value::from))
                            .map_err(|_| {
                                format!(
                                    "invalid literal for int() with base 10: {}",
                                    python_repr(value)
                                )
                            })?
                    }
                    other => {
                        return Err(format!(
                            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                            python_type(other)
                        ));
                    }
                }
            }
            "null" => {
                let pairs = &items[1..];
                if pairs.len() % 2 != 0 {
                    return Err("zip() argument 2 is shorter than argument 1".into());
                }
                let mut result = Map::new();
                for pair in pairs.chunks(2).rev() {
                    let key = match &pair[0] {
                        Value::String(key) => key.clone(),
                        other => other.to_string(),
                    };
                    let value = self.resolve(&pair[1])?;
                    result.insert(key, value);
                }
                Value::Object(result)
            }
            "ArrayBuffer" => match second {
                Some(Value::String(text)) => match b64decode(text) {
                    Ok(bytes) => bytes_value(&bytes),
                    Err(error) => {
                        self.errors
                            .push(format!("Invalid ArrayBuffer at {index}: {error}"));
                        Value::Null
                    }
                },
                _ => {
                    let rest = Value::Array(items[1..].to_vec());
                    let error = match second {
                        // `value[1]` in the message raises first.
                        None => return Err(missing()),
                        Some(_) => format!("Invalid ArrayBuffer encoding: {}", python_repr(&rest)),
                    };
                    self.errors
                        .push(format!("Invalid ArrayBuffer at {index}: {error}"));
                    Value::Null
                }
            },
            name if name == "DataView" || item_size(name).is_some() => {
                match self.typed_array(name, items) {
                    Ok(value) => value,
                    Err(error) => {
                        self.errors
                            .push(format!("Invalid {name} at {index}: {error}"));
                        Value::Null
                    }
                }
            }
            name => {
                self.errors.push(format!(
                    "Invalid type at {index}: {}",
                    crate::batch::py_repr(name)
                ));
                Value::Null
            }
        };
        Ok(Some(result))
    }

    fn typed_array(&self, name: &str, items: &[Value]) -> Result<Value, String> {
        let data = match items.get(1) {
            None => return Err("Missing ArrayBuffer reference".into()),
            Some(Value::String(text)) => b64decode(text)?,
            Some(reference) => {
                let buffer = match reference {
                    Value::Number(number) => number
                        .as_u64()
                        .and_then(|at| usize::try_from(at).ok())
                        .filter(|at| *at < self.parsed.len()),
                    _ => None,
                };
                let Some(buffer) = buffer else {
                    return Err(format!(
                        "Invalid ArrayBuffer index: {}",
                        python_repr(reference)
                    ));
                };
                match self.parsed[buffer].as_array().map(Vec::as_slice) {
                    Some([Value::String(kind), Value::String(text), ..])
                        if kind == "ArrayBuffer" =>
                    {
                        b64decode(text)?
                    }
                    _ => {
                        return Err(format!("Invalid ArrayBuffer reference: {buffer}"));
                    }
                }
            }
        };
        let offset = match items.get(2) {
            None => 0,
            Some(value) => non_negative_int(Some(value))
                .flatten()
                .ok_or_else(|| format!("Invalid byte offset: {}", python_repr(value)))?,
        };
        let length = match items.get(3) {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                non_negative_int(Some(value))
                    .flatten()
                    .ok_or_else(|| format!("Invalid length: {}", python_repr(value)))?,
            ),
        };
        if name == "DataView" {
            let end = length.map_or(data.len(), |length| offset + length);
            if offset > data.len() || end > data.len() {
                return Err("View exceeds ArrayBuffer length".into());
            }
            return Ok(bytes_value(&data[offset..end]));
        }
        let size = item_size(name).expect("typed array");
        if offset % size != 0 {
            return Err(format!(
                "Byte offset {offset} is not aligned to {size}-byte elements"
            ));
        }
        if offset > data.len() {
            return Err("View exceeds ArrayBuffer length".into());
        }
        let view = match length {
            None => {
                let view = &data[offset..];
                if view.len() % size != 0 {
                    return Err(format!("Byte length is not a multiple of {size}"));
                }
                view
            }
            Some(length) => {
                let end = offset + length * size;
                if end > data.len() {
                    return Err("View exceeds ArrayBuffer length".into());
                }
                &data[offset..end]
            }
        };
        Ok(Value::Array(
            view.chunks(size).map(|item| unpack(name, item)).collect(),
        ))
    }
}

/// `devalue.parse_iter(parsed, revivers=...)`: the value and the yielded
/// errors, or the error upstream raises.
pub fn parse_iter(parsed: &Value, revivers: &[(&str, Reviver)]) -> Result<Parsed, String> {
    let items = match parsed {
        Value::Number(number) if number.is_i64() => {
            let index = number.as_i64().unwrap_or_default();
            return match special(index) {
                Some(value) if index != HOLE => Ok(Parsed {
                    value,
                    errors: Vec::new(),
                }),
                _ => Err("invalid integer input".into()),
            };
        }
        Value::Array(items) if items.is_empty() => {
            return Err("expected a non-empty list as input".into());
        }
        Value::Array(items) => items,
        _ => return Err("expected int or list as input".into()),
    };
    let mut state = State {
        parsed: items,
        revivers,
        resolved: HashMap::new(),
        pending: Vec::new(),
        errors: Vec::new(),
    };
    let value = state.resolve(&Value::from(0))?;
    Ok(Parsed {
        value,
        errors: state.errors,
    })
}

/// `devalue.parse`: the first yielded error is raised.
pub fn parse(parsed: &Value, revivers: &[(&str, Reviver)]) -> Result<Value, String> {
    let parsed = parse_iter(parsed, revivers)?;
    match parsed.errors.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(parsed.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// `test/test_devalue.py` `TEST_CASES_EQUALS` and `TEST_CASES_IS`.
    #[test]
    fn parses_upstream_cases() {
        let u16s: Vec<u8> = [1u16, 2, 3, 4]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let f16s = [0x00, 0xc0, 0x00, 0x3e]; // -2.0, 1.5
        let u32s: Vec<u8> = [12345u32, 12345678]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let i64s: Vec<u8> = [-1i64, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
        let u64s: Vec<u8> = [1u64, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
        let cases = [
            (json!([-42]), json!(-42)),
            (json!(["woo!!!"]), json!("woo!!!")),
            (json!([["Object", 42]]), json!(42)),
            (json!([["Object", "yar"]]), json!("yar")),
            (json!(-4), Value::Null),
            (json!(-5), Value::Null),
            (json!(-6), json!(-0.0)),
            (json!([["RegExp", "regexp", "gim"]]), json!("regexp")),
            (
                json!([["Date", "2001-09-09T01:46:40.000Z"]]),
                json!("2001-09-09T01:46:40.000Z"),
            ),
            (json!([[1, 2, 3], "a", "b", "c"]), json!(["a", "b", "c"])),
            (json!([[]]), json!([])),
            (json!([[-2, 1, -2], "b"]), json!([null, "b", null])),
            (
                json!([{"foo": 1, "x-y": 2}, "bar", "z"]),
                json!({"foo": "bar", "x-y": "z"}),
            ),
            (json!([["Set", 1, 2, 3], 1, 2, 3]), json!([1, 2, 3])),
            (json!([["Map", 1, 2], "a", "b"]), json!([["a", "b"]])),
            (json!([["BigInt", "1"]]), json!(1)),
            (json!([["Uint8Array", "AQID"]]), json!([1, 2, 3])),
            (
                json!([["Uint8Array", 1], ["ArrayBuffer", "AQID"]]),
                json!([1, 2, 3]),
            ),
            (
                json!([["Uint8Array", 1, 1, 2], ["ArrayBuffer", "AQIDBAU="]]),
                json!([2, 3]),
            ),
            (
                json!([["Uint16Array", 1, 2, 2], ["ArrayBuffer", b64(&u16s)]]),
                json!([2, 3]),
            ),
            (
                json!([["DataView", 1, 1, 2], ["ArrayBuffer", "AQIDBAU="]]),
                json!([2, 3]),
            ),
            (
                json!([["DataView", 1, 2], ["ArrayBuffer", "AQIDBAU="]]),
                json!([3, 4, 5]),
            ),
            (json!([["Float16Array", b64(&f16s)]]), json!([-2.0, 1.5])),
            (
                json!([["Uint32Array", b64(&u32s)]]),
                json!([12345, 12345678]),
            ),
            (json!([["BigInt64Array", b64(&i64s)]]), json!([-1, 2])),
            (json!([["BigUint64Array", b64(&u64s)]]), json!([1, 2])),
            (json!([["ArrayBuffer", "AQID"]]), json!([1, 2, 3])),
            (json!([[1, 1], "a string"]), json!(["a string", "a string"])),
            (json!([[1, 1], null]), json!([null, null])),
            (json!([[1, 1], {}]), json!([{}, {}])),
            (json!([["null"]]), json!({})),
            (json!([{}]), json!({})),
            (json!([true]), json!(true)),
            (json!([["Object", false]]), json!(false)),
            (json!(-1), Value::Null),
            (json!([null]), Value::Null),
            (json!(-3), Value::Null),
        ];
        for (unparsed, expected) in cases {
            assert_eq!(parse(&unparsed, &[]), Ok(expected), "{unparsed}");
        }
    }

    /// `TEST_CASES_INVALID`.
    #[test]
    fn rejects_upstream_invalid_cases() {
        let cases = [
            (json!(""), "expected int or list as input"),
            (json!(-2), "invalid integer input"),
            (json!("hello"), "expected int or list as input"),
            (json!(42), "invalid integer input"),
            (json!(true), "expected int or list as input"),
            (Value::Null, "expected int or list as input"),
            (json!({}), "expected int or list as input"),
            (json!([]), "expected a non-empty list as input"),
            (
                json!([[1, 2, 3, 4, 5, 6, 7, -7], 1, 2, 3, 4, 5, 6, 7]),
                "invalid index: -7",
            ),
        ];
        for (unparsed, error) in cases {
            assert_eq!(parse(&unparsed, &[]), Err(error.into()), "{unparsed}");
        }
    }

    /// Cycles cannot be shared; the back-reference is `null`.
    #[test]
    fn cycles_resolve_to_null() {
        assert_eq!(
            parse(&json!([["Map", 1, 0], "self"]), &[]),
            Ok(json!([["self", null]]))
        );
        assert_eq!(
            parse(&json!([["Set", 0, 1], 42]), &[]),
            Ok(json!([null, 42]))
        );
        assert_eq!(parse(&json!([{"self": 0}]), &[]), Ok(json!({"self": null})));
        assert_eq!(
            parse(&json!([[1, 2], {"second": 2}, {"first": 1}]), &[]),
            Ok(json!([{"second": null}, {"first": {"second": null}}]))
        );
    }

    #[test]
    fn applies_revivers() {
        assert_eq!(
            parse(
                &json!([["indirect", 1], {"a": 2}, "b"]),
                &[("indirect", Reviver::Indirect)]
            ),
            Ok(json!({"a": "b"}))
        );
        assert_eq!(
            parse(
                &json!([["parse", 1], "{\"a\":0}"]),
                &[("parse", Reviver::Json)]
            ),
            Ok(json!({"a": 0}))
        );
        assert_eq!(
            parse(
                &json!([{"a": 1, "b": 3}, ["EmptyRef", 2], "false", ["EmptyRef", 2]]),
                &[("EmptyRef", Reviver::Json)]
            ),
            Ok(json!({"a": false, "b": false}))
        );
        let parsed = parse_iter(&json!([["Ref", 0]]), &[("Ref", Reviver::Indirect)]).unwrap();
        assert_eq!(parsed.errors, ["'Ref' cannot point to itself (index: 0)"]);
    }

    #[test]
    fn object_keys_follow_upstream_insertion_order() {
        let value = parse(&json!([{"a": 1, "b": 2}, 1, 2]), &[]).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["b", "a"]);
    }

    #[test]
    fn collects_recoverable_errors() {
        let parsed = parse_iter(&json!([[1, 5], ["nope"]]), &[]).unwrap();
        assert_eq!(parsed.value, json!([null, null]));
        assert_eq!(
            parsed.errors,
            ["list index out of range", "Invalid type at 1: 'nope'"]
        );
    }

    #[test]
    fn dates_follow_python_fromisoformat() {
        // Upstream fa98866 (`datetime.fromisoformat`, from 3d28cf4).
        for valid in ["2001-09-09", "2001-W01-1", "2001-09-09T009Z"] {
            let parsed = parse_iter(&json!([["Date", valid]]), &[]).unwrap();
            assert_eq!((parsed.value, parsed.errors), (json!(valid), vec![]));
        }
        for (invalid, error) in [
            (json!("2001-09-09 "), "invalid date: '2001-09-09 '"),
            (json!("not a date"), "invalid date: 'not a date'"),
            (json!(5), "invalid date: 5"),
        ] {
            let parsed = parse_iter(&json!([["Date", invalid]]), &[]).unwrap();
            assert_eq!(
                (parsed.value, parsed.errors),
                (Value::Null, vec![error.to_owned()])
            );
        }
    }
}
