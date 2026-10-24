//! The Python objects upstream's `JSInterpreter` computes with, their
//! `str`/`repr`/`json.dumps` forms, Python equality and truthiness, and
//! `LocalNameSpace`'s chained scopes.
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

/// A scope dictionary (`dict` of names), shared like the Python object.
pub type Scope = Rc<RefCell<HashMap<String, JsValue>>>;

/// A new scope holding `pairs`.
pub fn scope<K: Into<String>>(pairs: impl IntoIterator<Item = (K, JsValue)>) -> Scope {
    Rc::new(RefCell::new(
        pairs.into_iter().map(|(k, v)| (k.into(), v)).collect(),
    ))
}

/// A value of the interpreter.
#[derive(Clone)]
pub enum JsValue {
    /// `JS_Undefined` (a class object upstream).
    Undefined,
    /// Python `None`: JavaScript `null` and statements without a value.
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A Python `str` (shared: `===` compares identity).
    Str(Rc<str>),
    /// A Python `list` (shared and mutable).
    List(Rc<RefCell<Vec<JsValue>>>),
    /// A Python `dict` in insertion order, keyed by any hashable value.
    Object(Rc<RefCell<Vec<(JsValue, JsValue)>>>),
    Function(JsFunction),
    /// A Python exception bound by `catch (e)`.
    Error(Rc<JsError>),
}

impl JsValue {
    pub fn str(text: impl Into<String>) -> Self {
        Self::Str(Rc::from(text.into()))
    }

    pub fn list(items: Vec<JsValue>) -> Self {
        Self::List(Rc::new(RefCell::new(items)))
    }

    /// A `dict` with string keys.
    pub fn object<K: Into<String>>(pairs: impl IntoIterator<Item = (K, JsValue)>) -> Self {
        Self::Object(Rc::new(RefCell::new(
            pairs.into_iter().map(|(k, v)| (Self::str(k), v)).collect(),
        )))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text.as_ref()),
            _ => None,
        }
    }
}

impl fmt::Debug for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&py_repr(self))
    }
}

impl PartialEq for JsValue {
    /// Python `==` (as upstream's tests compare results).
    fn eq(&self, other: &Self) -> bool {
        py_eq(self, other)
    }
}

/// A function built by `build_function` (`resf`), with the `repr` given by
/// `function_with_repr`.
#[derive(Clone)]
pub struct JsFunction {
    pub(super) repr: Rc<str>,
    pub(super) argnames: Rc<Vec<String>>,
    pub(super) code: Rc<str>,
    pub(super) global_stack: Rc<Vec<Frame>>,
}

impl JsFunction {
    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.code, &other.code) && Rc::ptr_eq(&self.global_stack, &other.global_stack)
    }
}

/// One of `LocalNameSpace.maps`: a `dict`, or (as `extract_object` passes
/// the caller's namespace on) a whole `LocalNameSpace`.
#[derive(Clone)]
pub enum Frame {
    Dict(Scope),
    Chain(Rc<Vec<Frame>>),
}

impl Frame {
    fn contains(&self, key: &str) -> bool {
        match self {
            Self::Dict(scope) => scope.borrow().contains_key(key),
            Self::Chain(frames) => frames.iter().any(|frame| frame.contains(key)),
        }
    }

    fn get(&self, key: &str) -> Option<JsValue> {
        match self {
            Self::Dict(scope) => scope.borrow().get(key).cloned(),
            Self::Chain(frames) => frames.iter().find_map(|frame| frame.get(key)),
        }
    }

    /// `frame[key] = value` (`dict.update` uses the same per key).
    pub(super) fn set(&self, key: &str, value: JsValue) {
        match self {
            Self::Dict(scope) => {
                scope.borrow_mut().insert(key.to_owned(), value);
            }
            Self::Chain(frames) => chain_set(frames, key, value),
        }
    }
}

/// `LocalNameSpace.__setitem__`: the first scope holding `key`, else the
/// innermost.
fn chain_set(frames: &[Frame], key: &str, value: JsValue) {
    match frames.iter().find(|frame| frame.contains(key)) {
        Some(frame) => frame.set(key, value),
        None => {
            if let Some(first) = frames.first() {
                first.set(key, value);
            }
        }
    }
}

