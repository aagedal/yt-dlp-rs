//! Upstream `yt_dlp/jsinterp.py`: `JSInterpreter`, the small interpreter
//! of the JavaScript subset some extractors run (VK's challenge salt).
//!
//! This follows upstream's string-rewriting evaluator, not JavaScript: a
//! statement is split at `;`, `,` and operators by `_separate`, values are
//! Python objects (see [`value`]) and every operator is upstream's `_js_*`
//! helper over Python's own (so `a % 3` has the divisor's sign, `===` is
//! `is`, `x.push(y)` returns the list, and `catch (e)` binds the name `(e)`).
//! Parsing quirks are kept: no `else if`, no comments, regex literals as
//! `PATTERN/FLAGS` strings, and only the `String`/`Math`/`Array` members
//! upstream knows.
//!
//! Not kept: Python integers are unbounded (past `i64` this uses floats),
//! complex powers, `str % x` formatting, and identity of floats and of ints
//! outside CPython's small-int cache (see [`value`]).
mod ops;
mod value;

pub use ops::{int_to_int32, js_number_to_string};
pub use value::{
    Frame, JsError, JsFunction, JsValue, Namespace, Scope, py_eq, py_float_repr, py_repr, py_str,
    scope,
};

use crate::batch::{is_py_space, py_strip};
use regex::Regex;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use value::{
    check_hashable, from_json, js_truthy, json_dumps, num, object_get, object_set, py_truthy,
    type_name,
};

type Result<T> = std::result::Result<T, JsError>;

/// `_OPERATORS` in precedence order (`None`-valued ones first).
const OPERATORS: [&str; 23] = [
    "?", "??", "||", "&&", "|", "^", "&", "===", "!==", "==", "!=", "<=", ">=", "<", ">", ">>",
    "<<", "+", "-", "*", "%", "/", "**",
];

/// `set(_OPERATORS) - _COMP_OPERATORS`: the compound assignment operators.
const ASSIGN_OPERATORS: [&str; 15] = [
    "?", "??", "||", "&&", "|", "^", "&", ">>", "<<", "+", "-", "*", "%", "/", "**",
];

/// `JSInterpreter._RE_FLAGS` (Python's `re` flag values for `i`, `m`, `s`
/// and `u`).
fn regex_flag(c: char) -> Option<u32> {
    Some(match c {
        'd' => 1024,
        'g' => 2048,
        'i' => 2,
        'm' => 8,
        's' => 16,
        'u' => 32,
        'y' => 4096,
        _ => return None,
    })
}

/// `JSInterpreter.Exception(msg, expr)`.
fn exception(message: impl AsRef<str>, expr: Option<&str>) -> JsError {
    exception_caused(message, expr, None)
}

fn exception_caused(
    message: impl AsRef<str>,
    expr: Option<&str>,
    cause: Option<JsError>,
) -> JsError {
    let message = message.as_ref();
    let message = match expr {
        Some(expr) => format!(
            "{} in: {}",
            message.trim_end_matches(is_py_space),
            truncate_string(expr, 50, 50)
        ),
        None => message.to_owned(),
    };
    JsError::Exception {
        message,
        cause: cause.map(Box::new),
    }
}

/// Upstream `truncate_string(s, left, right)`.
fn truncate_string(text: &str, left: usize, right: usize) -> Cow<'_, str> {
    let length = text.chars().count();
    if length <= left + right {
        return Cow::Borrowed(text);
    }
    let head: String = text.chars().take(left - 3).collect();
    let tail: String = text.chars().skip(length - right).collect();
    Cow::Owned(format!("{head}...{tail}"))
}

/// Python `s.lstrip()`.
fn py_lstrip(text: &str) -> &str {
    text.trim_start_matches(is_py_space)
}

/// The bytes of leading Python whitespace.
fn space_len(text: &str) -> usize {
    text.len() - py_lstrip(text).len()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The length of `_NAME_RE` (`[a-zA-Z_$][\w$]*`) at the start of `text`.
fn name_len(text: &str) -> usize {
    let mut chars = text.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return 0,
    }
    for (index, c) in chars {
        if !(is_word(c) || c == '$') {
            return index;
        }
    }
    text.len()
}

/// `(.*)$` (no dot-all): `text` without a final newline, if it has no other.
fn dollar(text: &str) -> Option<&str> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    (!body.contains('\n')).then_some(body)
}

/// The match lengths of `_NESTED_BRACKETS` at the start of `text`, in the
/// order Python's backtracking tries them.
fn nested_brackets(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let plain = |from: usize| {
        bytes[from.min(bytes.len())..]
            .iter()
            .take_while(|b| !matches!(b, b'[' | b']'))
            .count()
    };
    let run = plain(0);
    let mut ends = Vec::new();
    if run == 0 {
        return ends;
    }
    if bytes.get(run) == Some(&b'[') {
        let inner = run + 1;
        let inner_run = plain(inner);
        if inner_run > 0 {
            let after = inner + inner_run;
            if bytes.get(after) == Some(&b'[') {
                let deepest = after + 1;
                let deepest_run = bytes[deepest.min(bytes.len())..]
                    .iter()
                    .take_while(|b| **b != b']')
                    .count();
                let close = deepest + deepest_run;
                if deepest_run > 0 && bytes.get(close + 1) == Some(&b']') {
                    ends.push(close + 2);
                }
            }
            if bytes.get(after) == Some(&b']') {
                ends.push(after + 1);
            }
        }
    }
    ends.extend((1..=run).rev());
    ends
}

/// `_separate(expr, delim, max_split)`: `expr` split at top-level `delim`s
/// (outside brackets, strings and regex literals, and not a unary `+`/`-`).
fn separate<'a>(expr: &'a str, delim: &str, max_split: Option<usize>) -> Vec<&'a str> {
    const OP_CHARS: &str = "+-*/%&|^=<>!,;{}:[";
    #[derive(Clone, Copy, PartialEq)]
    enum AfterOp {
        True,
        False,
        Op,
    }
    let mut out = Vec::new();
    if expr.is_empty() {
        return out;
    }
    let delim: Vec<char> = delim.chars().collect();
    let delim_len = delim.len() - 1;
    let mut counters = [0usize; 3];
    let (mut start, mut splits, mut pos) = (0, 0, 0);
    let mut in_quote: Option<char> = None;
    let (mut escaping, mut after_op, mut in_regex_char_group) = (false, AfterOp::True, false);
    for (index, c) in expr.char_indices() {
        let opening = "({[".find(c);
        let closing = ")}]".find(c);
        if in_quote.is_none() && opening.is_some() {
            counters[opening.unwrap_or_default()] += 1;
        } else if in_quote.is_none() && closing.is_some() {
            let counter = &mut counters[closing.unwrap_or_default()];
            // Something's wrong if we get negative, but ignore it anyway.
            *counter = counter.saturating_sub(1);
        } else if !escaping {
            if "'\"/".contains(c) && (in_quote.is_none() || in_quote == Some(c)) {
                if in_quote.is_some() || after_op != AfterOp::False || c != '/' {
                    in_quote = if in_quote.is_some() && !in_regex_char_group {
                        None
                    } else {
                        Some(c)
                    };
                }
            } else if in_quote == Some('/') && (c == '[' || c == ']') {
                in_regex_char_group = c == '[';
            }
        }
        escaping = !escaping && in_quote.is_some() && c == '\\';
        let in_unary_op = in_quote.is_none()
            && !in_regex_char_group
            && after_op == AfterOp::Op
            && (c == '-' || c == '+');
        after_op = if in_quote.is_none() && OP_CHARS.contains(c) {
            AfterOp::Op
        } else if is_py_space(c) {
            after_op
        } else {
            AfterOp::False
        };
        if c != delim[pos] || counters.iter().any(|n| *n > 0) || in_quote.is_some() || in_unary_op {
            pos = 0;
            continue;
        } else if pos != delim_len {
            pos += 1;
            continue;
        }
        // The delimiter is ASCII: its characters are single bytes.
        out.push(&expr[start..(index - delim_len).max(start)]);
        start = index + 1;
        pos = 0;
        splits += 1;
        if max_split.is_some_and(|max| max > 0 && splits >= max) {
            break;
        }
    }
    out.push(&expr[start..]);
    out
}

/// Python `s[start:end]` by characters.
fn char_slice(text: &str, start: usize, end: Option<usize>) -> &str {
    let byte = |n: usize| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
    let start = byte(start);
    let end = end.map_or(text.len(), byte).max(start);
    &text[start..end]
}

#[derive(Clone, Copy, PartialEq)]
enum Keyword {
    Var,
    Return,
    Throw,
}

