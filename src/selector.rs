//! Bounded native format-selector grammar and typed, side-effect-free predicates.
use crate::{Error, Result, VideoFormat, VideoInfo};
use std::cmp::Ordering;

#[derive(Debug)]
pub(crate) struct Atom {
    pub name: String,
    pub index: Option<u64>,
    filters: Vec<Filter>,
}
impl Atom {
    /// A ranked selector (`best`/`worst` with an optional type and `*`) or an
    /// upstream extension selector, as opposed to a literal format ID.
    pub(crate) fn is_selector(&self) -> bool {
        let name = self.name.as_str();
        let ranked = ["best", "worst"].iter().any(|prefix| {
            name.strip_prefix(prefix).is_some_and(|kind| {
                matches!(kind, "" | "video" | "audio" | "*" | "video*" | "audio*")
            })
        });
        ranked || AUDIO_EXTS.contains(&name) || VIDEO_EXTS.contains(&name)
    }
    pub(crate) fn matches(&self, format: &VideoFormat) -> bool {
        self.filters
            .iter()
            .all(|filter| filter.matches(View::Format(format)))
    }
    pub(crate) fn matches_info(&self, info: &VideoInfo) -> bool {
        self.filters
            .iter()
            .all(|filter| filter.matches(View::Info(info)))
    }
}
fn invalid(message: &str) -> Error {
    Error::FormatSelection(message.into())
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Width,
    Height,
    Filesize,
    Tbr,
    Abr,
    Ext,
    Id,
    Language,
    Note,
    Group,
}
impl Field {
    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "width" => Self::Width,
            "height" => Self::Height,
            "filesize" => Self::Filesize,
            "tbr" => Self::Tbr,
            "abr" => Self::Abr,
            "ext" => Self::Ext,
            "format_id" => Self::Id,
            "language" => Self::Language,
            "format_note" => Self::Note,
            "audio_group" => Self::Group,
            _ => return Err(invalid("unsupported format filter field")),
        })
    }
    fn numeric(self) -> bool {
        matches!(
            self,
            Self::Width | Self::Height | Self::Filesize | Self::Tbr | Self::Abr
        )
    }
}
#[derive(Clone, Copy)]
enum View<'a> {
    Format(&'a VideoFormat),
    Info(&'a VideoInfo),
}
enum Number {
    Integer(u64),
    Float(f64),
}
impl<'a> View<'a> {
    fn numeric(self, field: Field) -> Option<Number> {
        let value = match (self, field) {
            (Self::Format(f), Field::Width) => {
                return f.width.map(|v| Number::Integer(u64::from(v)));
            }
            (Self::Format(f), Field::Height) => {
                return f.height.map(|v| Number::Integer(u64::from(v)));
            }
            (Self::Format(f), Field::Filesize) => return f.filesize.map(Number::Integer),
            (Self::Info(f), Field::Width) => return f.width.map(|v| Number::Integer(u64::from(v))),
            (Self::Info(f), Field::Height) => {
                return f.height.map(|v| Number::Integer(u64::from(v)));
            }
            (Self::Info(f), Field::Filesize) => return f.filesize.map(Number::Integer),
            (Self::Format(f), Field::Abr) => f.abr,
            (Self::Format(f), Field::Tbr) => f.tbr,
            _ => None,
        };
        value
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(Number::Float)
    }
    fn string(self, field: Field) -> Option<&'a str> {
        match (self, field) {
            (Self::Format(f), Field::Ext) => Some(&f.ext),
            (Self::Format(f), Field::Id) => Some(&f.format_id),
            (Self::Format(f), Field::Language) => f.language.as_deref(),
            (Self::Format(f), Field::Note) => f.format_note.as_deref(),
            (Self::Format(f), Field::Group) => f.audio_group.as_deref(),
            (Self::Info(f), Field::Ext) => Some(&f.ext),
            (Self::Info(f), Field::Id) => f.format_id.as_deref(),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Prefix,
    Suffix,
    Contains,
}
#[derive(Debug)]
enum Value {
    Numeric(Rational),
    Text(String),
    // Upstream's numeric grammar precedes its string grammar, including when
    // the actual typed field is text. A present string never equals a number.
    NumericAgainstText,
}
#[derive(Debug)]
struct Filter {
    field: Field,
    op: Op,
    negate: bool,
    unknown: bool,
    value: Value,
}
impl Filter {
    fn matches(&self, view: View<'_>) -> bool {
        let matched = match &self.value {
            Value::Numeric(threshold) => {
                let Some(actual) = view.numeric(self.field) else {
                    return self.unknown;
                };
                let order = match actual {
                    Number::Integer(actual) => threshold.compare_integer(actual),
                    Number::Float(actual) => actual
                        .partial_cmp(&(threshold.numerator as f64 / threshold.denominator as f64))
                        .expect("finite filter operands"),
                };
                match self.op {
                    Op::Lt => order.is_lt(),
                    Op::Le => order.is_le(),
                    Op::Gt => order.is_gt(),
                    Op::Ge => order.is_ge(),
                    Op::Eq => order.is_eq(),
                    _ => unreachable!(),
                }
            }
            Value::NumericAgainstText => {
                if view.string(self.field).is_none() {
                    return self.unknown;
                }
                false
            }
            Value::Text(value) => {
                let Some(actual) = view.string(self.field) else {
                    return self.unknown;
                };
                match self.op {
                    Op::Eq => actual == value,
                    Op::Prefix => actual.starts_with(value),
                    Op::Suffix => actual.ends_with(value),
                    Op::Contains => actual.contains(value),
                    _ => unreachable!(),
                }
            }
        };
        matched != self.negate
    }
}
#[derive(Debug)]
struct Rational {
    numerator: u128,
    denominator: u128,
}
impl Rational {
    // Compare integral metadata without casting to f64 or multiplying it by a
    // large denominator. This remains exact for the entire u64 filesize domain.
    fn compare_integer(&self, actual: u64) -> Ordering {
        u128::from(actual)
            .cmp(&(self.numerator / self.denominator))
            .then_with(|| {
                if self.numerator.is_multiple_of(self.denominator) {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            })
    }
    fn parse(text: &str) -> Result<Self> {
        let end = text
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(text.len());
        let (decimal, suffix) = text.split_at(end);
        let (whole, fraction) = decimal.split_once('.').unwrap_or((decimal, ""));
        if whole.is_empty() && fraction.is_empty()
            || fraction.contains('.')
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|v| v.is_ascii_digit())
        {
            return Err(invalid("invalid numeric format filter value"));
        }
        let fraction = fraction.trim_end_matches('0');
        let denominator = 10u128
            .checked_pow(
                fraction
                    .len()
                    .try_into()
                    .map_err(|_| invalid("numeric filter precision overflow"))?,
            )
            .ok_or_else(|| invalid("numeric filter precision overflow"))?;
        let integer = if whole.is_empty() {
            0
        } else {
            whole
                .parse::<u128>()
                .map_err(|_| invalid("numeric filter overflow"))?
        };
        let fractional = if fraction.is_empty() {
            0
        } else {
            fraction
                .parse::<u128>()
                .map_err(|_| invalid("numeric filter overflow"))?
        };
        let mut numerator = integer
            .checked_mul(denominator)
            .and_then(|v| v.checked_add(fractional))
            .ok_or_else(|| invalid("numeric filter overflow"))?;
        if !suffix.is_empty() {
            let multiplier = unit(suffix)?;
            // Reduce first so long decimal precision does not overflow merely
            // because a power-of-ten unit cancels its denominator.
            let common = gcd(multiplier, denominator);
            numerator = numerator
                .checked_mul(multiplier / common)
                .ok_or_else(|| invalid("numeric filter unit overflow"))?;
            let denominator = denominator / common;
            let whole = numerator / denominator;
            let remainder = numerator % denominator;
            let half = denominator / 2;
            let round_up = remainder > half
                || denominator.is_multiple_of(2) && remainder == half && !whole.is_multiple_of(2);
            let rounded = whole
                .checked_add(u128::from(round_up))
                .ok_or_else(|| invalid("numeric filter rounding overflow"))?;
            return Ok(Self {
                numerator: rounded,
                denominator: 1,
            });
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
fn unit(suffix: &str) -> Result<u128> {
    let bytes = suffix.as_bytes();
    let Some(first) = bytes.first() else {
        return Ok(1);
    };
    let Some(power) = b"kmgtpezy"
        .iter()
        .position(|v| *v == first.to_ascii_lowercase())
    else {
        return Err(invalid("invalid numeric filter unit"));
    };
    let tail = &suffix[1..];
    if !matches!(tail, "" | "b" | "B" | "i" | "iB")
        || tail.starts_with('i') && first.is_ascii_lowercase()
    {
        return Err(invalid("invalid numeric filter unit"));
    }
    // Pinned parse_filesize's historical lowercase-prefix/capital-B spelling
    // (kB, mB, ...) is binary; uppercase KB/MB and lowercase kb/mb are decimal.
    // A bare suffix gets B appended by _build_format_filter.
    let binary = tail.starts_with('i') || first.is_ascii_lowercase() && matches!(tail, "" | "B");
    (if binary { 1024u128 } else { 1000u128 })
        .checked_pow(power as u32 + 1)
        .ok_or_else(|| invalid("numeric filter unit overflow"))
}

fn filter(text: &str) -> Result<Filter> {
    let text = text.trim();
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .unwrap_or(text.len());
    let field = Field::parse(&text[..end])?;
    let mut rest = text[end..].trim_start();
    let mut spaced_negation = false;
    let negate = if let Some(tail) = rest.strip_prefix('!') {
        spaced_negation = tail.len() != tail.trim_start().len();
        if field.numeric() && spaced_negation {
            return Err(invalid(
                "numeric != filter operator cannot contain whitespace",
            ));
        }
        rest = tail.trim_start();
        true
    } else {
        false
    };
    let (op, tail) = [
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("^=", Op::Prefix),
        ("$=", Op::Suffix),
        ("*=", Op::Contains),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("=", Op::Eq),
    ]
    .into_iter()
    .find_map(|(token, op)| rest.strip_prefix(token).map(|tail| (op, tail)))
    .ok_or_else(|| invalid("unsupported format filter operator"))?;
    if field.numeric()
        && (matches!(op, Op::Prefix | Op::Suffix | Op::Contains) || negate && !matches!(op, Op::Eq))
        || !field.numeric() && matches!(op, Op::Lt | Op::Le | Op::Gt | Op::Ge)
    {
        return Err(invalid("operator is unsupported for this format field"));
    }
    rest = tail.trim_start();
    let unknown = if let Some(tail) = rest.strip_prefix('?') {
        rest = tail.trim_start();
        true
    } else {
        false
    };
    let value = if field.numeric() {
        Value::Numeric(Rational::parse(rest.trim())?)
    } else if matches!(op, Op::Eq) && !spaced_negation && numeric_lexical(rest.trim()) {
        Rational::parse(rest.trim())?;
        Value::NumericAgainstText
    } else {
        Value::Text(string_value(rest.trim())?)
    };
    Ok(Filter {
        field,
        op,
        negate,
        unknown,
        value,
    })
}
fn numeric_lexical(text: &str) -> bool {
    let end = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(text.len());
    if end == 0 {
        return false;
    }
    let suffix = &text[end..];
    if suffix.is_empty() {
        return true;
    }
    let mut chars = suffix.chars();
    if !chars.next().is_some_and(|c| "kKmMgGtTpPeEzZyY".contains(c)) {
        return false;
    }
    matches!(chars.as_str(), "" | "B" | "b" | "i" | "iB" | "ib")
}

fn string_value(text: &str) -> Result<String> {
    if let Some(quote) = text.chars().next().filter(|c| matches!(c, '\'' | '"')) {
        let mut result = String::new();
        let mut chars = text[quote.len_utf8()..].char_indices();
        while let Some((index, c)) = chars.next() {
            if c == quote {
                if result.is_empty()
                    || !text[quote.len_utf8() + index + c.len_utf8()..]
                        .trim()
                        .is_empty()
                {
                    return Err(invalid("empty or malformed quoted filter value"));
                }
                return Ok(result);
            }
            if c == '\\' {
                let (_, escaped) = chars
                    .next()
                    .ok_or_else(|| invalid("unterminated filter escape"))?;
                if !matches!(escaped, '\\' | '\'' | '"') {
                    result.push('\\');
                }
                result.push(escaped);
            } else {
                result.push(c);
            }
        }
        return Err(invalid("unterminated quoted filter value"));
    }
    if text.is_empty()
        || !text
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(invalid("invalid unquoted string filter value"));
    }
    Ok(text.into())
}
/// Groups retain their filter context: evaluate the inner expression only after
/// narrowing its available formats with `filter`, rather than filtering output.
#[derive(Debug)]
pub(crate) enum Expression {
    Atom(Atom),
    First(Vec<Expression>),
    Merge(Box<Expression>, Box<Expression>),
    Group {
        inner: Box<Expression>,
        filter: Atom,
    },
}
impl Expression {
    fn streams(&self) -> usize {
        match self {
            Self::Atom(_) => 1,
            Self::First(branches) => branches.iter().map(Self::streams).max().unwrap_or(0),
            Self::Merge(first, second) => first.streams() + second.streams(),
            Self::Group { inner, .. } => inner.streams(),
        }
    }
}

/// Upstream `_format_selection_exts`: extension selectors.
pub(crate) const AUDIO_EXTS: &[&str] = &[
    "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav",
];
pub(crate) const VIDEO_EXTS: &[&str] = &["avi", "flv", "mkv", "mov", "mp4", "webm", "3gp"];

fn canonical_name(name: &str) -> Result<(String, Option<u64>)> {
    // Upstream `(best|worst|b|w)(video|audio|v|a)?(\*)?(\.N)?`.
    fn alias(name: &str) -> Option<String> {
        let (name, starred) = name
            .strip_suffix('*')
            .map_or((name, ""), |name| (name, "*"));
        for prefix in ["best", "worst", "b", "w"] {
            if let Some(tail) = name.strip_prefix(prefix) {
                let kind = match tail {
                    "" => "",
                    "video" | "v" => "video",
                    "audio" | "a" => "audio",
                    _ => continue,
                };
                return Some(format!(
                    "{}{kind}{starred}",
                    if prefix.starts_with('b') {
                        "best"
                    } else {
                        "worst"
                    }
                ));
            }
        }
        None
    }
    if let Some(name) = alias(name) {
        return Ok((name, None));
    }
    if let Some((prefix, index)) = name.rsplit_once('.')
        && let Some(canonical) = alias(prefix)
        && index
            .as_bytes()
            .first()
            .is_some_and(|first| (b'1'..=b'9').contains(first))
        && index.bytes().all(|b| b.is_ascii_digit())
    {
        // Upstream's lazy sequence is bounded by the platform's signed index.
        // Reject oversized indices during the full parse, including fallbacks,
        // rather than silently turning a runtime error into another selection.
        let index = index
            .parse::<u64>()
            .ok()
            .filter(|index| *index <= isize::MAX as u64)
            .ok_or_else(|| invalid("format selector index exceeds the platform index limit"))?;
        return Ok((canonical, Some(index)));
    }
    if name.is_empty()
        || matches!(name, "all" | "mergeall")
        || !name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"_.-".contains(&v))
    {
        return Err(invalid("unsupported or malformed format atom"));
    }
    Ok((name.into(), None))
}

struct Parser<'a> {
    text: &'a str,
    position: usize,
    nodes: usize,
    fallbacks: usize,
}
impl Parser<'_> {
    fn rest(&self) -> &str {
        &self.text[self.position..]
    }
    fn whitespace(&mut self) {
        self.position += self.rest().len() - self.rest().trim_start().len();
    }
    fn node(&mut self, expression: Expression) -> Result<Expression> {
        self.nodes += 1;
        if self.nodes > 128 {
            return Err(invalid("format expression exceeds 128 AST nodes"));
        }
        Ok(expression)
    }
    fn filters(&mut self) -> Result<Vec<Filter>> {
        self.whitespace();
        let mut filters = Vec::new();
        while self.rest().starts_with('[') {
            if filters.len() == 16 {
                return Err(invalid("format node exceeds 16 filters"));
            }
            self.position += 1;
            let start = self.position;
            let mut quote = None;
            let mut escaped = false;
            let mut close = None;
            for (index, c) in self.rest().char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if quote.is_some() && c == '\\' {
                    escaped = true;
                    continue;
                }
                if let Some(current) = quote {
                    if c == current {
                        quote = None;
                    }
                } else if matches!(c, '\'' | '"') {
                    quote = Some(c);
                } else if c == '[' {
                    return Err(invalid("nested format filters are unsupported"));
                } else if c == ']' {
                    close = Some(index);
                    break;
                }
            }
            let end = close.ok_or_else(|| invalid("unterminated format filter"))?;
            filters.push(filter(&self.text[start..start + end])?);
            self.position = start + end + 1;
            self.whitespace();
        }
        Ok(filters)
    }
    fn primary(&mut self, depth: usize) -> Result<Expression> {
        self.whitespace();
        if self.rest().starts_with('(') {
            if depth >= 16 {
                return Err(invalid("format groups exceed nesting depth 16"));
            }
            self.position += 1;
            let inner = self.expression(depth + 1)?;
            self.whitespace();
            if !self.rest().starts_with(')') {
                return Err(invalid("unbalanced format group"));
            }
            self.position += 1;
            let filters = self.filters()?;
            return self.node(Expression::Group {
                inner: Box::new(inner),
                filter: Atom {
                    name: "best".into(),
                    index: None,
                    filters,
                },
            });
        }
        let end = self
            .rest()
            .find(|c: char| c.is_whitespace() || matches!(c, '[' | '+' | '/' | '(' | ')'))
            .unwrap_or(self.rest().len());
        let (name, index) = if end == 0 && self.rest().starts_with('[') {
            ("best".into(), None)
        } else {
            canonical_name(&self.rest()[..end])?
        };
        self.position += end;
        let filters = self.filters()?;
        self.node(Expression::Atom(Atom {
            name,
            index,
            filters,
        }))
    }
    fn merge(&mut self, depth: usize) -> Result<Expression> {
        let mut expression = self.primary(depth)?;
        while self.rest().starts_with('+') {
            self.position += 1;
            let right = self.primary(depth)?;
            expression = self.node(Expression::Merge(Box::new(expression), Box::new(right)))?;
        }
        Ok(expression)
    }
    fn expression(&mut self, depth: usize) -> Result<Expression> {
        let first = self.merge(depth)?;
        let mut branches = vec![first];
        while self.rest().starts_with('/') {
            self.fallbacks += 1;
            if self.fallbacks > 15 {
                return Err(invalid(
                    "format expression exceeds 16 global fallback choices",
                ));
            }
            self.position += 1;
            branches.push(self.merge(depth)?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().expect("one branch"))
        } else {
            self.node(Expression::First(branches))
        }
    }
}
pub(crate) fn parse(expression: &str) -> Result<Expression> {
    if expression.is_empty() || expression.len() > 8192 {
        return Err(invalid("format expression must contain 1..8192 bytes"));
    }
    let mut parser = Parser {
        text: expression,
        position: 0,
        nodes: 0,
        fallbacks: 0,
    };
    let result = parser.expression(0)?;
    parser.whitespace();
    if !parser.rest().is_empty() {
        return Err(invalid("unsupported or malformed format expression"));
    }
    if result.streams() > 2 {
        return Err(invalid(
            "format expression may select at most two streams on any path",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn sample() -> VideoFormat {
        serde_json::from_value(json!({"format_id":"video-x", "url":"https://example.com/video.mp4", "ext":"mp4", "width":1280, "height":720, "filesize":1024, "tbr":2000.0, "abr":128.5, "language":"en-US", "format_note":"Main / [720p]+audio", "audio_group":"group-a"})).unwrap()
    }
    fn atom(predicate: &str) -> Atom {
        let Expression::Atom(atom) = parse(&format!("best[{predicate}]")).unwrap() else {
            panic!("expected atom")
        };
        atom
    }
    #[test]
    fn quoted_delimiters_and_chained_filters_do_not_split_the_expression() {
        let ast = parse(r#"bestvideo[format_note*="/ [720p]+audio"][height>=720]+bestaudio[language='en-US']/18[ext=mp4]"#).unwrap();
        let Expression::First(branches) = ast else {
            panic!("expected fallback")
        };
        assert_eq!(branches.len(), 2);
        let Expression::Merge(first, second) = &branches[0] else {
            panic!("expected merge")
        };
        let Expression::Atom(first) = first.as_ref() else {
            panic!("expected first atom")
        };
        let Expression::Atom(second) = second.as_ref() else {
            panic!("expected second atom")
        };
        assert_eq!(first.name, "bestvideo");
        assert_eq!(second.name, "bestaudio");
        assert!(first.is_selector());
        assert!(first.matches(&sample()));
        let Expression::Atom(last) = &branches[1] else {
            panic!("expected fallback atom")
        };
        assert_eq!(last.name, "18");
        assert!(!last.is_selector());
    }
    #[test]
    fn numeric_operators_and_question_unknown_semantics() {
        for predicate in [
            "filesize<1025",
            "filesize<=1024",
            "filesize>1023",
            "filesize>=1024",
            "filesize=1024",
            "filesize!=1023",
            "width>=1280",
            "height<721",
            "abr=128.5",
            "tbr>1999.9",
        ] {
            assert!(atom(predicate).matches(&sample()), "{predicate}");
        }
        for predicate in [
            "filesize<1024",
            "filesize>1024",
            "filesize!=1024",
            "width=1",
            "abr=128",
        ] {
            assert!(!atom(predicate).matches(&sample()), "{predicate}");
        }
        let mut missing = sample();
        missing.filesize = None;
        for predicate in ["filesize<1025", "filesize!=0"] {
            assert!(!atom(predicate).matches(&missing));
        }
        for predicate in ["filesize < ? 1025", "filesize != ? 0"] {
            assert!(atom(predicate).matches(&missing));
        }
    }
    #[test]
    fn every_string_operator_negation_and_missing_marker_is_typed() {
        for predicate in [
            "ext=mp4",
            "ext!=webm",
            "format_id^=video",
            "format_id!^=audio",
            "language$=US",
            "language!$=GB",
            "format_note*='[720p]+'",
            "format_note!*='[360p]'",
            "audio_group=group-a",
            "language ! ^= ? 'fr'",
        ] {
            assert!(atom(predicate).matches(&sample()), "{predicate}");
        }
        for predicate in [
            "ext=webm",
            "format_id!^=video",
            "language!$=US",
            "format_note!*='[720p]'",
        ] {
            assert!(!atom(predicate).matches(&sample()), "{predicate}");
        }
        let mut missing = sample();
        missing.language = None;
        assert!(!atom("language!=fr").matches(&missing));
        assert!(atom("language!=?fr").matches(&missing));
        assert!(atom("language^=?en").matches(&missing));
    }
    #[test]
    fn unicode_and_upstream_quote_backslash_escape_semantics_are_preserved() {
        let mut format = sample();
        format.language = Some("日本語".into());
        assert!(atom("language=日本語").matches(&format));
        format.format_note = Some(r#"A\B "quote" 'single' \n"#.into());
        assert!(atom(r#"format_note="A\\B \"quote\" \'single\' \n""#).matches(&format));
        assert!(!atom(r#"format_note*="\u0041""#).matches(&format));
        format.format_note = Some("é 日本語 ] [ + /".into());
        assert!(atom(r#"format_note='é 日本語 ] [ + /'"#).matches(&format));
    }
    #[test]
    fn unit_case_quirks_binary_decimal_and_exact_ties_even_match_pinned_policy() {
        for (value, expected) in [
            ("1k", 1024),
            ("1K", 1000),
            ("1kB", 1024),
            ("1KB", 1000),
            ("1kb", 1000),
            ("1Kb", 1000),
            ("1Ki", 1024),
            ("1KiB", 1024),
            ("1M", 1_000_000),
            ("1MiB", 1_048_576),
            ("1.5k", 1536),
            ("0.5kB", 512),
            ("1.2345KB", 1234),
            ("1.2355KB", 1236),
            ("0.0005KB", 0),
            ("0.0015KB", 2),
        ] {
            let mut format = sample();
            format.filesize = Some(expected);
            assert!(
                atom(&format!("filesize={value}")).matches(&format),
                "{value}"
            );
        }
        assert!(!atom("filesize=1024.5").matches(&sample()));
        assert!(atom("filesize<1024.5").matches(&sample()));
    }
    #[test]
    fn all_u64_filesizes_are_compared_exactly_without_float_conversion() {
        let mut format = sample();
        format.filesize = Some(9_007_199_254_740_993);
        assert!(!atom("filesize=9007199254740992").matches(&format));
        assert!(atom("filesize>9007199254740992").matches(&format));
        assert!(atom("filesize=9007199254740993").matches(&format));
        format.filesize = Some(u64::MAX);
        assert!(atom("filesize=18446744073709551615").matches(&format));
        assert!(atom("filesize<18446744073709551615.5").matches(&format));
        assert!(!atom("filesize>18446744073709551615.5").matches(&format));
    }
    #[test]
    fn zero_bitrate_is_known_and_nonfinite_or_negative_metadata_is_unknown() {
        let mut format = sample();
        for bitrate in [0.0, -0.0] {
            format.abr = Some(bitrate);
            assert!(atom("abr=0").matches(&format));
        }
        for bitrate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            format.abr = Some(bitrate);
            assert!(!atom("abr!=0").matches(&format));
            assert!(atom("abr!=?0").matches(&format));
        }
    }
    #[test]
    fn info_without_formats_keeps_absent_id_and_stream_metadata_unknown() {
        let mut info: VideoInfo = serde_json::from_value(json!({"id":"direct", "title":"Direct", "url":"https://example.com/direct.mp4", "webpage_url":"https://example.com/direct.mp4", "ext":"mp4", "extractor":"direct", "extractor_key":"Direct", "direct":true, "filesize":1024, "width":640})).unwrap();
        assert!(atom("filesize>=1024").matches_info(&info));
        assert!(atom("width=640").matches_info(&info));
        assert!(atom("ext=mp4").matches_info(&info));
        for predicate in [
            "format_id!=foo",
            "abr!=0",
            "tbr>0",
            "language!=fr",
            "audio_group!=x",
            "format_note!=x",
        ] {
            assert!(!atom(predicate).matches_info(&info), "{predicate}");
        }
        assert!(atom("format_id!=?foo").matches_info(&info));
        info.format_id = Some("18".into());
        assert!(atom("format_id='18'").matches_info(&info));
    }
    #[test]
    fn malformed_later_fallback_is_rejected_in_full_and_unsupported_grammar_is_not_guessed() {
        for expression in [
            "",
            "best/",
            "/best",
            "best+",
            "best+best+best",
            "best,(worst)",
            "best/all",
            "best[height>1",
            "best[height>1]]",
            "best[width~=1]",
            "best[height>1]/worst[unknown=1]",
            "best[height>1]/worst[height=]",
            "best[format_note='']",
            "best[language=]",
            "best[language='unterminated]",
            "best[height='720']",
            "best[language>en]",
            "best[height*=7]",
            "best[height!>1]",
            "best[height=[[1]]",
            "best[language=en/US]",
            "best[language=foo+bar]",
        ] {
            assert!(parse(expression).is_err(), "{expression}");
        }
    }
    #[test]
    fn invalid_numeric_values_units_and_overflow_are_rejected() {
        for value in [
            "-1",
            "+1",
            ".",
            "1..2",
            "NaN",
            "inf",
            "1e3",
            "1miB",
            "1Kib",
            "1KiBB",
            "1kilobytes",
            "1 000",
            "340282366920938463463374607431768211456",
            "340282366920938463463374607431768211455YB",
            "0.0000000000000000000000000000000000000001",
        ] {
            assert!(
                parse(&format!("best[filesize={value}]")).is_err(),
                "{value}"
            );
        }
        assert!(atom("filesize>.5").matches(&sample()));
        assert!(atom("filesize>1.").matches(&sample()));
    }
    #[test]
    fn upstream_numeric_first_text_values_and_spaced_negation_are_preserved() {
        let mut format = sample();
        format.format_id = "95".into();
        format.ext = "123".into();
        format.language = Some("123".into());
        for predicate in ["format_id=95", "ext=123", "language=123"] {
            assert!(!atom(predicate).matches(&format), "{predicate}");
        }
        for predicate in [
            "format_id!=95",
            "ext!=123",
            "language!=123",
            "format_id='95'",
            "ext='123'",
            "language='123'",
        ] {
            assert!(atom(predicate).matches(&format), "{predicate}");
        }
        assert!(!atom("format_id ! =95").matches(&format));
        assert!(!atom("format_id!= '95'").matches(&format));
        assert!(atom("format_id^=95").matches(&format));
        for predicate in ["width ! =1280", "filesize! =1024"] {
            assert!(parse(&format!("best[{predicate}]")).is_err());
        }
        format.language = None;
        assert!(!atom("language!=123").matches(&format));
        assert!(atom("language!=?123").matches(&format));
    }

    #[test]
    fn parser_resource_limits_are_exact_and_allow_bounded_whitespace() {
        assert!(
            matches!(parse(&["best"; 16].join("/")).unwrap(), Expression::First(branches) if branches.len() == 16)
        );
        assert!(parse(&["best"; 17].join("/")).is_err());
        assert!(parse(&format!("best{}", "[ext=mp4]".repeat(16))).is_ok());
        assert!(parse(&format!("best{}", "[ext=mp4]".repeat(17))).is_err());
        assert!(parse(&"x".repeat(8193)).is_err());
        assert!(parse("  best [ height <= ? 720 ] + bestaudio [ language = 'en' ] / 18  ").is_ok());
    }
    #[test]
    fn grouping_precedence_and_postfix_filters_are_explicit_in_the_ast() {
        let Expression::First(branches) = parse("a+b/c+d").unwrap() else {
            panic!("expected fallback")
        };
        assert!(
            branches
                .iter()
                .all(|branch| matches!(branch, Expression::Merge(_, _)))
        );
        let Expression::Merge(first, second) = parse("(a/b)+c").unwrap() else {
            panic!("expected merge")
        };
        let Expression::Group { inner, filter } = first.as_ref() else {
            panic!("expected group")
        };
        assert!(matches!(inner.as_ref(), Expression::First(_)));
        assert!(filter.matches(&sample()));
        assert!(matches!(second.as_ref(), Expression::Atom(_)));
        let Expression::Group { inner, filter } =
            parse("(best/worst)[height>=720][ext=mp4]").unwrap()
        else {
            panic!("expected filtered group")
        };
        assert!(matches!(inner.as_ref(), Expression::First(_)));
        assert!(filter.matches(&sample()));
        let mut format = sample();
        format.height = Some(360);
        assert!(!filter.matches(&format));
        let Expression::Atom(atom) = parse("[height>=720][ext=mp4]").unwrap() else {
            panic!("expected implicit best")
        };
        assert_eq!(atom.name, "best");
        assert!(atom.matches(&sample()));
    }
    #[test]
    fn aliases_canonicalize_exact_upstream_short_forms_without_guessing_modifiers() {
        for (alias, expected) in [
            ("b", "best"),
            ("bv", "bestvideo"),
            ("ba", "bestaudio"),
            ("bestv", "bestvideo"),
            ("besta", "bestaudio"),
            ("bvideo", "bestvideo"),
            ("baudio", "bestaudio"),
            ("w", "worst"),
            ("wv", "worstvideo"),
            ("wa", "worstaudio"),
            ("worstv", "worstvideo"),
            ("worsta", "worstaudio"),
            ("wvideo", "worstvideo"),
            ("waudio", "worstaudio"),
        ] {
            let Expression::Atom(atom) = parse(alias).unwrap() else {
                panic!("expected alias atom")
            };
            assert_eq!(atom.name, expected);
            assert!(atom.is_selector());
        }
        for (alias, expected) in [
            ("bv*", "bestvideo*"),
            ("wa*.2", "worstaudio*"),
            ("b*", "best*"),
        ] {
            let Expression::Atom(atom) = parse(alias).unwrap() else {
                panic!("expected starred atom")
            };
            assert_eq!(atom.name, expected);
        }
        for expression in ["bv**", "a,b", "all", "mergeall"] {
            assert!(parse(expression).is_err(), "{expression}");
        }
        let Expression::Atom(atom) = parse("file.1").unwrap() else {
            panic!("expected literal ID")
        };
        assert_eq!(atom.name, "file.1");
        assert!(!atom.is_selector());
    }
    #[test]
    fn indexed_aliases_are_positive_one_based_and_invalid_index_spellings_stay_literal() {
        for (name, canonical, index) in [
            ("bv.2", "bestvideo", 2),
            ("ba.3", "bestaudio", 3),
            ("best.1", "best", 1),
            ("b.4", "best", 4),
            ("w.2", "worst", 2),
            ("wv.3", "worstvideo", 3),
            ("worsta.12", "worstaudio", 12),
        ] {
            let Expression::Atom(atom) = parse(name).unwrap() else {
                panic!("expected indexed atom")
            };
            assert_eq!(atom.name, canonical);
            assert_eq!(atom.index, Some(index));
            assert!(atom.is_selector());
        }
        for name in [
            "bv.0",
            "bv.01",
            "bv.x",
            "bv.-1",
            "bestvideo.",
            "file.2",
            "18.2",
            "bestfoo.2",
        ] {
            let Expression::Atom(atom) = parse(name).unwrap() else {
                panic!("expected literal atom")
            };
            assert_eq!(atom.name, name);
            assert_eq!(atom.index, None);
            assert!(!atom.is_selector());
        }
        let Expression::Atom(atom) = parse(&format!("bv.{}", isize::MAX)).unwrap() else {
            panic!("expected bounded platform index")
        };
        assert_eq!(atom.name, "bestvideo");
        assert_eq!(atom.index, Some(isize::MAX as u64));
        for index in [
            (isize::MAX as u64 + 1).to_string(),
            u64::MAX.to_string(),
            "18446744073709551616".into(),
            "9".repeat(8189),
        ] {
            assert!(parse(&format!("bv.{index}")).is_err(), "{index}");
        }
        assert!(parse(&format!("best/bv.{}", isize::MAX as u64 + 1)).is_err());
        let Expression::Group { filter, .. } = parse("(bv.2)[height>=720]").unwrap() else {
            panic!("expected group")
        };
        assert_eq!(filter.index, None);
        let Expression::Atom(atom) = parse("[height>=720]").unwrap() else {
            panic!("expected implicit atom")
        };
        assert_eq!(atom.index, None);
    }

    #[test]
    fn recursive_groups_have_global_bounds_and_no_path_can_request_three_streams() {
        for expression in ["a+(b/c)", "(a+b)/(c+d)", "(a/b)+(c/d)", "((a))"] {
            assert!(parse(expression).is_ok(), "{expression}");
        }
        for expression in [
            "()",
            "( )",
            "(a",
            "a)",
            "(a)(b)",
            "a(b)",
            "(a/)",
            "(/a)",
            "(a)+",
            "a+(b+c)",
            "(a+b)+(c/d)",
            "best/(a+b+c)",
            "best/(worst[unknown=1])",
        ] {
            assert!(parse(expression).is_err(), "{expression}");
        }
        assert!(parse(&format!("{}best{}", "(".repeat(16), ")".repeat(16))).is_ok());
        assert!(parse(&format!("{}best{}", "(".repeat(17), ")".repeat(17))).is_err());
        assert!(parse(&format!("({})/worst", ["best"; 16].join("/"))).is_err());
        let node = |groups| format!("{}best{}", "(".repeat(groups), ")".repeat(groups));
        let mut bounded = vec![node(7); 15];
        bounded.push(node(6));
        assert!(parse(&bounded.join("/")).is_ok()); // 128 expression nodes.
        assert!(parse(&vec![node(7); 16].join("/")).is_err()); // 129 nodes.
        assert!(parse(&format!("(best){}", "[ext=mp4]".repeat(16))).is_ok());
        assert!(parse(&format!("(best){}", "[ext=mp4]".repeat(17))).is_err());
    }

    #[test]
    fn saved_predicate_fixture_matches_actual_pinned_upstream_results() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/selectors/filters.json")).unwrap();
        assert_eq!(
            fixture["upstream_commit"],
            "897341241f2bfb2439fe0367f4c7c9ccd29a3828"
        );
        let formats: Vec<VideoFormat> = serde_json::from_value(fixture["formats"].clone()).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let atom = atom(case["predicate"].as_str().unwrap());
            let actual: Vec<_> = formats
                .iter()
                .filter(|format| atom.matches(format))
                .map(|format| format.format_id.as_str())
                .collect();
            let expected: Vec<_> = case["expected_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect();
            assert_eq!(actual, expected, "{}: {}", case["name"], case["predicate"]);
        }
    }
}