/// Upstream `LocalNameSpace` (a `ChainMap`).
#[derive(Clone)]
pub struct Namespace(pub(super) Vec<Frame>);

impl Namespace {
    pub fn new(frames: Vec<Frame>) -> Self {
        Self(frames)
    }

    pub fn get(&self, key: &str) -> Option<JsValue> {
        self.0.iter().find_map(|frame| frame.get(key))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.iter().any(|frame| frame.contains(key))
    }

    /// `ChainMap.__getitem__`: a `KeyError` when missing.
    pub(super) fn getitem(&self, key: &str) -> Result<JsValue, JsError> {
        self.get(key)
            .ok_or_else(|| JsError::python("KeyError", crate::batch::py_repr(key)))
    }

    pub fn set(&self, key: &str, value: JsValue) {
        chain_set(&self.0, key, value);
    }

    pub(super) fn set_local(&self, key: &str, value: JsValue) {
        if let Some(first) = self.0.first() {
            first.set(key, value);
        }
    }

    pub(super) fn get_local(&self, key: &str) -> JsValue {
        match self.0.first() {
            Some(first) if first.contains(key) => first.get(key).unwrap_or(JsValue::Undefined),
            _ => JsValue::Undefined,
        }
    }

    pub(super) fn new_child(&self, scope: Scope) -> Self {
        let mut frames = vec![Frame::Dict(scope)];
        frames.extend(self.0.iter().cloned());
        Self(frames)
    }
}

/// An exception raised while interpreting.
#[derive(Clone, Debug)]
pub enum JsError {
    /// `JSInterpreter.Exception` (an `ExtractorError`) and its `cause`.
    Exception {
        message: String,
        cause: Option<Box<JsError>>,
    },
    /// `JS_Break` (`Invalid break`).
    Break,
    /// `JS_Continue` (`Invalid continue`).
    Continue,
    /// `JS_Throw` of the thrown value.
    Throw(JsValue),
    /// A Python built-in exception (`TypeError`, `KeyError`, ...) upstream
    /// lets escape.
    Python { kind: &'static str, message: String },
}

impl JsError {
    pub(super) fn python(kind: &'static str, message: impl Into<String>) -> Self {
        Self::Python {
            kind,
            message: message.into(),
        }
    }

    pub(super) fn type_error(message: impl Into<String>) -> Self {
        Self::python("TypeError", message)
    }

    /// `ExtractorError.orig_msg` (`str(e)` for a Python exception).
    pub fn orig_msg(&self) -> String {
        match self {
            Self::Exception { message, .. } => message.clone(),
            Self::Break => "Invalid break".into(),
            Self::Continue => "Invalid continue".into(),
            Self::Throw(value) => format!("Uncaught exception {}", py_str(value)),
            Self::Python { message, .. } => message.clone(),
        }
    }

    fn class_name(&self) -> &'static str {
        match self {
            Self::Exception { .. } => "Exception",
            Self::Break => "JS_Break",
            Self::Continue => "JS_Continue",
            Self::Throw(_) => "JS_Throw",
            Self::Python { kind, .. } => kind,
        }
    }

    /// Python `repr(e)`.
    pub fn repr(&self) -> String {
        match self {
            Self::Python {
                kind: "KeyError",
                message,
            } => format!("KeyError({message})"),
            _ => format!(
                "{}({})",
                self.class_name(),
                crate::batch::py_repr(&self.to_string())
            ),
        }
    }
}

impl fmt::Display for JsError {
    /// Python `str(e)`: an `ExtractorError` adds its cause and upstream's
    /// bug report request.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Python { message, .. } => f.write_str(message),
            Self::Exception {
                cause: Some(cause), ..
            } => write!(
                f,
                "{} (caused by {}); {}",
                self.orig_msg(),
                cause.repr(),
                crate::BUG_REPORTS_MESSAGE
            ),
            _ => write!(f, "{}; {}", self.orig_msg(), crate::BUG_REPORTS_MESSAGE),
        }
    }
}

impl std::error::Error for JsError {}