/// `(?P<var>(?:var|const|let)\s)|return(?:\s+|(?=["\'])|$)|(?P<throw>throw\s+)`.
fn match_keyword(stmt: &str) -> Option<(Keyword, usize)> {
    for keyword in ["var", "const", "let"] {
        if let Some(rest) = stmt.strip_prefix(keyword)
            && let Some(c) = rest.chars().next()
            && is_py_space(c)
        {
            return Some((Keyword::Var, keyword.len() + c.len_utf8()));
        }
    }
    if let Some(rest) = stmt.strip_prefix("return") {
        let spaces = space_len(rest);
        if spaces > 0 {
            return Some((Keyword::Return, 6 + spaces));
        }
        if rest.is_empty() || rest.starts_with(['"', '\'']) {
            return Some((Keyword::Return, 6));
        }
    }
    if let Some(rest) = stmt.strip_prefix("throw") {
        let spaces = space_len(rest);
        if spaces > 0 {
            return Some((Keyword::Throw, 5 + spaces));
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Control {
    Try,
    If,
    Switch,
    For,
}

/// `KEYWORD\s*OPEN` at the start of `text`: the offset of `OPEN`.
fn keyword_open(text: &str, keyword: &str, open: char) -> Option<usize> {
    let rest = text.strip_prefix(keyword)?;
    let spaces = space_len(rest);
    rest[spaces..]
        .starts_with(open)
        .then_some(keyword.len() + spaces)
}

/// `(?P<try>try)\s*\{|(?P<if>if)\s*\(|(?P<switch>switch)\s*\(|(?P<for>for)\s*\(`.
fn match_control(expr: &str) -> Option<(Control, usize)> {
    [
        (Control::Try, "try", '{'),
        (Control::If, "if", '('),
        (Control::Switch, "switch", '('),
        (Control::For, "for", '('),
    ]
    .into_iter()
    .find_map(|(control, keyword, open)| {
        keyword_open(expr, keyword, open).map(|offset| (control, offset))
    })
}

/// `catch\s*(?P<err>\(\s*NAME\s*\))?\{`: the `err` group and the offset of
/// the `{`.
fn match_catch(text: &str) -> Option<(Option<&str>, usize)> {
    let rest = text.strip_prefix("catch")?;
    let offset = 5 + space_len(rest);
    let after = &text[offset..];
    if let Some(inner) = after.strip_prefix('(') {
        let name_start = 1 + space_len(inner);
        let name = name_len(&after[name_start..]);
        if name > 0 {
            let close = name_start + name + space_len(&after[name_start + name..]);
            if after[close..].starts_with(')') && after[close + 1..].starts_with('{') {
                return Some((Some(&after[..close + 1]), offset + close + 1));
            }
        }
    }
    after.starts_with('{').then_some((None, offset))
}

/// The assignment regex: `NAME([NESTED])?\s*(OP)?=(?!=)(.*)$`.
struct Assignment<'a> {
    out: &'a str,
    index: Option<&'a str>,
    op: Option<&'static str>,
    expr: &'a str,
}

fn assignment_tail(text: &str) -> Option<(Option<&'static str>, &str)> {
    let text = &text[space_len(text)..];
    for op in ASSIGN_OPERATORS {
        if let Some(rest) = text.strip_prefix(op)
            && let Some(rest) = rest.strip_prefix('=')
            && !rest.starts_with('=')
        {
            return dollar(rest).map(|expr| (Some(op), expr));
        }
    }
    let rest = text.strip_prefix('=')?;
    if rest.starts_with('=') {
        return None;
    }
    dollar(rest).map(|expr| (None, expr))
}

fn match_assignment(expr: &str) -> Option<Assignment<'_>> {
    let n = name_len(expr);
    if n == 0 {
        return None;
    }
    let out = &expr[..n];
    if expr[n..].starts_with('[') {
        for end in nested_brackets(&expr[n + 1..]) {
            let close = n + 1 + end;
            if expr.as_bytes().get(close) == Some(&b']')
                && let Some((op, rhs)) = assignment_tail(&expr[close + 1..])
            {
                return Some(Assignment {
                    out,
                    index: Some(&expr[n + 1..close]),
                    op,
                    expr: rhs,
                });
            }
        }
    }
    assignment_tail(&expr[n..]).map(|(op, rhs)| Assignment {
        out,
        index: None,
        op,
        expr: rhs,
    })
}

/// One `(?P<pre_sign>\+\+|--)(?P<var1>NAME)|(?P<var2>NAME)(?P<post_sign>\+\+|--)`
/// match: character span, name, prefix and increment.
struct IncDec {
    start: usize,
    end: usize,
    var: String,
    pre: bool,
    plus: bool,
}

fn find_inc_dec(expr: &str) -> Vec<IncDec> {
    let chars: Vec<char> = expr.chars().collect();
    let name_at = |p: usize| -> usize {
        match chars.get(p) {
            Some(c) if c.is_ascii_alphabetic() || *c == '_' || *c == '$' => {}
            _ => return 0,
        }
        1 + chars[p + 1..]
            .iter()
            .take_while(|c| is_word(**c) || **c == '$')
            .count()
    };
    let sign_at = |p: usize| match (chars.get(p), chars.get(p + 1)) {
        (Some('+'), Some('+')) => Some(true),
        (Some('-'), Some('-')) => Some(false),
        _ => None,
    };
    let mut found = Vec::new();
    let mut p = 0;
    while p < chars.len() {
        if let Some(plus) = sign_at(p) {
            let n = name_at(p + 2);
            if n > 0 {
                found.push(IncDec {
                    start: p,
                    end: p + 2 + n,
                    var: chars[p + 2..p + 2 + n].iter().collect(),
                    pre: true,
                    plus,
                });
                p += 2 + n;
                continue;
            }
        }
        let n = name_at(p);
        if n > 0
            && let Some(plus) = sign_at(p + n)
        {
            found.push(IncDec {
                start: p,
                end: p + n + 2,
                var: chars[p..p + n].iter().collect(),
                pre: false,
                plus,
            });
            p += n + 2;
            continue;
        }
        p += 1;
    }
    found
}

/// The alternatives of the expression regex (`return`, `attribute`,
/// `indexing` and `function`).
enum Form<'a> {
    Name(&'a str),
    Attribute {
        var: &'a str,
        nullish: bool,
        member: Option<&'a str>,
        member2: Option<&'a str>,
        end: usize,
    },
    Indexing {
        name: &'a str,
        idx: &'a str,
    },
    Function {
        name: &'a str,
        args: &'a str,
    },
}

fn match_form(expr: &str) -> Option<Form<'_>> {
    let n = name_len(expr);
    if n == 0 {
        return None;
    }
    let reserved = ["if", "return", "true", "false", "null", "undefined", "NaN"];
    if !reserved.iter().any(|word| expr.starts_with(word)) && dollar(&expr[n..]) == Some("") {
        return Some(Form::Name(&expr[..n]));
    }
    let var = &expr[..n];
    let rest = &expr[n..];
    let dot = if rest.starts_with("?.") {
        Some((true, 2))
    } else if rest.starts_with('.') {
        Some((false, 1))
    } else {
        None
    };
    if let Some((nullish, dot)) = dot {
        let after = &rest[dot..];
        let member = after.find('(').unwrap_or(after.len());
        if member > 0 {
            return Some(Form::Attribute {
                var,
                nullish,
                member: Some(&after[..member]),
                member2: None,
                end: n + dot + member,
            });
        }
    }
    if let Some(inner) = rest.strip_prefix('[') {
        for end in nested_brackets(inner) {
            let close = n + 1 + end;
            if expr.as_bytes().get(close) == Some(&b']') {
                return Some(Form::Attribute {
                    var,
                    nullish: false,
                    member: None,
                    member2: Some(&expr[n + 1..close]),
                    end: close + 1 + space_len(&expr[close + 1..]),
                });
            }
        }
    }
    let enclosed = |open: char, close: char| {
        let body = rest.strip_prefix(open)?;
        let body = body.strip_suffix('\n').unwrap_or(body);
        let inner = body.strip_suffix(close)?;
        (!inner.contains('\n')).then_some(inner)
    };
    if let Some(idx) = enclosed('[', ']').filter(|idx| !idx.is_empty()) {
        return Some(Form::Indexing { name: var, idx });
    }
    enclosed('(', ')').map(|args| Form::Function { name: var, args })
}

/// `String`, `Math` and `Array` (Python's `str`, `float` and `list`).
#[derive(Clone, Copy, PartialEq)]
enum PyType {
    Str,
    Float,
    List,
}

impl PyType {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "String" => Self::Str,
            "Math" => Self::Float,
            "Array" => Self::List,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Str => "str",
            Self::Float => "float",
            Self::List => "list",
        }
    }

    fn instance(self, value: &JsValue) -> bool {
        matches!(
            (self, value),
            (Self::Str, JsValue::Str(_))
                | (Self::Float, JsValue::Float(_))
                | (Self::List, JsValue::List(_))
        )
    }
}

/// What a member is looked up on.
enum Target {
    Value(JsValue),
    Type(PyType),
}

/// Python `int(value)`.
fn py_int(value: &JsValue) -> Result<i64> {
    match value {
        JsValue::Bool(b) => Ok(i64::from(*b)),
        JsValue::Int(i) => Ok(*i),
        JsValue::Float(f) if f.is_nan() => Err(JsError::python(
            "ValueError",
            "cannot convert float NaN to integer",
        )),
        JsValue::Float(f) if f.is_infinite() => Err(JsError::python(
            "OverflowError",
            "cannot convert float infinity to integer",
        )),
        JsValue::Float(f) => Ok(f.trunc() as i64),
        JsValue::Str(text) => {
            let trimmed = py_strip(text);
            let digits = trimmed.trim_start_matches(['+', '-']);
            let valid = !digits.is_empty()
                && !digits.starts_with('_')
                && !digits.ends_with('_')
                && !digits.contains("__")
                && digits.chars().all(|c| c.is_ascii_digit() || c == '_')
                && trimmed.len() - digits.len() <= 1;
            valid
                .then(|| trimmed.replace('_', "").parse().ok())
                .flatten()
                .ok_or_else(|| {
                    JsError::python(
                        "ValueError",
                        format!(
                            "invalid literal for int() with base 10: {}",
                            crate::batch::py_repr(text)
                        ),
                    )
                })
        }
        other => Err(JsError::type_error(format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            type_name(other)
        ))),
    }
}