/// Python's type name of a value (for error messages).
pub(super) fn type_name(value: &JsValue) -> &'static str {
    match value {
        JsValue::Undefined => "type",
        JsValue::Null => "NoneType",
        JsValue::Bool(_) => "bool",
        JsValue::Int(_) => "int",
        JsValue::Float(_) => "float",
        JsValue::Str(_) => "str",
        JsValue::List(_) => "list",
        JsValue::Object(_) => "dict",
        JsValue::Function(_) => "function_with_repr",
        JsValue::Error(error) => error.class_name(),
    }
}

/// Python truthiness (`a or 0`, `if not b`).
pub(super) fn py_truthy(value: &JsValue) -> bool {
    match value {
        JsValue::Null => false,
        JsValue::Bool(b) => *b,
        JsValue::Int(i) => *i != 0,
        JsValue::Float(f) => *f != 0.0,
        JsValue::Str(s) => !s.is_empty(),
        JsValue::List(items) => !items.borrow().is_empty(),
        JsValue::Object(pairs) => !pairs.borrow().is_empty(),
        JsValue::Undefined | JsValue::Function(_) | JsValue::Error(_) => true,
    }
}

/// `_js_ternary`'s condition: false for `False, None, 0, '', JS_Undefined`
/// and NaN.
pub(super) fn js_truthy(value: &JsValue) -> bool {
    match value {
        JsValue::Undefined | JsValue::Null | JsValue::Bool(false) | JsValue::Int(0) => false,
        JsValue::Float(f) => !(*f == 0.0 || f.is_nan()),
        JsValue::Str(s) => !s.is_empty(),
        _ => true,
    }
}

/// A Python number (`bool` is an `int`).
#[derive(Clone, Copy)]
pub(super) enum Num {
    I(i64),
    F(f64),
}

pub(super) fn num(value: &JsValue) -> Option<Num> {
    match value {
        JsValue::Bool(b) => Some(Num::I(i64::from(*b))),
        JsValue::Int(i) => Some(Num::I(*i)),
        JsValue::Float(f) => Some(Num::F(*f)),
        _ => None,
    }
}

impl Num {
    pub(super) fn float(self) -> f64 {
        match self {
            Self::I(i) => i as f64,
            Self::F(f) => f,
        }
    }
}