/// Python `seq[index]` with negative indexes.
fn py_index(length: usize, index: i64) -> Option<usize> {
    let index = if index < 0 {
        index + length as i64
    } else {
        index
    };
    (0..length as i64)
        .contains(&index)
        .then_some(index as usize)
}

/// `slice(*argvals, None)`'s indices for a sequence of `length`.
fn py_slice(args: &[JsValue], length: usize) -> Result<(usize, usize)> {
    let bound = |value: Option<&JsValue>, default: i64| -> Result<i64> {
        match value {
            None | Some(JsValue::Null) => Ok(default),
            Some(JsValue::Int(i)) => Ok(*i),
            Some(JsValue::Bool(b)) => Ok(i64::from(*b)),
            Some(_) => Err(JsError::type_error(
                "slice indices must be integers or None or have an __index__ method",
            )),
        }
    };
    let length = length as i64;
    let clamp = |i: i64| {
        if i < 0 {
            (i + length).max(0)
        } else {
            i.min(length)
        }
    };
    // `slice(x)` is `slice(None, x)`; `slice(x, None)` starts at `x`.
    let (start, stop) = match args.len() {
        0 => (0, length),
        1 => (clamp(bound(args.first(), 0)?), length),
        _ => (
            clamp(bound(args.first(), 0)?),
            clamp(bound(args.get(1), length)?),
        ),
    };
    Ok((start as usize, stop.max(start) as usize))
}

/// The right-hand side of `?` when `:` is missing (`_js_ternary`'s
/// `True`/`False` defaults).
enum Branch<'a> {
    Expr(&'a str),
    Bool(bool),
}

/// Upstream `JSInterpreter`.
pub struct JsInterpreter {
    code: String,
    functions: RefCell<HashMap<String, JsValue>>,
    objects: RefCell<HashMap<String, JsValue>>,
    undefined_varnames: RefCell<BTreeSet<String>>,
    named_object_counter: Cell<u64>,
}

impl JsInterpreter {
    pub fn new(code: impl Into<String>) -> Self {
        Self::with_objects(code, HashMap::new())
    }

    /// `JSInterpreter(code, objects)`.
    pub fn with_objects(code: impl Into<String>, objects: HashMap<String, JsValue>) -> Self {
        Self {
            code: code.into(),
            functions: RefCell::new(HashMap::new()),
            objects: RefCell::new(objects),
            undefined_varnames: RefCell::new(BTreeSet::new()),
            named_object_counter: Cell::new(0),
        }
    }

    /// `_undefined_varnames`: names read without being defined.
    pub fn undefined_varnames(&self) -> BTreeSet<String> {
        self.undefined_varnames.borrow().clone()
    }

    fn next_object_name(&self) -> (u64, String) {
        let counter = self.named_object_counter.get() + 1;
        self.named_object_counter.set(counter);
        (counter, format!("__yt_dlp_jsinterp_obj{counter}"))
    }

    /// `_named_object(namespace, obj)`: functions get the `F<N>` repr.
    fn named(&self, obj: JsValue) -> (String, JsValue) {
        let (counter, name) = self.next_object_name();
        let obj = match obj {
            JsValue::Function(function) if function.repr.is_empty() => {
                JsValue::Function(JsFunction {
                    repr: format!("F<{counter}>").into(),
                    ..function
                })
            }
            other => other,
        };
        (name, obj)
    }

    fn named_object(&self, namespace: &Namespace, obj: JsValue) -> String {
        let (name, obj) = self.named(obj);
        namespace.set(&name, obj);
        name
    }

    /// `_dump(obj, namespace)`: JSON text, or a named object.
    fn dump(&self, obj: &JsValue, namespace: &Namespace) -> Result<String> {
        Ok(match json_dumps(obj)? {
            Some(text) => text,
            None => self.named_object(namespace, obj.clone()),
        })
    }

    /// `_separate_at_paren(expr, delim)`: the bracketed head (without its
    /// brackets) and the rest, both stripped.
    fn separate_at_paren(expr: &str, delim: Option<char>) -> Result<(&str, &str)> {
        let delim = match delim {
            Some(delim) => delim.to_string(),
            None => match expr.chars().next() {
                None => String::new(),
                Some('(') => ")".into(),
                Some('{') => "}".into(),
                Some('[') => "]".into(),
                Some(other) => {
                    return Err(JsError::python(
                        "KeyError",
                        crate::batch::py_repr(&other.to_string()),
                    ));
                }
            },
        };
        let separated = separate(expr, &delim, Some(1));
        if separated.len() < 2 {
            return Err(exception(
                format!("No terminating paren {delim}"),
                Some(expr),
            ));
        }
        let head = separated[0];
        let head = head
            .char_indices()
            .nth(1)
            .map_or("", |(index, _)| &head[index..]);
        Ok((py_strip(head), py_strip(separated[1])))
    }

    /// `_operator(op, left_val, right_expr, expr, local_vars, allow_recursion)`.
    fn operator(
        &self,
        op: Option<&str>,
        left_val: JsValue,
        right_expr: &str,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<JsValue> {
        let mut branch = Branch::Expr(right_expr);
        match op {
            Some("||") | Some("&&") => {
                if (op == Some("&&")) ^ js_truthy(&left_val) {
                    return Ok(left_val); // short circuiting
                }
            }
            Some("??") => {
                if !matches!(left_val, JsValue::Null | JsValue::Undefined) {
                    return Ok(left_val);
                }
            }
            Some("?") => {
                let parts = separate(right_expr, ":", Some(1));
                branch = match (js_truthy(&left_val), parts.first(), parts.get(1)) {
                    (true, Some(part), _) | (false, _, Some(part)) => Branch::Expr(part),
                    (truthy, _, _) => Branch::Bool(truthy),
                };
            }
            _ => {}
        }
        let right_val = match branch {
            Branch::Expr(right_expr) => {
                self.interpret_expression(right_expr, local_vars, allow_recursion)?
            }
            // Interpreting `False` is interpreting nothing; `True` is not
            // iterable.
            Branch::Bool(false) => JsValue::Null,
            Branch::Bool(true) => {
                return Err(JsError::type_error("'bool' object is not iterable"));
            }
        };
        let Some(op) = op.filter(|op| !matches!(*op, "?" | "??" | "||" | "&&")) else {
            return Ok(right_val);
        };
        // TODO upstream: only correct for str+str and str+number.
        if op == "+"
            && (matches!(left_val, JsValue::Str(_)) || matches!(right_val, JsValue::Str(_)))
        {
            return Ok(JsValue::str(format!(
                "{}{}",
                py_str(&left_val),
                py_str(&right_val)
            )));
        }
        ops::apply(op, &left_val, &right_val).map_err(|cause| {
            exception_caused(
                format!(
                    "Failed to evaluate {} {op} {}",
                    py_repr(&left_val),
                    py_repr(&right_val)
                ),
                Some(expr),
                Some(cause),
            )
        })
    }

    /// `_index(obj, idx, allow_undefined)`.
    fn index(&self, obj: &JsValue, idx: &JsValue, allow_undefined: bool) -> Result<JsValue> {
        if idx.as_str() == Some("length") {
            return match obj {
                JsValue::Str(text) => Ok(JsValue::Int(text.chars().count() as i64)),
                JsValue::List(items) => Ok(JsValue::Int(items.borrow().len() as i64)),
                JsValue::Object(pairs) => Ok(JsValue::Int(pairs.borrow().len() as i64)),
                other => Err(JsError::type_error(format!(
                    "object of type '{}' has no len()",
                    type_name(other)
                ))),
            };
        }
        let found = match obj {
            JsValue::List(items) => py_int(idx).and_then(|index| {
                let items = items.borrow();
                py_index(items.len(), index)
                    .map(|index| items[index].clone())
                    .ok_or_else(|| JsError::python("IndexError", "list index out of range"))
            }),
            JsValue::Object(pairs) => {
                let key = JsValue::str(py_str(idx));
                object_get(&pairs.borrow(), &key)
                    .ok_or_else(|| JsError::python("KeyError", py_repr(&key)))
            }
            JsValue::Str(_) => Err(JsError::type_error(
                "string indices must be integers, not 'str'",
            )),
            other => Err(JsError::type_error(format!(
                "'{}' object is not subscriptable",
                type_name(other)
            ))),
        };
        found.or_else(|cause| {
            if allow_undefined {
                Ok(JsValue::Undefined)
            } else {
                Err(exception_caused(
                    format!("Cannot get index {}", py_str(idx)),
                    Some(&py_repr(obj)),
                    Some(cause),
                ))
            }
        })
    }

    /// `interpret_statement(stmt, local_vars, allow_recursion)`: the value and
    /// whether the statement returns.
    pub fn interpret_statement(
        &self,
        stmt: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<(JsValue, bool)> {
        self.statement(stmt, local_vars, allow_recursion, false)
    }

    /// `interpret_expression(expr, local_vars, allow_recursion)`.
    pub fn interpret_expression(
        &self,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<JsValue> {
        let (ret, should_return) = self.statement(expr, local_vars, allow_recursion, false)?;
        if should_return {
            return Err(exception("Cannot return from an expression", Some(expr)));
        }
        Ok(ret)
    }

    fn statement(
        &self,
        stmt: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
        is_var_declaration: bool,
    ) -> Result<(JsValue, bool)> {
        if allow_recursion < 0 {
            return Err(exception("Recursion limit reached", None));
        }
        let allow_recursion = allow_recursion - 1;
        let mut sub_statements = separate(stmt, ";", None);
        let last = sub_statements.pop().unwrap_or("");
        for sub_stmt in sub_statements {
            let (ret, should_return) =
                self.statement(sub_stmt, local_vars, allow_recursion, false)?;
            if should_return {
                return Ok((ret, should_return));
            }
        }
        self.single_statement(
            py_strip(last),
            local_vars,
            allow_recursion,
            is_var_declaration,
        )
    }

    /// The last of a statement's `;`-separated parts.
    fn single_statement(
        &self,
        stmt: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
        mut is_var_declaration: bool,
    ) -> Result<(JsValue, bool)> {
        let mut should_return = false;
        let mut expr = stmt.to_owned();
        if let Some((keyword, length)) = match_keyword(stmt) {
            expr = py_strip(&stmt[length..]).to_owned();
            if keyword == Keyword::Throw {
                let thrown = self.interpret_expression(&expr, local_vars, allow_recursion)?;
                return Err(JsError::Throw(thrown));
            }
            should_return = keyword != Keyword::Var;
            is_var_declaration = is_var_declaration || keyword == Keyword::Var;
        }
        if expr.is_empty() {
            return Ok((JsValue::Null, should_return));
        }

        let quote = expr.chars().next().filter(|c| "'\"/".contains(*c));
        if let Some(quote) = quote {
            let parts = separate(&expr, &quote.to_string(), Some(1));
            let [inner, outer] = parts[..] else {
                return Err(JsError::python(
                    "ValueError",
                    "not enough values to unpack (expected 2, got 1)",
                ));
            };
            let (value, outer) = if quote == '/' {
                let (flags, outer) = regex_flags(outer);
                // We don't support regex methods yet, so no point compiling it.
                (
                    JsValue::str(format!("{inner}/{}", regex_flags_str(flags))),
                    outer,
                )
            } else {
                (json_literal(&format!("{inner}{quote}"))?, outer)
            };
            if outer.is_empty() {
                return Ok((value, should_return));
            }
            expr = self.named_object(local_vars, value) + outer;
        }

        if let Some(obj) = expr.strip_prefix("new ") {
            if let Some(date) = obj.strip_prefix("Date") {
                if date.starts_with('(') {
                    let (left, right) = Self::separate_at_paren(date, None)?;
                    let value = self.interpret_expression(left, local_vars, allow_recursion)?;
                    let date = value
                        .as_str()
                        .and_then(|text| crate::dates::unified_timestamp(text, false));
                    let Some(date) = date else {
                        return Err(exception(
                            format!("Failed to parse date {}", crate::batch::py_repr(left)),
                            Some(&expr),
                        ));
                    };
                    expr = self.dump(&JsValue::Int(date * 1000), local_vars)? + right;
                } else {
                    return Err(exception(format!("Unsupported object {obj}"), Some(&expr)));
                }
            } else {
                return Err(exception(format!("Unsupported object {obj}"), Some(&expr)));
            }
        }

        if let Some(rest) = expr.strip_prefix("void ") {
            self.interpret_expression(rest, local_vars, allow_recursion)?;
            return Ok((JsValue::Null, should_return));
        }

        if expr.starts_with('{') {
            let (inner, outer) = Self::separate_at_paren(&expr, None)?;
            // Try for object expression (Map).
            let sub_expressions: Vec<Vec<&str>> = separate(inner, ",", None)
                .into_iter()
                .map(|sub_expr| separate(py_strip(sub_expr), ":", Some(1)))
                .collect();
            if sub_expressions.iter().all(|sub_expr| sub_expr.len() == 2) {
                let mut pairs = Vec::new();
                for sub_expr in &sub_expressions {
                    let (key, value) = (sub_expr[0], sub_expr[1]);
                    let value = self.interpret_expression(value, local_vars, allow_recursion)?;
                    let key = if name_len(key) > 0 {
                        JsValue::str(key)
                    } else {
                        self.interpret_expression(key, local_vars, allow_recursion)?
                    };
                    check_hashable(&key)?;
                    object_set(&mut pairs, key, value);
                }
                return Ok((JsValue::Object(Rc::new(RefCell::new(pairs))), should_return));
            }
            let (inner, should_abort) =
                self.statement(inner, local_vars, allow_recursion, false)?;
            if outer.is_empty() || should_abort {
                return Ok((inner, should_abort || should_return));
            }
            expr = self.dump(&inner, local_vars)? + outer;
        }

        if expr.starts_with('(') {
            let (inner, outer) = Self::separate_at_paren(&expr, None)?;
            let (inner, should_abort) =
                self.statement(inner, local_vars, allow_recursion, false)?;
            if outer.is_empty() || should_abort {
                return Ok((inner, should_abort || should_return));
            }
            expr = self.dump(&inner, local_vars)? + outer;
        }

        if expr.starts_with('[') {
            let (inner, outer) = Self::separate_at_paren(&expr, None)?;
            let items = separate(inner, ",", None)
                .into_iter()
                .map(|item| self.interpret_expression(item, local_vars, allow_recursion))
                .collect::<Result<Vec<_>>>()?;
            expr = self.named_object(local_vars, JsValue::list(items)) + outer;
        }

        if let Some((control, open)) = match_control(&expr) {
            let rest = match control {
                Control::If => self.if_statement(&expr[open..], local_vars, allow_recursion)?,
                Control::Try => self.try_statement(&expr[open..], local_vars, allow_recursion)?,
                Control::For => self.for_statement(&expr[open..], local_vars, allow_recursion)?,
                Control::Switch => {
                    self.switch_statement(&expr[open..], local_vars, allow_recursion)?
                }
            };
            return match rest {
                Err(done) => Ok(done),
                Ok(rest) => {
                    let (ret, should_abort) =
                        self.statement(&rest, local_vars, allow_recursion, false)?;
                    Ok((ret, should_abort || should_return))
                }
            };
        }

        // Comma separated statements.
        let sub_expressions = separate(&expr, ",", None);
        if sub_expressions.len() > 1 {
            let mut ret = JsValue::Null;
            for sub_expr in sub_expressions {
                let (value, should_abort) =
                    self.statement(sub_expr, local_vars, allow_recursion, is_var_declaration)?;
                if should_abort {
                    return Ok((value, true));
                }
                ret = value;
            }
            return Ok((ret, false));
        }

        if let Some(assignment) = match_assignment(&expr) {
            return self
                .assign(
                    &assignment,
                    &expr,
                    local_vars,
                    allow_recursion,
                    is_var_declaration,
                )
                .map(|value| (value, should_return));
        }

        let original = expr.clone();
        for found in find_inc_dec(&original) {
            let mut ret = local_vars.getitem(&found.var)?;
            let step = JsValue::Int(if found.plus { 1 } else { -1 });
            local_vars.set(&found.var, ops::py_add(&ret, &step)?);
            if found.pre {
                ret = local_vars.getitem(&found.var)?;
            }
            let dumped = self.dump(&ret, local_vars)?;
            expr = format!(
                "{}{dumped}{}",
                char_slice(&expr, 0, Some(found.start)),
                char_slice(&expr, found.end, None)
            );
        }
        if expr.is_empty() {
            return Ok((JsValue::Null, should_return));
        }

        self.expression_value(&expr, stmt, local_vars, allow_recursion, is_var_declaration)
            .map(|(value, should_abort)| (value, should_abort || should_return))
    }

    /// `if (...) {...} else {...}`: `Err` with the result of a returning
    /// branch, else the statement after it.
    fn if_statement(
        &self,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<std::result::Result<String, (JsValue, bool)>> {
        let (condition, rest) = Self::separate_at_paren(expr, None)?;
        let (if_expr, mut rest) = Self::separate_at_paren(py_lstrip(rest), None)?;
        // TODO upstream: "else if" is not handled.
        let mut else_expr = "";
        if let Some(open) = keyword_open(rest, "else", '{') {
            (else_expr, rest) = Self::separate_at_paren(&rest[open..], None)?;
        }
        let condition =
            js_truthy(&self.interpret_expression(condition, local_vars, allow_recursion)?);
        let branch = if condition { if_expr } else { else_expr };
        let (ret, should_abort) = self.statement(branch, local_vars, allow_recursion, false)?;
        if should_abort {
            return Ok(Err((ret, true)));
        }
        Ok(Ok(rest.to_owned()))
    }

    /// `try {...} catch (e) {...} finally {...}`.
    fn try_statement(
        &self,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<std::result::Result<String, (JsValue, bool)>> {
        let (try_expr, mut rest) = Self::separate_at_paren(expr, None)?;
        let mut err = None;
        match self.statement(try_expr, local_vars, allow_recursion, false) {
            Ok((ret, true)) => return Ok(Err((ret, true))),
            Ok(_) => {}
            // XXX upstream: this works for now, but makes debugging future
            // issues very hard.
            Err(error) => err = Some(error),
        }
        let mut pending = (JsValue::Null, false);
        if let Some((name, open)) = match_catch(rest) {
            let sub_expr;
            (sub_expr, rest) = Self::separate_at_paren(&rest[open..], None)?;
            if let Some(error) = err.take() {
                let catch_vars = scope::<String>([]);
                if let Some(name) = name {
                    let value = match error {
                        JsError::Throw(value) => value,
                        other => JsValue::Error(Rc::new(other)),
                    };
                    catch_vars.borrow_mut().insert(name.to_owned(), value);
                }
                let catch_vars = local_vars.new_child(catch_vars);
                pending = self.statement(sub_expr, &catch_vars, allow_recursion, false)?;
            }
        }
        if let Some(open) = keyword_open(rest, "finally", '{') {
            let sub_expr;
            (sub_expr, rest) = Self::separate_at_paren(&rest[open..], None)?;
            let (ret, should_abort) =
                self.statement(sub_expr, local_vars, allow_recursion, false)?;
            if should_abort {
                return Ok(Err((ret, true)));
            }
        }
        if pending.1 {
            return Ok(Err((pending.0, true)));
        }
        if let Some(error) = err {
            return Err(error);
        }
        Ok(Ok(rest.to_owned()))
    }

    /// `for (start; condition; increment) body`.
    fn for_statement(
        &self,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<std::result::Result<String, (JsValue, bool)>> {
        let (constructor, remaining) = Self::separate_at_paren(expr, None)?;
        let (body, rest) = if remaining.starts_with('{') {
            let (body, rest) = Self::separate_at_paren(remaining, None)?;
            (body.to_owned(), rest)
        } else if let Some(open) = keyword_open(remaining, "switch", '(') {
            // FIXME upstream: ?
            let (switch_val, remaining) = Self::separate_at_paren(&remaining[open..], None)?;
            let (body, rest) = Self::separate_at_paren(remaining, Some('}'))?;
            (format!("switch({switch_val}){{{body}}}"), rest)
        } else {
            (remaining.to_owned(), "")
        };
        let parts = separate(constructor, ";", None);
        let [start, condition, increment] = parts[..] else {
            return Err(unpack_error(3, parts.len()));
        };
        self.interpret_expression(start, local_vars, allow_recursion)?;
        loop {
            let condition = self.interpret_expression(condition, local_vars, allow_recursion)?;
            if !js_truthy(&condition) {
                break;
            }
            match self.statement(&body, local_vars, allow_recursion, false) {
                Ok((ret, true)) => return Ok(Err((ret, true))),
                Ok(_) | Err(JsError::Continue) => {}
                Err(JsError::Break) => break,
                Err(error) => return Err(error),
            }
            self.interpret_expression(increment, local_vars, allow_recursion)?;
        }
        Ok(Ok(rest.to_owned()))
    }

    /// `switch (value) { case ...: ... default: ... }`.
    fn switch_statement(
        &self,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<std::result::Result<String, (JsValue, bool)>> {
        let (switch_val, remaining) = Self::separate_at_paren(expr, None)?;
        let switch_val = self.interpret_expression(switch_val, local_vars, allow_recursion)?;
        let (body, rest) = Self::separate_at_paren(remaining, Some('}'))?;
        let body = body.replace("default:", "case default:");
        let items: Vec<&str> = body.split("case ").skip(1).collect();
        for default in [false, true] {
            let mut matched = false;
            for item in &items {
                let parts = separate(item, ":", Some(1));
                let [case, stmt] = parts[..] else {
                    return Err(unpack_error(2, parts.len()));
                };
                let (case, stmt) = (py_strip(case), py_strip(stmt));
                if default {
                    matched = matched || case == "default";
                } else if !matched {
                    matched = case != "default"
                        && py_eq(
                            &switch_val,
                            &self.interpret_expression(case, local_vars, allow_recursion)?,
                        );
                }
                if !matched {
                    continue;
                }
                match self.statement(stmt, local_vars, allow_recursion, false) {
                    // Upstream returns the bare value here, which its
                    // `Debugger.wrap_interpreter` then unpacks as a pair.
                    Ok((ret, true)) => return Ok(Err(unpack_pair(ret)?)),
                    Ok(_) => {}
                    Err(JsError::Break) => break,
                    Err(error) => return Err(error),
                }
            }
            if matched {
                break;
            }
        }
        Ok(Ok(rest.to_owned()))
    }

    fn assign(
        &self,
        assignment: &Assignment<'_>,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
        is_var_declaration: bool,
    ) -> Result<JsValue> {
        let out = assignment.out;
        let left_val = local_vars.get(out).unwrap_or(JsValue::Null);
        let Some(index) = assignment.index else {
            let value = self.operator(
                assignment.op,
                left_val,
                assignment.expr,
                expr,
                local_vars,
                allow_recursion,
            )?;
            if is_var_declaration {
                local_vars.set_local(out, value);
            } else {
                local_vars.set(out, value);
            }
            return local_vars.getitem(out);
        };
        if matches!(left_val, JsValue::Null | JsValue::Undefined) {
            return Err(exception(
                format!("Cannot index undefined variable {out}"),
                Some(expr),
            ));
        }
        let idx = self.interpret_expression(index, local_vars, allow_recursion)?;
        if num(&idx).is_none() {
            return Err(exception(
                format!("List index {} must be integer", py_str(&idx)),
                Some(expr),
            ));
        }
        let idx = py_int(&idx)?;
        let current = self.index(&left_val, &JsValue::Int(idx), false)?;
        let value = self.operator(
            assignment.op,
            current,
            assignment.expr,
            expr,
            local_vars,
            allow_recursion,
        )?;
        match &left_val {
            JsValue::List(items) => {
                let mut items = items.borrow_mut();
                let Some(slot) = py_index(items.len(), idx) else {
                    return Err(JsError::python(
                        "IndexError",
                        "list assignment index out of range",
                    ));
                };
                items[slot] = value.clone();
            }
            JsValue::Object(pairs) => {
                object_set(&mut pairs.borrow_mut(), JsValue::Int(idx), value.clone());
            }
            other => {
                return Err(JsError::type_error(format!(
                    "'{}' object does not support item assignment",
                    type_name(other)
                )));
            }
        }
        Ok(value)
    }

    /// The rest of `interpret_statement`: literals, names, operators,
    /// members and calls.
    fn expression_value(
        &self,
        expr: &str,
        stmt: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
        is_var_declaration: bool,
    ) -> Result<(JsValue, bool)> {
        let form = match_form(expr);
        if !expr.is_empty() && expr.bytes().all(|b| b.is_ascii_digit()) {
            let value = expr.parse().map_or_else(
                |_| JsValue::Float(expr.parse().unwrap_or(f64::INFINITY)),
                JsValue::Int,
            );
            return Ok((value, false));
        }
        match expr {
            "break" => return Err(JsError::Break),
            "continue" => return Err(JsError::Continue),
            "undefined" => return Ok((JsValue::Undefined, false)),
            "NaN" => return Ok((JsValue::Float(f64::NAN), false)),
            _ => {}
        }
        if let Some(Form::Name(var)) = form {
            // Declared variables.
            if is_var_declaration {
                let ret = local_vars.get_local(var);
                // Register varname in local namespace; set value as
                // JS_Undefined or its pre-existing value.
                local_vars.set_local(var, ret.clone());
                return Ok((ret, false));
            }
            let ret = local_vars.get(var).unwrap_or_else(|| {
                self.undefined_varnames.borrow_mut().insert(var.to_owned());
                JsValue::Undefined
            });
            return Ok((ret, false));
        }

        if let Some(value) = crate::js_to_json::js_to_json_strict(expr)
            .as_deref()
            .and_then(json_loads)
        {
            return Ok((value, false));
        }

        if let Some(Form::Indexing { name, idx }) = form {
            let value = local_vars.getitem(name)?;
            let idx = self.interpret_expression(idx, local_vars, allow_recursion)?;
            return Ok((self.index(&value, &idx, false)?, false));
        }

        for op in OPERATORS {
            let mut separated: Vec<String> = separate(expr, op, None)
                .into_iter()
                .map(str::to_owned)
                .collect();
            let mut right_expr = separated.pop().unwrap_or_default();
            loop {
                if "?<>*-".contains(op)
                    && separated.len() > 1
                    && separated
                        .last()
                        .is_some_and(|last| py_strip(last).is_empty())
                {
                    separated.pop();
                } else if !(!separated.is_empty() && op == "?" && right_expr.starts_with('.')) {
                    break;
                }
                right_expr = format!("{op}{right_expr}");
                if op != "-" {
                    right_expr = format!("{}{op}{right_expr}", separated.pop().unwrap_or_default());
                }
            }
            if separated.is_empty() {
                continue;
            }
            let left_val =
                self.interpret_expression(&separated.join(op), local_vars, allow_recursion)?;
            let value = self.operator(
                Some(op),
                left_val,
                &right_expr,
                expr,
                local_vars,
                allow_recursion,
            )?;
            return Ok((value, false));
        }

        match form {
            Some(Form::Attribute {
                var,
                nullish,
                member,
                member2,
                end,
            }) => {
                let member = match member {
                    Some(member) => JsValue::str(member),
                    None => self.interpret_expression(
                        member2.unwrap_or_default(),
                        local_vars,
                        allow_recursion,
                    )?,
                };
                let rest = &expr[end..];
                let (arg_str, remaining) = if rest.starts_with('(') {
                    let (args, remaining) = Self::separate_at_paren(rest, None)?;
                    (Some(args), remaining)
                } else {
                    (None, rest)
                };
                let value = self.eval_method(
                    var,
                    member,
                    nullish,
                    arg_str,
                    expr,
                    local_vars,
                    allow_recursion,
                )?;
                if remaining.is_empty() {
                    return Ok((value, false));
                }
                let name = self.named_object(local_vars, value);
                self.statement(&(name + remaining), local_vars, allow_recursion, false)
            }
            Some(Form::Function { name, args }) => {
                let argvals = separate(args, ",", None)
                    .into_iter()
                    .map(|arg| self.interpret_expression(arg, local_vars, allow_recursion))
                    .collect::<Result<Vec<_>>>()?;
                let function = match local_vars.get(name) {
                    Some(function) => function,
                    None => {
                        let cached = self.functions.borrow().get(name).cloned();
                        match cached {
                            Some(function) => function,
                            None => {
                                let function = self.extract_function(name)?;
                                self.functions
                                    .borrow_mut()
                                    .insert(name.to_owned(), function.clone());
                                function
                            }
                        }
                    }
                };
                Ok((
                    self.call_value(&function, argvals, &[], allow_recursion)?,
                    false,
                ))
            }
            _ => {
                let shown = if expr == stmt {
                    Cow::Borrowed("")
                } else {
                    truncate_string(expr, 20, 20)
                };
                Err(exception(
                    format!("Unsupported JS expression {shown}"),
                    Some(stmt),
                ))
            }
        }
    }

    /// `eval_method()`: a member of `var`, called with `arg_str` if given.
    #[allow(clippy::too_many_arguments)]
    fn eval_method(
        &self,
        var: &str,
        mut member: JsValue,
        nullish: bool,
        arg_str: Option<&str>,
        expr: &str,
        local_vars: &Namespace,
        allow_recursion: i32,
    ) -> Result<JsValue> {
        let assertion = |condition: bool, member: &JsValue, message: &str| {
            if condition {
                Ok(())
            } else {
                Err(exception(
                    format!("{} {message}", py_str(member)),
                    Some(expr),
                ))
            }
        };
        if var == "console" && member.as_str() == Some("debug") {
            return Ok(JsValue::Null);
        }
        let mut obj = match local_vars.get(var) {
            Some(value) => Target::Value(value),
            None => match PyType::named(var) {
                Some(py_type) => Target::Type(py_type),
                None => {
                    let known = self.objects.borrow().contains_key(var);
                    if !known {
                        let stack = vec![Frame::Chain(Rc::new(local_vars.0.clone()))];
                        match self.extract_object(var, &stack) {
                            Ok(object) => {
                                self.objects.borrow_mut().insert(var.to_owned(), object);
                            }
                            Err(JsError::Exception { .. }) if nullish => {}
                            Err(error) => return Err(error),
                        }
                    }
                    Target::Value(
                        self.objects
                            .borrow()
                            .get(var)
                            .cloned()
                            .unwrap_or(JsValue::Undefined),
                    )
                }
            },
        };
        if nullish && matches!(obj, Target::Value(JsValue::Undefined)) {
            return Ok(JsValue::Undefined);
        }

        // Member access.
        let Some(arg_str) = arg_str else {
            return match obj {
                Target::Value(value) => self.index(&value, &member, nullish),
                Target::Type(py_type) => Err(exception(
                    format!("Cannot get index {}", py_str(&member)),
                    Some(&format!("<class '{}'>", py_type.name())),
                )),
            };
        };

        // Function call.
        let mut argvals = separate(arg_str, ",", None)
            .into_iter()
            .map(|arg| self.interpret_expression(arg, local_vars, allow_recursion))
            .collect::<Result<Vec<_>>>()?;

        // Fixup prototype call.
        if let Target::Type(py_type) = obj
            && let Some(prototype) = member.as_str().and_then(|m| m.strip_prefix("prototype."))
        {
            let (new_member, func_prototype) = prototype.split_once('.').unwrap_or((prototype, ""));
            let new_member = new_member.to_owned();
            assertion(!argvals.is_empty(), &member, "takes one or more arguments")?;
            assertion(
                py_type.instance(&argvals[0]),
                &member,
                &format!("needs binding to type <class '{}'>", py_type.name()),
            )?;
            match func_prototype {
                "call" => obj = Target::Value(argvals.remove(0)),
                "apply" => {
                    assertion(argvals.len() == 2, &member, "takes two arguments")?;
                    let args = argvals.pop().unwrap_or(JsValue::Null);
                    obj = Target::Value(argvals.remove(0));
                    let JsValue::List(args) = args else {
                        return Err(exception(
                            format!("{} second argument needs to be a list", py_str(&member)),
                            Some(expr),
                        ));
                    };
                    argvals = args.borrow().clone();
                }
                _ => {
                    return Err(exception(
                        format!("Unsupported Function method {func_prototype}"),
                        Some(expr),
                    ));
                }
            }
            member = JsValue::str(new_member);
        }

        let obj = match obj {
            Target::Type(PyType::Str) => {
                if member.as_str() == Some("fromCharCode") {
                    assertion(!argvals.is_empty(), &member, "takes one or more arguments")?;
                    return argvals
                        .iter()
                        .map(py_chr)
                        .collect::<Result<String>>()
                        .map(JsValue::str);
                }
                return Err(exception(
                    format!("Unsupported String method {}", py_str(&member)),
                    Some(expr),
                ));
            }
            Target::Type(PyType::Float) => {
                if member.as_str() == Some("pow") {
                    assertion(argvals.len() == 2, &member, "takes two arguments")?;
                    return ops::py_pow(&argvals[0], &argvals[1]);
                }
                return Err(exception(
                    format!("Unsupported Math method {}", py_str(&member)),
                    Some(expr),
                ));
            }
            // Upstream calls `list[member]` (a generic alias) with
            // `allow_recursion=`.
            Target::Type(PyType::List) => {
                return Err(JsError::type_error("list() takes no keyword arguments"));
            }
            Target::Value(value) => value,
        };
        self.builtin_method(obj, member, argvals, expr, allow_recursion)
    }

    /// The `String`/`Array` methods upstream implements, else a call of a
    /// list item or object member.
    fn builtin_method(
        &self,
        obj: JsValue,
        member: JsValue,
        argvals: Vec<JsValue>,
        expr: &str,
        allow_recursion: i32,
    ) -> Result<JsValue> {
        let assertion = |condition: bool, message: &str| {
            if condition {
                Ok(())
            } else {
                Err(exception(
                    format!("{} {message}", py_str(&member)),
                    Some(expr),
                ))
            }
        };
        let no_attribute = |name: &str| {
            JsError::python(
                "AttributeError",
                format!("'{}' object has no attribute '{name}'", type_name(&obj)),
            )
        };
        let list = match &obj {
            JsValue::List(items) => Some(items.clone()),
            _ => None,
        };
        match member.as_str().unwrap_or_default() {
            "split" => {
                assertion(!argvals.is_empty(), "takes one or more arguments")?;
                assertion(argvals.len() == 1, "with limit argument is not implemented")?;
                if !py_truthy(&argvals[0]) {
                    return py_list(&obj);
                }
                let JsValue::Str(text) = &obj else {
                    return Err(no_attribute("split"));
                };
                let JsValue::Str(separator) = &argvals[0] else {
                    return Err(JsError::type_error(format!(
                        "must be str or None, not {}",
                        type_name(&argvals[0])
                    )));
                };
                Ok(JsValue::list(
                    text.split(&**separator).map(JsValue::str).collect(),
                ))
            }
            "join" => {
                assertion(list.is_some(), "must be applied on a list")?;
                assertion(argvals.len() == 1, "takes exactly one argument")?;
                let JsValue::Str(separator) = &argvals[0] else {
                    return Err(JsError::python(
                        "AttributeError",
                        format!(
                            "'{}' object has no attribute 'join'",
                            type_name(&argvals[0])
                        ),
                    ));
                };
                let items = list.unwrap_or_default();
                let items = items.borrow();
                let mut parts = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let JsValue::Str(text) = item else {
                        return Err(JsError::type_error(format!(
                            "sequence item {index}: expected str instance, {} found",
                            type_name(item)
                        )));
                    };
                    parts.push(text.as_ref());
                }
                Ok(JsValue::str(parts.join(separator)))
            }
            "reverse" => {
                assertion(argvals.is_empty(), "does not take any arguments")?;
                let Some(items) = list else {
                    return Err(no_attribute("reverse"));
                };
                items.borrow_mut().reverse();
                Ok(obj)
            }
            "slice" => {
                assertion(
                    matches!(obj, JsValue::List(_) | JsValue::Str(_)),
                    "must be applied on a list or string",
                )?;
                assertion(argvals.len() <= 2, "takes between 0 and 2 arguments")?;
                match &obj {
                    JsValue::Str(text) => {
                        let (start, stop) = py_slice(&argvals, text.chars().count())?;
                        Ok(JsValue::str(char_slice(text, start, Some(stop))))
                    }
                    _ => {
                        let items = list.unwrap_or_default();
                        let items = items.borrow();
                        let (start, stop) = py_slice(&argvals, items.len())?;
                        Ok(JsValue::list(items[start..stop].to_vec()))
                    }
                }
            }
            "splice" => {
                assertion(list.is_some(), "must be applied on a list")?;
                assertion(!argvals.is_empty(), "takes one or more arguments")?;
                let items = list.unwrap_or_default();
                let length = items.borrow().len() as i64;
                let mut index = py_int(&argvals[0])?;
                let how_many = match argvals.get(1) {
                    Some(value) => py_int(value)?,
                    None => length,
                };
                if index < 0 {
                    index += length;
                }
                let mut removed = Vec::new();
                let mut items = items.borrow_mut();
                for _ in index..(index.saturating_add(how_many)).min(length) {
                    let Some(at) = py_index(items.len(), index) else {
                        return Err(JsError::python("IndexError", "pop index out of range"));
                    };
                    removed.push(items.remove(at));
                }
                for (offset, item) in argvals.iter().skip(2).enumerate() {
                    let at = index + offset as i64;
                    let len = items.len() as i64;
                    let at = if at < 0 {
                        (at + len).max(0)
                    } else {
                        at.min(len)
                    };
                    items.insert(at as usize, item.clone());
                }
                Ok(JsValue::list(removed))
            }
            "unshift" => {
                assertion(list.is_some(), "must be applied on a list")?;
                assertion(!argvals.is_empty(), "takes one or more arguments")?;
                let items = list.unwrap_or_default();
                for item in argvals.into_iter().rev() {
                    items.borrow_mut().insert(0, item);
                }
                Ok(obj)
            }
            "pop" => {
                assertion(list.is_some(), "must be applied on a list")?;
                assertion(argvals.is_empty(), "does not take any arguments")?;
                Ok(list
                    .and_then(|items| items.borrow_mut().pop())
                    .unwrap_or(JsValue::Null))
            }
            "push" => {
                assertion(!argvals.is_empty(), "takes one or more arguments")?;
                let Some(items) = list else {
                    return Err(no_attribute("extend"));
                };
                items.borrow_mut().extend(argvals);
                Ok(obj)
            }
            "forEach" => {
                assertion(!argvals.is_empty(), "takes one or more arguments")?;
                assertion(argvals.len() <= 2, "takes at-most 2 arguments")?;
                let function = &argvals[0];
                let this = argvals.get(1).cloned().unwrap_or_else(|| JsValue::str(""));
                let mut results = Vec::new();
                let mut index = 0;
                loop {
                    let item = match &obj {
                        JsValue::List(items) => items.borrow().get(index).cloned(),
                        JsValue::Str(text) => {
                            text.chars().nth(index).map(|c| JsValue::str(c.to_string()))
                        }
                        JsValue::Object(pairs) => {
                            pairs.borrow().get(index).map(|(key, _)| key.clone())
                        }
                        other => {
                            return Err(JsError::type_error(format!(
                                "'{}' object is not iterable",
                                type_name(other)
                            )));
                        }
                    };
                    let Some(item) = item else {
                        break;
                    };
                    let args = vec![item, JsValue::Int(index as i64), obj.clone()];
                    let kwargs = [("this", this.clone())];
                    results.push(self.call_value(function, args, &kwargs, allow_recursion)?);
                    index += 1;
                }
                Ok(JsValue::list(results))
            }
            "indexOf" => {
                assertion(!argvals.is_empty(), "takes one or more arguments")?;
                assertion(argvals.len() <= 2, "takes at-most 2 arguments")?;
                let start = match argvals.get(1) {
                    None => 0,
                    Some(JsValue::Int(i)) => *i,
                    Some(JsValue::Bool(b)) => i64::from(*b),
                    Some(_) => {
                        return Err(JsError::type_error(
                            "slice indices must be integers or have an __index__ method",
                        ));
                    }
                };
                let from = |length: usize| {
                    let length = length as i64;
                    (if start < 0 {
                        (start + length).max(0)
                    } else {
                        start
                    }) as usize
                };
                match &obj {
                    JsValue::List(items) => {
                        let items = items.borrow();
                        let found =
                            items
                                .iter()
                                .enumerate()
                                .skip(from(items.len()))
                                .find(|(_, item)| {
                                    value::py_is(item, &argvals[0]) || py_eq(item, &argvals[0])
                                });
                        Ok(JsValue::Int(found.map_or(-1, |(index, _)| index as i64)))
                    }
                    JsValue::Str(text) => {
                        let JsValue::Str(needle) = &argvals[0] else {
                            return Err(JsError::type_error(format!(
                                "must be str, not {}",
                                type_name(&argvals[0])
                            )));
                        };
                        let chars: Vec<char> = text.chars().collect();
                        let needle: Vec<char> = needle.chars().collect();
                        let start = from(chars.len());
                        let found = (start..=chars.len())
                            .find(|at| chars[*at..].starts_with(&needle))
                            .filter(|_| start <= chars.len());
                        Ok(JsValue::Int(found.map_or(-1, |at| at as i64)))
                    }
                    _ => Err(no_attribute("index")),
                }
            }
            "charCodeAt" => {
                let JsValue::Str(text) = &obj else {
                    return Err(exception(
                        format!("{} must be applied on a string", py_str(&member)),
                        Some(expr),
                    ));
                };
                assertion(argvals.len() == 1, "takes exactly one argument")?;
                let index = match argvals[0] {
                    JsValue::Int(i) => i,
                    JsValue::Bool(b) => i64::from(b),
                    _ => 0,
                };
                let chars: Vec<char> = text.chars().collect();
                if index >= chars.len() as i64 {
                    return Ok(JsValue::Null);
                }
                py_index(chars.len(), index)
                    .map(|at| JsValue::Int(i64::from(u32::from(chars[at]))))
                    .ok_or_else(|| JsError::python("IndexError", "string index out of range"))
            }
            _ => {
                let function = match &obj {
                    JsValue::List(items) => {
                        let index = py_int(&member)?;
                        let items = items.borrow();
                        py_index(items.len(), index)
                            .map(|at| items[at].clone())
                            .ok_or_else(|| {
                                JsError::python("IndexError", "list index out of range")
                            })?
                    }
                    JsValue::Object(pairs) => {
                        check_hashable(&member)?;
                        object_get(&pairs.borrow(), &member)
                            .ok_or_else(|| JsError::python("KeyError", py_repr(&member)))?
                    }
                    JsValue::Str(_) => {
                        return Err(JsError::type_error(
                            "string indices must be integers, not 'str'",
                        ));
                    }
                    other => {
                        return Err(JsError::type_error(format!(
                            "'{}' object is not subscriptable",
                            type_name(other)
                        )));
                    }
                };
                self.call_value(&function, argvals, &[], allow_recursion)
            }
        }
    }

    /// `extract_object(objname, *global_stack)`: an object literal of
    /// functions (`NAME = {key: function(args){code}, ...};`).
    pub fn extract_object(&self, objname: &str, global_stack: &[Frame]) -> Result<JsValue> {
        const FUNC_NAME: &str = r#"(?:[a-zA-Z$0-9]+|"[a-zA-Z$0-9]+"|'[a-zA-Z$0-9]+')"#;
        // `(?<![a-zA-Z$0-9.])` as a consumed `(?:^|[^a-zA-Z$0-9.])`.
        let object = Regex::new(&format!(
            r"(?:^|[^a-zA-Z$0-9.]){}\s*=\s*\{{\s*(?P<fields>(?:{FUNC_NAME}\s*:\s*function\s*\(.*?\)\s*\{{.*?\}}(?:,\s*)?)*)\}}\s*;",
            regex::escape(objname)
        ))
        .expect("object pattern");
        let Some(found) = object.captures(&self.code) else {
            return Err(exception(format!("Could not find object {objname}"), None));
        };
        let fields = Regex::new(&format!(
            r"(?P<key>{FUNC_NAME})\s*:\s*function\s*\((?P<args>(?:[a-zA-Z_$][\w$]*|,)*)\)\{{(?P<code>[^}}]+)\}}"
        ))
        .expect("fields pattern");
        let mut pairs = Vec::new();
        for field in fields.captures_iter(&found["fields"]) {
            let argnames: Vec<String> = field["args"].split(',').map(str::to_owned).collect();
            let name = remove_quotes(&field["key"]);
            let function = self.build_function(argnames, &field["code"], global_stack.to_vec());
            let function = with_repr(function, &format!("F<{name}>"));
            object_set(&mut pairs, JsValue::str(name), function);
        }
        Ok(JsValue::Object(Rc::new(RefCell::new(pairs))))
    }

    /// `extract_function_code(funcname)`: the argument names and body.
    pub fn extract_function_code(&self, funcname: &str) -> Result<(Vec<String>, String)> {
        let name = regex::escape(funcname);
        let function = Regex::new(&format!(
            r"(?s)(?:function\s+{name}|[{{;,]\s*{name}\s*=\s*function|(?:var|const|let)\s+{name}\s*=\s*function)\s*\((?P<args>[^)]*)\)\s*(?P<code>\{{.+\}})"
        ))
        .expect("function pattern");
        let Some(found) = function.captures(&self.code) else {
            return Err(exception(
                format!("Could not find JS function \"{funcname}\""),
                None,
            ));
        };
        let (code, _) = Self::separate_at_paren(&found["code"], None)?;
        let argnames = found["args"]
            .split(',')
            .map(|arg| py_strip(arg).to_owned())
            .collect();
        Ok((argnames, code.to_owned()))
    }

    /// `extract_function(funcname)`.
    pub fn extract_function(&self, funcname: &str) -> Result<JsValue> {
        self.extract_function_with_globals(funcname, Vec::new())
    }

    /// `extract_function(funcname, *global_stack)`.
    pub fn extract_function_with_globals(
        &self,
        funcname: &str,
        global_stack: Vec<Scope>,
    ) -> Result<JsValue> {
        let (argnames, code) = self.extract_function_code(funcname)?;
        let stack = global_stack.into_iter().map(Frame::Dict).collect();
        let function = self.extract_function_from_code(&argnames, &code, stack)?;
        Ok(with_repr(function, &format!("F<{funcname}>")))
    }

    /// `extract_function_from_code(argnames, code, *global_stack)`: nested
    /// `function(...){...}`s become named objects of a new local scope.
    pub fn extract_function_from_code(
        &self,
        argnames: &[String],
        code: &str,
        global_stack: Vec<Frame>,
    ) -> Result<JsValue> {
        static NESTED: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(r"function\((?P<args>[^)]*)\)\s*\{").expect("nested function pattern")
        });
        let local_vars = scope::<String>([]);
        let mut code = code.to_owned();
        while let Some(found) = NESTED.captures(&code) {
            let whole = found.get(0).expect("match");
            let (start, body_start) = (whole.start(), whole.end());
            let args: Vec<String> = found["args"]
                .split(',')
                .map(|arg| py_strip(arg).to_owned())
                .collect();
            let (body, remaining) = Self::separate_at_paren(&code[body_start - 1..], None)?;
            let mut stack = vec![Frame::Dict(local_vars.clone())];
            stack.extend(global_stack.iter().cloned());
            let function = self.extract_function_from_code(&args, body, stack)?;
            let (name, function) = self.named(function);
            local_vars.borrow_mut().insert(name.clone(), function);
            code = format!("{}{name}{remaining}", &code[..start]);
        }
        let mut stack = vec![Frame::Dict(local_vars)];
        stack.extend(global_stack);
        Ok(self.build_function(argnames.to_vec(), &code, stack))
    }

    /// `call_function(funcname, *args)`.
    pub fn call_function(&self, funcname: &str, args: Vec<JsValue>) -> Result<JsValue> {
        let function = self.extract_function(funcname)?;
        self.call(&function, args)
    }

    /// Calls a function value (`resf(args)`).
    pub fn call(&self, function: &JsValue, args: Vec<JsValue>) -> Result<JsValue> {
        self.call_value(function, args, &[], 100)
    }

    /// `build_function(argnames, code, *global_stack)`.
    fn build_function(
        &self,
        argnames: Vec<String>,
        code: &str,
        global_stack: Vec<Frame>,
    ) -> JsValue {
        let global_stack = if global_stack.is_empty() {
            vec![Frame::Dict(scope::<String>([]))]
        } else {
            global_stack
        };
        JsValue::Function(JsFunction {
            repr: "".into(),
            argnames: Rc::new(argnames),
            code: code.into(),
            global_stack: Rc::new(global_stack),
        })
    }

    /// `resf(args, kwargs, allow_recursion)`: the arguments (missing ones
    /// `None`) and keywords are set in the function's first scope, which
    /// persists between calls.
    fn call_value(
        &self,
        function: &JsValue,
        args: Vec<JsValue>,
        kwargs: &[(&str, JsValue)],
        allow_recursion: i32,
    ) -> Result<JsValue> {
        let JsValue::Function(function) = function else {
            return Err(JsError::type_error(format!(
                "'{}' object is not callable",
                type_name(function)
            )));
        };
        let first = &function.global_stack[0];
        let mut args = args.into_iter();
        for name in function.argnames.iter() {
            first.set(name, args.next().unwrap_or(JsValue::Null));
        }
        for (name, value) in kwargs {
            first.set(name, value.clone());
        }
        let var_stack = Namespace::new(function.global_stack.to_vec());
        let code = function.code.replace('\n', " ");
        let (ret, should_abort) = self.statement(&code, &var_stack, allow_recursion - 1, false)?;
        Ok(if should_abort { ret } else { JsValue::Null })
    }
}

/// `function_with_repr(func, repr)`.
fn with_repr(function: JsValue, repr: &str) -> JsValue {
    match function {
        JsValue::Function(function) => JsValue::Function(JsFunction {
            repr: repr.into(),
            ..function
        }),
        other => other,
    }
}

/// Upstream `remove_quotes(s)`.
fn remove_quotes(text: &str) -> &str {
    if text.len() >= 2 {
        for quote in ['"', '\''] {
            if text.starts_with(quote) && text.ends_with(quote) {
                return &text[1..text.len() - 1];
            }
        }
    }
    text
}

/// `_regex_flags(expr)`: the flags at the start of `expr`, and the rest
/// after the first non-flag character (which upstream skips).
fn regex_flags(expr: &str) -> (u32, &str) {
    let mut flags = 0;
    let mut chars = expr.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        match regex_flag(c) {
            Some(flag) => flags |= flag,
            None => return (flags, &expr[index + c.len_utf8()..]),
        }
        if chars.peek().is_none() {
            return (flags, "");
        }
    }
    (flags, expr)
}

/// Python `json.loads(text)`. A lone number is read like Python reads it
/// (`-0` is the int `0`, `1e400` the float `inf`).
fn json_loads(text: &str) -> Option<JsValue> {
    static NUMBER: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r"^[ \t\n\r]*(-?(?:0|[1-9][0-9]*))(\.[0-9]+)?([eE][-+]?[0-9]+)?[ \t\n\r]*$")
            .expect("number pattern")
    });
    if let Some(found) = NUMBER.captures(text) {
        let number = found.get(0).map_or("", |all| all.as_str()).trim();
        return Some(if found.get(2).is_none() && found.get(3).is_none() {
            number.parse().map_or_else(
                |_| JsValue::Float(number.parse().unwrap_or(f64::NAN)),
                JsValue::Int,
            )
        } else {
            JsValue::Float(number.parse().unwrap_or(f64::NAN))
        });
    }
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .map(from_json)
}