/// Python `==`.
pub fn py_eq(a: &JsValue, b: &JsValue) -> bool {
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return match (x, y) {
            (Num::I(x), Num::I(y)) => x == y,
            (x, y) => x.float() == y.float(),
        };
    }
    match (a, b) {
        (JsValue::Undefined, JsValue::Undefined) | (JsValue::Null, JsValue::Null) => true,
        (JsValue::Str(x), JsValue::Str(y)) => x == y,
        (JsValue::List(x), JsValue::List(y)) => {
            Rc::ptr_eq(x, y) || {
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| py_eq(a, b))
            }
        }
        (JsValue::Object(x), JsValue::Object(y)) => {
            Rc::ptr_eq(x, y) || {
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len()
                    && x.iter().all(|(key, value)| {
                        y.iter()
                            .find(|(other, _)| key_eq(key, other))
                            .is_some_and(|(_, other)| py_eq(value, other))
                    })
            }
        }
        (JsValue::Function(x), JsValue::Function(y)) => x.same(y),
        (JsValue::Error(x), JsValue::Error(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// Python `is`: identity of objects and strings (CPython's cached empty and
/// single Latin-1 character strings are one object each). Ints are the same
/// object within CPython's small-int cache (`-5..=1024` in the Python 3.15
/// the fixtures are captured with, `-5..=256` before); floats are separate
/// objects (as each literal and result is upstream; the same variable
/// compared with itself is not tracked).
pub(super) fn py_is(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Undefined, JsValue::Undefined) | (JsValue::Null, JsValue::Null) => true,
        (JsValue::Bool(x), JsValue::Bool(y)) => x == y,
        (JsValue::Int(x), JsValue::Int(y)) => x == y && (-5..=1024).contains(x),
        (JsValue::Float(_), JsValue::Float(_)) => false,
        (JsValue::Str(x), JsValue::Str(y)) => {
            Rc::ptr_eq(x, y)
                || (x == y && x.chars().nth(1).is_none() && x.chars().all(|c| c < '\u{100}'))
        }
        (JsValue::List(x), JsValue::List(y)) => Rc::ptr_eq(x, y),
        (JsValue::Object(x), JsValue::Object(y)) => Rc::ptr_eq(x, y),
        (JsValue::Function(x), JsValue::Function(y)) => x.same(y),
        (JsValue::Error(x), JsValue::Error(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// `hash(value)` succeeding: lists and dicts are unhashable.
pub(super) fn check_hashable(value: &JsValue) -> Result<(), JsError> {
    match value {
        JsValue::List(_) | JsValue::Object(_) => Err(JsError::type_error(format!(
            "unhashable type: '{}'",
            type_name(value)
        ))),
        _ => Ok(()),
    }
}

/// Equal `dict` keys (hash and `==`; NaN only matches itself by identity).
fn key_eq(a: &JsValue, b: &JsValue) -> bool {
    py_eq(a, b)
}

/// `d[key]` of a dict.
pub(super) fn object_get(pairs: &[(JsValue, JsValue)], key: &JsValue) -> Option<JsValue> {
    pairs
        .iter()
        .find(|(other, _)| key_eq(key, other))
        .map(|(_, value)| value.clone())
}

/// `d[key] = value` of a dict.
pub(super) fn object_set(pairs: &mut Vec<(JsValue, JsValue)>, key: JsValue, value: JsValue) {
    match pairs.iter_mut().find(|(other, _)| key_eq(&key, other)) {
        Some(pair) => pair.1 = value,
        None => pairs.push((key, value)),
    }
}

/// Python `repr(float)` (and `str(float)`).
pub fn py_float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    // Shortest round-trip digits and the decimal exponent.
    let formatted = format!("{:e}", value.abs());
    let (mantissa, exponent) = formatted.split_once('e').expect("exponent");
    let exponent: i32 = exponent.parse().expect("exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if value < 0.0 { "-" } else { "" };
    if (-4..16).contains(&exponent) {
        if exponent >= 0 {
            let point = exponent as usize + 1;
            let (int, frac) = if digits.len() > point {
                (digits[..point].to_owned(), digits[point..].to_owned())
            } else {
                (format!("{digits:0<point$}"), "0".to_owned())
            };
            format!("{sign}{int}.{frac}")
        } else {
            let zeros = "0".repeat((-exponent - 1) as usize);
            format!("{sign}0.{zeros}{digits}")
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        format!("{sign}{first}{rest}e{exp_sign}{:02}", exponent.abs())
    }
}

const UNDEFINED_REPR: &str = "<class 'yt_dlp.jsinterp.JS_Undefined'>";

/// Python `str(value)`.
pub fn py_str(value: &JsValue) -> String {
    match value {
        JsValue::Str(text) => text.to_string(),
        JsValue::Error(error) => error.to_string(),
        other => py_repr(other),
    }
}

/// Python `repr(value)`.
pub fn py_repr(value: &JsValue) -> String {
    let mut seen = Vec::new();
    repr_into(value, &mut seen)
}

fn repr_into(value: &JsValue, seen: &mut Vec<usize>) -> String {
    match value {
        JsValue::Undefined => UNDEFINED_REPR.into(),
        JsValue::Null => "None".into(),
        JsValue::Bool(true) => "True".into(),
        JsValue::Bool(false) => "False".into(),
        JsValue::Int(i) => i.to_string(),
        JsValue::Float(f) => py_float_repr(*f),
        JsValue::Str(text) => crate::batch::py_repr(text),
        JsValue::Function(function) => function.repr.to_string(),
        JsValue::Error(error) => error.repr(),
        JsValue::List(items) => {
            let id = Rc::as_ptr(items) as usize;
            if seen.contains(&id) {
                return "[...]".into();
            }
            seen.push(id);
            let parts: Vec<String> = items
                .borrow()
                .iter()
                .map(|item| repr_into(item, seen))
                .collect();
            seen.pop();
            format!("[{}]", parts.join(", "))
        }
        JsValue::Object(pairs) => {
            let id = Rc::as_ptr(pairs) as usize;
            if seen.contains(&id) {
                return "{...}".into();
            }
            seen.push(id);
            let parts: Vec<String> = pairs
                .borrow()
                .iter()
                .map(|(key, value)| format!("{}: {}", repr_into(key, seen), repr_into(value, seen)))
                .collect();
            seen.pop();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

/// Python `json.dumps(value)`: `None` for its `TypeError` (values JSON
/// cannot hold), an error for its circular reference `ValueError`.
pub(super) fn json_dumps(value: &JsValue) -> Result<Option<String>, JsError> {
    let mut seen = Vec::new();
    let mut out = String::new();
    Ok(dump_into(value, &mut out, &mut seen)?.then_some(out))
}

fn circular() -> JsError {
    JsError::python("ValueError", "Circular reference detected")
}

fn dump_into(value: &JsValue, out: &mut String, seen: &mut Vec<usize>) -> Result<bool, JsError> {
    match value {
        JsValue::Null => out.push_str("null"),
        JsValue::Bool(true) => out.push_str("true"),
        JsValue::Bool(false) => out.push_str("false"),
        JsValue::Int(i) => out.push_str(&i.to_string()),
        JsValue::Float(f) => out.push_str(&json_float(*f)),
        JsValue::Str(text) => dump_str(text, out),
        JsValue::List(items) => {
            let id = Rc::as_ptr(items) as usize;
            if seen.contains(&id) {
                return Err(circular());
            }
            seen.push(id);
            out.push('[');
            for (index, item) in items.borrow().iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                if !dump_into(item, out, seen)? {
                    return Ok(false);
                }
            }
            out.push(']');
            seen.pop();
        }
        JsValue::Object(pairs) => {
            let id = Rc::as_ptr(pairs) as usize;
            if seen.contains(&id) {
                return Err(circular());
            }
            seen.push(id);
            out.push('{');
            for (index, (key, item)) in pairs.borrow().iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                let key = match key {
                    JsValue::Str(text) => text.to_string(),
                    JsValue::Int(i) => i.to_string(),
                    JsValue::Float(f) => json_float(*f),
                    JsValue::Bool(b) => b.to_string(),
                    JsValue::Null => "null".into(),
                    _ => return Ok(false),
                };
                dump_str(&key, out);
                out.push_str(": ");
                if !dump_into(item, out, seen)? {
                    return Ok(false);
                }
            }
            out.push('}');
            seen.pop();
        }
        JsValue::Undefined | JsValue::Function(_) | JsValue::Error(_) => return Ok(false),
    }
    Ok(true)
}

fn json_float(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else {
        py_float_repr(value)
    }
}

/// A JSON string with `ensure_ascii`.
fn dump_str(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A `json.loads` result as interpreter values.
pub(super) fn from_json(value: serde_json::Value) -> JsValue {
    match value {
        serde_json::Value::Null => JsValue::Null,
        serde_json::Value::Bool(b) => JsValue::Bool(b),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(i) => JsValue::Int(i),
            None => JsValue::Float(number.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(text) => JsValue::str(text),
        serde_json::Value::Array(items) => {
            JsValue::list(items.into_iter().map(from_json).collect())
        }
        serde_json::Value::Object(map) => {
            JsValue::object(map.into_iter().map(|(key, value)| (key, from_json(value))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        for (value, expected) in [
            (1.0, "1.0"),
            (-4.0, "-4.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (1e15, "1000000000000000.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1234.1234, "1234.1234"),
            (f64::INFINITY, "inf"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(py_float_repr(value), expected);
        }
    }

    #[test]
    fn json_dumps_matches_python() {
        let value = JsValue::list(vec![
            JsValue::Int(1),
            JsValue::Float(2.0),
            JsValue::str("é\"\n"),
            JsValue::Null,
            JsValue::object([("a", JsValue::Bool(true))]),
            JsValue::Float(f64::NAN),
        ]);
        assert_eq!(
            json_dumps(&value).unwrap().unwrap(),
            r#"[1, 2.0, "\u00e9\"\n", null, {"a": true}, NaN]"#
        );
        assert!(json_dumps(&JsValue::Undefined).unwrap().is_none());
    }
}