/// `str(flags)`: an `int`, or (once `i`, `m`, `s` or `u` made it one) a
/// `re.RegexFlag` with its unknown bits in hex.
fn regex_flags_str(flags: u32) -> String {
    const NAMES: [(u32, &str); 4] = [
        (2, "IGNORECASE"),
        (8, "MULTILINE"),
        (16, "DOTALL"),
        (32, "UNICODE"),
    ];
    if flags & 58 == 0 {
        return flags.to_string();
    }
    let mut parts: Vec<String> = NAMES
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| format!("re.{name}"))
        .collect();
    if flags & !58 != 0 {
        parts.push(format!("{:#x}", flags & !58));
    }
    parts.join("|")
}

/// `json.loads(js_to_json(literal, strict=True))` of a string literal.
fn json_literal(literal: &str) -> Result<JsValue> {
    crate::js_to_json::js_to_json_strict(literal)
        .as_deref()
        .and_then(json_loads)
        .ok_or_else(|| JsError::python("ValueError", format!("Unknown value: {literal}")))
}

/// Python `chr(value)`.
fn py_chr(value: &JsValue) -> Result<char> {
    let code = match value {
        JsValue::Int(i) => *i,
        JsValue::Bool(b) => i64::from(*b),
        other => {
            return Err(JsError::type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                type_name(other)
            )));
        }
    };
    u32::try_from(code)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| JsError::python("ValueError", "chr() arg not in range(0x110000)"))
}

/// Python `list(obj)`.
fn py_list(obj: &JsValue) -> Result<JsValue> {
    Ok(JsValue::list(match obj {
        JsValue::Str(text) => text.chars().map(|c| JsValue::str(c.to_string())).collect(),
        JsValue::List(items) => items.borrow().clone(),
        JsValue::Object(pairs) => pairs.borrow().iter().map(|(key, _)| key.clone()).collect(),
        other => {
            return Err(JsError::type_error(format!(
                "'{}' object is not iterable",
                type_name(other)
            )));
        }
    }))
}

/// Python's tuple unpacking error.
fn unpack_error(expected: usize, got: usize) -> JsError {
    JsError::python(
        "ValueError",
        if got < expected {
            format!("not enough values to unpack (expected {expected}, got {got})")
        } else {
            format!("too many values to unpack (expected {expected})")
        },
    )
}

/// `ret, should_ret = value`.
fn unpack_pair(value: JsValue) -> Result<(JsValue, bool)> {
    let items: Vec<JsValue> = match &value {
        JsValue::List(items) => items.borrow().clone(),
        JsValue::Str(text) => text.chars().map(|c| JsValue::str(c.to_string())).collect(),
        JsValue::Object(pairs) => pairs.borrow().iter().map(|(key, _)| key.clone()).collect(),
        other => {
            return Err(JsError::type_error(format!(
                "cannot unpack non-iterable {} object",
                type_name(other)
            )));
        }
    };
    match <[JsValue; 2]>::try_from(items) {
        Ok([ret, should_return]) => Ok((ret, py_truthy(&should_return))),
        Err(items) => Err(unpack_error(2, items.len())),
    }
}

#[cfg(test)]
mod tests;
