//! Pure, bounded DASH plans. No network or SIDX discovery. Static MPDs
//! follow exact timing rules; dynamic (live) MPDs reproduce upstream's
//! snapshot lists (see [`dynamic_times`]).
use crate::{ByteRange, Error, Fragment, Result, StreamKind, VideoFormat};
use roxmltree::{Document, Node, ParsingOptions};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use url::Url;

/// Upstream `dynamic_mpd` (`--allow-dynamic-mpd`, the default, or
/// `--ignore-dynamic-mpd`).
static DYNAMIC_MPD: AtomicBool = AtomicBool::new(true);

/// `params['dynamic_mpd']`: whether dynamic MPDs give formats.
pub(crate) fn set_dynamic_mpd(process: bool) {
    DYNAMIC_MPD.store(process, Ordering::Relaxed);
}

/// Segments per representation (a 2 s-segment timeline covers 11 days).
const LIMIT: usize = 500_000;
const NS: u128 = 1_000_000_000;
fn bad(message: impl Into<String>) -> Error {
    Error::UnsupportedManifest(message.into())
}
fn number(text: &str) -> Result<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("invalid DASH unsigned integer"));
    }
    text.parse().map_err(|_| bad("DASH integer overflow"))
}
fn positive(text: &str) -> Result<u64> {
    let value = number(text)?;
    if value == 0 {
        return Err(bad("DASH duration/timescale must be positive"));
    }
    Ok(value)
}
fn add(a: u128, b: u128) -> Result<u128> {
    a.checked_add(b).ok_or_else(|| bad("DASH timing overflow"))
}
fn mul(a: u128, b: u128) -> Result<u128> {
    a.checked_mul(b).ok_or_else(|| bad("DASH timing overflow"))
}
fn ceil(a: u128, b: u128) -> Result<u128> {
    Ok(a / b + u128::from(!a.is_multiple_of(b)))
}

/// Exact restricted ISO 8601 day/hour/minute/second duration, in nanoseconds.
fn duration(text: &str) -> Result<u128> {
    let mut rest = text
        .strip_prefix('P')
        .ok_or_else(|| bad("invalid DASH ISO duration"))?;
    let mut time = false;
    let mut last = 0;
    let mut seen = false;
    let mut total = 0;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('T') {
            if time {
                return Err(bad("invalid DASH duration time marker"));
            }
            time = true;
            rest = tail;
            if rest.is_empty() {
                return Err(bad("empty DASH duration time"));
            }
            continue;
        }
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .ok_or_else(|| bad("missing DASH duration unit"))?;
        let amount = &rest[..end];
        let unit = rest.as_bytes()[end];
        let (rank, scale) = match (time, unit) {
            // Zero years and months contribute nothing exactly (`P0Y0M0DT...`).
            (false, b'Y') if amount.bytes().all(|byte| byte == b'0') => (1, 0),
            (false, b'M') if amount.bytes().all(|byte| byte == b'0') => (2, 0),
            (false, b'D') => (3, 86_400),
            (true, b'H') => (4, 3600),
            (true, b'M') => (5, 60),
            (true, b'S') => (6, 1),
            _ => return Err(bad("unsupported DASH duration unit")),
        };
        if rank <= last {
            return Err(bad("duplicate or unordered DASH duration unit"));
        }
        last = rank;
        let nanos = if let Some((whole, fraction)) = amount.split_once('.') {
            if unit != b'S' || fraction.is_empty() || fraction.len() > 9 {
                return Err(bad("unsupported DASH fractional duration"));
            }
            let fraction = number(fraction)?;
            add(
                mul(u128::from(number(whole)?), NS)?,
                u128::from(fraction)
                    * 10u128.pow(9 - amount.split_once('.').unwrap().1.len() as u32),
            )?
        } else {
            mul(u128::from(number(amount)?), NS)?
        };
        total = add(total, mul(nanos, scale)?)?;
        seen = true;
        rest = &rest[end + 1..];
    }
    if !seen {
        return Err(bad("empty DASH duration"));
    }
    Ok(total)
}

/// Upstream `find(_add_ns(name))`: elements in the MPD's own namespace only.
fn children<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Vec<Node<'a, 'input>> {
    let namespace = node.document().root_element().tag_name().namespace();
    node.children()
        .filter(|n| {
            n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == namespace
        })
        .collect()
}
fn one<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Result<Option<Node<'a, 'input>>> {
    let nodes = children(node, name);
    if nodes.len() > 1 {
        return Err(bad(format!(
            "multiple DASH {name} elements are unsupported"
        )));
    }
    Ok(nodes.into_iter().next())
}
/// ElementTree's `get(name)`: an attribute without a namespace.
fn plain<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attr| attr.namespace().is_none() && attr.name() == name)
        .map(|attr| attr.value())
}
fn uri(base: &Url, value: &str) -> Result<Url> {
    if value.is_empty() || value.contains(['\r', '\n']) || value.len() > 16_384 {
        return Err(bad("empty or oversized DASH URI"));
    }
    let url = base.join(value)?;
    if url.as_str().len() > 16_384 {
        return Err(bad("oversized resolved DASH URI"));
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(bad("DASH resources require HTTP(S) URLs without fragments"));
    }
    Ok(url)
}
fn base_url(node: Node<'_, '_>, inherited: &Url, explicit: bool) -> Result<(Url, bool)> {
    match one(node, "BaseURL")? {
        Some(base) => Ok((uri(inherited, base.text().unwrap_or("").trim())?, true)),
        None => Ok((inherited.clone(), explicit)),
    }
}
/// A directory-like base is no direct resource; a query (OK.ru's
/// `<BaseURL>?expires=...</BaseURL>` on `https://host/`) names one.
fn directory(base: &Url) -> bool {
    base.path().ends_with('/') && base.query().is_none()
}
fn range(text: &str) -> Result<ByteRange> {
    let (start, end) = text
        .split_once('-')
        .ok_or_else(|| bad("invalid DASH byte range"))?;
    let offset = number(start)?;
    let end = number(end)?;
    let length = end
        .checked_sub(offset)
        .and_then(|n| n.checked_add(1))
        .filter(|n| *n > 0)
        .ok_or_else(|| bad("invalid DASH byte range endpoints"))?;
    offset
        .checked_add(length)
        .ok_or_else(|| bad("DASH byte range overflow"))?;
    Ok(ByteRange { offset, length })
}
fn fragment(
    base: &Url,
    source: Option<&str>,
    bytes: Option<&str>,
    duration: Option<f64>,
) -> Result<Fragment> {
    let url = match source {
        Some(source) => uri(base, source)?,
        None if bytes.is_some() && !base.path().ends_with('/') => base.clone(),
        _ => return Err(bad("DASH fragment has no resource URI")),
    };
    // Upstream `location_key`: `url` for absolute HTTP(S) locations.
    let absolute = |source: &str| {
        let lower = source.get(..8).unwrap_or(source).to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    };
    Ok(Fragment {
        url,
        byte_range: bytes.map(range).transpose()?,
        encryption: None,
        path: source.filter(|source| !absolute(source)).map(str::to_owned),
        duration,
        fragment_count: None,
    })
}
/// Upstream's text representation keys: the MPD as `manifest_url` and, when
/// segmented, the base of relative fragment paths.
fn text_extra(manifest: &Url, base: Option<Url>) -> serde_json::Map<String, serde_json::Value> {
    let mut extra = serde_json::Map::new();
    extra.insert(crate::info_order::FORMAT_SHAPE.into(), "dash-text".into());
    extra.insert("manifest_url".into(), manifest.as_str().into());
    if let Some(base) = base {
        extra.insert("fragment_base_url".into(), base.as_str().into());
    }
    extra
}

#[derive(Clone)]
struct Entry {
    t: Option<u64>,
    d: u64,
    r: i64,
}
#[derive(Clone, Default)]
struct Address {
    kind: Option<String>,
    attrs: BTreeMap<String, String>,
    timeline: Option<Vec<Entry>>,
    initialization: Option<(Option<String>, Option<String>)>,
    segments: Option<Vec<(Option<String>, Option<String>)>>,
}
impl Address {
    fn inherit(&self, node: Node<'_, '_>) -> Result<Self> {
        let template = one(node, "SegmentTemplate")?;
        let list = one(node, "SegmentList")?;
        if template.is_some() && list.is_some() {
            return Err(bad("ambiguous DASH addressing"));
        }
        let Some(source) = template.or(list) else {
            return Ok(self.clone());
        };
        let kind = source.tag_name().name();
        if self.kind.as_deref().is_some_and(|parent| parent != kind) {
            return Err(bad("mixed inherited DASH addressing schemes"));
        }
        let mut inherited = self.clone();
        inherited.kind = Some(kind.into());
        // Like upstream, other attributes (`endNumber`, `indexRange`,
        // availability hints, namespaced ones, ...) and child elements are
        // not read.
        for attr in source.attributes() {
            if attr.namespace().is_none()
                && matches!(
                    attr.name(),
                    "timescale"
                        | "duration"
                        | "startNumber"
                        | "presentationTimeOffset"
                        | "media"
                        | "initialization"
                )
            {
                inherited
                    .attrs
                    .insert(attr.name().into(), attr.value().into());
            }
        }
        if let Some(timeline) = one(source, "SegmentTimeline")? {
            let mut entries = Vec::new();
            for entry in children(timeline, "S") {
                let r = plain(entry, "r")
                    .unwrap_or("0")
                    .parse::<i64>()
                    .map_err(|_| bad("invalid DASH repeat"))?;
                if r < -1 {
                    return Err(bad("invalid DASH negative repeat"));
                }
                entries.push(Entry {
                    t: plain(entry, "t").map(number).transpose()?,
                    d: positive(
                        plain(entry, "d")
                            .ok_or_else(|| bad("DASH timeline duration is required"))?,
                    )?,
                    r,
                });
            }
            if entries.is_empty() {
                return Err(bad("empty DASH timeline"));
            }
            inherited.timeline = Some(entries);
        }
        if let Some(init) = one(source, "Initialization")? {
            // Upstream's `attrib['sourceURL']` (a KeyError without one); a
            // byte range of the representation's own resource is also read.
            if plain(init, "sourceURL").is_none() && plain(init, "range").is_none() {
                return Err(bad("DASH Initialization has no sourceURL"));
            }
            inherited.initialization = Some((
                plain(init, "sourceURL").map(str::to_owned),
                plain(init, "range").map(str::to_owned),
            ));
        }
        let urls = children(source, "SegmentURL");
        if kind == "SegmentTemplate" && !urls.is_empty() {
            return Err(bad("SegmentURL in DASH template"));
        }
        if kind == "SegmentList"
            && (inherited.attrs.contains_key("media")
                || inherited.attrs.contains_key("initialization"))
        {
            return Err(bad("template attributes in DASH SegmentList"));
        }
        if !urls.is_empty() {
            let mut segments = Vec::new();
            for url in urls {
                // Upstream's `attrib['media']`; a byte range of the
                // representation's own resource is also read.
                if plain(url, "media").is_none() && plain(url, "mediaRange").is_none() {
                    return Err(bad("DASH SegmentURL has no media"));
                }
                segments.push((
                    plain(url, "media").map(str::to_owned),
                    plain(url, "mediaRange").map(str::to_owned),
                ));
            }
            inherited.segments = Some(segments);
        }
        Ok(inherited)
    }
    fn integer(&self, name: &str, default: u64, nonzero: bool) -> Result<u64> {
        self.attrs
            .get(name)
            .map(|v| if nonzero { positive(v) } else { number(v) })
            .unwrap_or(Ok(default))
    }
}

fn expand(
    template: &str,
    id: Option<&str>,
    bandwidth: Option<u64>,
    number_value: u64,
    time: u64,
) -> Result<String> {
    expand_template(template, id, bandwidth, number_value, time, false)
}

fn expand_template(
    template: &str,
    id: Option<&str>,
    bandwidth: Option<u64>,
    number_value: u64,
    time: u64,
    initialization: bool,
) -> Result<String> {
    let mut rest = template;
    let mut output = String::new();
    let mut uses_number = false;
    let mut uses_time = false;
    while let Some(start) = rest.find('$') {
        output.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        if let Some(tail) = rest.strip_prefix('$') {
            output.push('$');
            rest = tail;
            continue;
        }
        let end = rest
            .find('$')
            .ok_or_else(|| bad("unterminated DASH template token"))?;
        let token = &rest[..end];
        let (name, padding) = token
            .split_once('%')
            .map_or((token, None), |(name, padding)| (name, Some(padding)));
        uses_number |= name == "Number";
        uses_time |= name == "Time";
        if initialization && (uses_number || uses_time) {
            return Err(bad(
                "DASH initialization cannot contain Number or Time tokens",
            ));
        }
        if uses_number && uses_time {
            return Err(bad(
                "DASH media templates cannot combine Number and Time tokens",
            ));
        }
        let value = match name {
            "RepresentationID" if padding.is_none() => id
                .ok_or_else(|| bad("DASH RepresentationID token requires an id"))?
                .to_owned(),
            "Number" | "Time" | "Bandwidth" => {
                let value = match name {
                    "Number" => number_value,
                    "Time" => time,
                    _ => bandwidth.ok_or_else(|| bad("DASH Bandwidth token requires bandwidth"))?,
                };
                let width = match padding {
                    None => 0,
                    Some(padding) => {
                        let width = padding
                            .strip_prefix('0')
                            .and_then(|s| s.strip_suffix('d'))
                            .ok_or_else(|| bad("unsupported DASH numeric formatting"))?;
                        let width = number(width)?;
                        if width == 0 || width > 20 {
                            return Err(bad("DASH numeric format width must be 1..20"));
                        }
                        width as usize
                    }
                };
                format!("{value:0width$}")
            }
            _ => return Err(bad("unsupported DASH template token")),
        };
        output.push_str(&value);
        if output.len() > 16_384 {
            return Err(bad("oversized DASH template URL"));
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    if output.len() > 16_384 {
        return Err(bad("oversized DASH template URL"));
    }
    Ok(output)
}

/// A Period's duration as segment plans use it.
#[derive(Clone, Copy, Default)]
struct Span {
    /// The exact duration in nanoseconds, when known.
    exact: Option<u128>,
    /// Upstream's parse of a dynamic MPD ([`dynamic_times`]).
    dynamic: bool,
    /// Dynamic only: upstream's `period_duration` taken from an earlier
    /// representation's fragment durations (seconds, as Python floats).
    inferred: Option<f64>,
}

/// Segment numbers, start times and durations (timescale units).
fn times(address: &Address, span: Span) -> Result<Vec<(u64, u64, u64)>> {
    if span.dynamic {
        return dynamic_times(address, span);
    }
    let duration_ns = span.exact;
    let scale = address.integer("timescale", 1, true)?;
    let pto = address.integer("presentationTimeOffset", 0, false)?;
    let first_number = address.integer("startNumber", 1, false)?;
    let period_end = duration_ns
        .map(|duration| add(mul(u128::from(pto), NS)?, mul(duration, u128::from(scale))?))
        .transpose()?;
    let mut times = Vec::new();
    let mut current = 0u64;
    if let Some(entries) = &address.timeline {
        for (index, entry) in entries.iter().enumerate() {
            let start = entry.t.unwrap_or(current);
            if start < pto {
                return Err(bad(
                    "DASH timeline requires unsupported pre-period trimming",
                ));
            }
            if start < current {
                return Err(bad("overlapping or backwards DASH timeline"));
            }
            let count = if entry.r >= 0 {
                u128::from(entry.r as u64) + 1
            } else {
                let end = if let Some(next) = entries.get(index + 1) {
                    let next_t = next
                        .t
                        .ok_or_else(|| bad("negative DASH repeat requires explicit next start"))?;
                    let difference = next_t
                        .checked_sub(start)
                        .ok_or_else(|| bad("backwards DASH repeat boundary"))?;
                    if !difference.is_multiple_of(entry.d) {
                        return Err(bad("DASH repeat overlaps next timeline entry"));
                    }
                    mul(u128::from(next_t), NS)?
                } else {
                    period_end.ok_or_else(|| bad("unbounded DASH negative repeat"))?
                };
                let begin = mul(u128::from(start), NS)?;
                if end <= begin {
                    return Err(bad("empty DASH negative repeat"));
                }
                ceil(end - begin, mul(u128::from(entry.d), NS)?)?
            };
            if count > LIMIT as u128 || times.len() + count as usize > LIMIT {
                return Err(bad("DASH fragment limit exceeded"));
            }
            current = start;
            // Upstream lists every S entry, even past a finite Period end.
            for _ in 0..count {
                let number = first_number
                    .checked_add(times.len() as u64)
                    .ok_or_else(|| bad("DASH segment number overflow"))?;
                times.push((number, current, entry.d));
                current = current
                    .checked_add(entry.d)
                    .ok_or_else(|| bad("DASH timeline overflow"))?;
            }
        }
    } else {
        let segment_duration = address.integer("duration", 0, true)?;
        if segment_duration == 0 {
            return Err(bad("DASH template requires duration or timeline"));
        }
        let duration = duration_ns
            .filter(|value| *value > 0)
            .ok_or_else(|| bad("DASH duration template requires a finite period duration"))?;
        let count = ceil(
            mul(duration, u128::from(scale))?,
            mul(NS, u128::from(segment_duration))?,
        )?;
        if count > LIMIT as u128 {
            return Err(bad("DASH fragment limit exceeded"));
        }
        for index in 0..count as u64 {
            let number = first_number
                .checked_add(index)
                .ok_or_else(|| bad("DASH segment number overflow"))?;
            let time = pto
                .checked_add(
                    index
                        .checked_mul(segment_duration)
                        .ok_or_else(|| bad("DASH timeline overflow"))?,
                )
                .ok_or_else(|| bad("DASH timeline overflow"))?;
            times.push((number, time, segment_duration));
        }
    }
    Ok(times)
}

/// Upstream `_parse_mpd_periods` for a dynamic MPD, which it parses like a
/// static one: `presentationTimeOffset` is ignored, a missing (or zero) `t`
/// continues the timeline, any time order is kept, a negative repeat (`r="-1"`,
/// open until the next entry or the Period end) is a single segment, and a
/// `$Number$` duration template has `ceil(period duration / duration)`
/// segments, none without a Period duration (`mediaPresentationDuration`,
/// `Period@duration` or one inferred from an earlier representation's
/// fragments, see [`Span::inferred`]). The list is upstream's snapshot of the
/// manifest, not the live edge.
fn dynamic_times(address: &Address, span: Span) -> Result<Vec<(u64, u64, u64)>> {
    let scale = address.integer("timescale", 1, true)?;
    address.integer("presentationTimeOffset", 0, false)?;
    let first_number = address.integer("startNumber", 1, false)?;
    let number = |index: usize| {
        first_number
            .checked_add(index as u64)
            .ok_or_else(|| bad("DASH segment number overflow"))
    };
    let mut times = Vec::new();
    if let Some(entries) = &address.timeline {
        // `segment_time = s.get('t') or segment_time`.
        let mut current = 0u64;
        for entry in entries {
            current = entry.t.filter(|t| *t != 0).unwrap_or(current);
            // `for _ in range(s.get('r', 0))`: no repeats when negative.
            let count = if entry.r >= 0 { entry.r as u64 + 1 } else { 1 };
            if count > LIMIT as u64 || times.len() + count as usize > LIMIT {
                return Err(bad("DASH fragment limit exceeded"));
            }
            for _ in 0..count {
                times.push((number(times.len())?, current, entry.d));
                current = current
                    .checked_add(entry.d)
                    .ok_or_else(|| bad("DASH timeline overflow"))?;
            }
        }
        return Ok(times);
    }
    let segment_duration = address.integer("duration", 0, true)?;
    if segment_duration == 0 {
        return Err(bad("DASH template requires duration or timeline"));
    }
    let count = match (span.exact, span.inferred) {
        (Some(exact), _) => ceil(
            mul(exact, u128::from(scale))?,
            mul(NS, u128::from(segment_duration))?,
        )?,
        // `math.ceil(float_or_none(period_duration, segment_duration))`.
        (None, Some(seconds)) => {
            let count = (seconds / (segment_duration as f64 / scale as f64)).ceil();
            if !count.is_finite() || count < 0.0 || count > LIMIT as f64 {
                return Err(bad("DASH fragment limit exceeded"));
            }
            count as u128
        }
        (None, None) => 0,
    };
    if count > LIMIT as u128 {
        return Err(bad("DASH fragment limit exceeded"));
    }
    for index in 0..count as usize {
        let time = (index as u64)
            .checked_mul(segment_duration)
            .ok_or_else(|| bad("DASH timeline overflow"))?;
        times.push((number(index)?, time, segment_duration));
    }
    Ok(times)
}

/// A representation's fragments (an initialization fragment first), and
/// upstream's `sum(frag['duration'] for frag in r['fragments'])` over its
/// media fragments when every one has a duration (`None` for none).
fn plans(
    address: &Address,
    base: &Url,
    explicit_base: bool,
    span: Span,
    id: Option<&str>,
    bandwidth: Option<u64>,
) -> Result<(Vec<Fragment>, Option<f64>)> {
    if !explicit_base
        && (address
            .initialization
            .as_ref()
            .is_some_and(|(source, _)| source.is_none())
            || address
                .segments
                .as_ref()
                .is_some_and(|segments| segments.iter().any(|(source, _)| source.is_none())))
    {
        return Err(bad(
            "DASH omitted resource URI requires an explicit resource BaseURL",
        ));
    }
    if address.kind.is_none() {
        if !explicit_base || directory(base) {
            return Err(bad(
                "DASH representation has no explicit direct resource or addressing",
            ));
        }
        return Ok((
            vec![Fragment {
                url: base.clone(),
                byte_range: None,
                encryption: None,
                path: None,
                duration: None,
                fragment_count: None,
            }],
            None,
        ));
    }
    let mut fragments = Vec::new();
    let media_start;
    if address.kind.as_deref() == Some("SegmentTemplate") {
        let media = address
            .attrs
            .get("media")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| bad("DASH template has no media URL"))?;
        // Upstream takes a template without `$Number$` to need a timeline
        // (`KeyError: 's'`).
        if span.dynamic
            && address.timeline.is_none()
            && !(media.contains("$Number$") || media.contains("$Number%"))
        {
            return Err(bad(
                "dynamic DASH templates without a timeline need a Number token",
            ));
        }
        let schedule = times(address, span)?;
        if let Some(initialization) = address.attrs.get("initialization") {
            if address.initialization.is_some() {
                return Err(bad("ambiguous DASH initialization"));
            }
            let init = expand_template(initialization, id, bandwidth, 0, 0, true)?;
            fragments.push(fragment(base, Some(&init), None, None)?);
        } else if let Some((source, range)) = &address.initialization {
            fragments.push(fragment(base, source.as_deref(), range.as_deref(), None)?);
        }
        media_start = fragments.len();
        // Like upstream, initialization is optional; each fragment's duration
        // is `d / timescale` (the constant `duration` without a timeline).
        let scale = address.integer("timescale", 1, true)? as f64;
        for (number, time, length) in schedule {
            let media = expand(media, id, bandwidth, number, time)?;
            fragments.push(fragment(
                base,
                Some(&media),
                None,
                Some(length as f64 / scale),
            )?);
        }
    } else {
        address.integer("timescale", 1, true)?;
        address.integer("startNumber", 1, false)?;
        address.integer("presentationTimeOffset", 0, false)?;
        if let Some(value) = address.attrs.get("duration") {
            positive(value)?;
        }
        // Like upstream, initialization is optional.
        if let Some((source, bytes)) = &address.initialization {
            fragments.push(fragment(base, source.as_deref(), bytes.as_deref(), None)?);
        }
        media_start = fragments.len();
        let mut segments = address
            .segments
            .as_deref()
            .filter(|segments| !segments.is_empty())
            .ok_or_else(|| bad("empty DASH SegmentList"))?;
        let timeline = match &address.timeline {
            // Upstream gives each entry `r + 1` SegmentURLs (none for a
            // negative repeat) and drops the rest.
            Some(entries) if span.dynamic => {
                let mut timeline = Vec::new();
                for entry in entries {
                    let count = usize::try_from(entry.r + 1).unwrap_or(0);
                    if timeline.len() + count > segments.len() {
                        return Err(bad("DASH timeline and SegmentList counts disagree"));
                    }
                    timeline.extend(std::iter::repeat_n((0, 0, entry.d), count));
                }
                segments = &segments[..timeline.len()];
                Some(timeline)
            }
            Some(_) => Some(times(address, span)?),
            None => None,
        };
        if timeline
            .as_ref()
            .is_some_and(|timeline| timeline.len() != segments.len())
        {
            return Err(bad("DASH timeline and SegmentList counts disagree"));
        }
        // Upstream includes every SegmentURL regardless of the Period end.
        if let Some(duration) = address
            .attrs
            .get("duration")
            .filter(|_| address.timeline.is_none())
        {
            positive(duration)?;
        }
        // Upstream: timeline durations, else the constant `duration`.
        let scale = address.integer("timescale", 1, true)? as f64;
        let constant = address
            .attrs
            .get("duration")
            .and_then(|value| positive(value).ok())
            .map(|value| value as f64 / scale);
        for (index, (source, bytes)) in segments.iter().enumerate() {
            let duration = match &timeline {
                Some(timeline) => Some(timeline[index].2 as f64 / scale),
                None => constant,
            };
            fragments.push(fragment(
                base,
                source.as_deref(),
                bytes.as_deref(),
                duration,
            )?);
        }
    }
    if fragments.len() > LIMIT {
        return Err(bad("DASH fragment limit exceeded"));
    }
    // `try_get(..., float)`: a missing duration (`KeyError`/`TypeError`) or
    // no media fragments (`sum` is the integer 0) give `None`.
    let media = &fragments[media_start..];
    let seconds = (!media.is_empty())
        .then(|| {
            media
                .iter()
                .try_fold(0.0, |total, fragment| Some(total + fragment.duration?))
        })
        .flatten();
    Ok((fragments, seconds))
}

#[cfg(test)]
pub(crate) fn parse(text: &str, manifest: &Url) -> Result<Vec<VideoFormat>> {
    Ok(parse_with_subtitles(text, manifest)?.0)
}

/// Subtitles by language, in manifest order.
pub(crate) type ManifestSubtitles = Vec<(String, crate::SubtitleFormat)>;

/// Each Period's duration in nanoseconds, when known: its `duration`, else
/// up to the next Period's start or, for the last, the presentation end.
/// A missing start is 0 for the first Period and the previous Period's end
/// when that is known. Known starts must increase, and the last Period's
/// explicit end must be the presentation's.
fn period_durations(root: Node<'_, '_>, periods: &[Node<'_, '_>]) -> Result<Vec<Option<u128>>> {
    let total = root
        .attribute("mediaPresentationDuration")
        .map(duration)
        .transpose()?;
    let mut bounds: Vec<(Option<u128>, Option<u128>)> = Vec::new();
    let mut last_start = None;
    for period in periods {
        let start = match period.attribute("start").map(duration).transpose()? {
            Some(start) => Some(start),
            None => match bounds.last() {
                None => Some(0),
                Some((Some(start), Some(length))) => Some(add(*start, *length)?),
                Some(_) => None,
            },
        };
        let length = period.attribute("duration").map(duration).transpose()?;
        if let Some(start) = start {
            if last_start.is_some_and(|previous| start <= previous) {
                return Err(bad("DASH Periods must start in increasing order"));
            }
            if total.is_some_and(|total| start > total) {
                return Err(bad("DASH Period starts after presentation end"));
            }
            last_start = Some(start);
        }
        bounds.push((start, length));
    }
    if let (Some(total), Some((Some(start), Some(length)))) = (total, bounds.last())
        && add(*start, *length)? != total
    {
        return Err(bad("DASH Period end disagrees with presentation duration"));
    }
    let mut durations = Vec::new();
    for (index, (start, length)) in bounds.iter().enumerate() {
        let end = bounds.get(index + 1).map_or(total, |(next, _)| *next);
        // Known adjacent bounds are ordered, so the difference cannot wrap.
        let duration = length.or_else(|| Some(end? - (*start)?));
        if duration == Some(0) {
            return Err(bad("empty DASH period"));
        }
        durations.push(duration);
    }
    Ok(durations)
}

/// Upstream `is_drm_protected`: a direct `ContentProtection` child.
fn protected(node: Node<'_, '_>) -> bool {
    node.children()
        .any(|child| child.is_element() && child.tag_name().name() == "ContentProtection")
}

/// Upstream `_parse_mpd_periods` as `_merge_mpd_periods` combines it.
#[derive(Default)]
struct Periods {
    formats: Vec<VideoFormat>,
    /// Upstream `format_key`: a format without its `format_id`, `fragments`
    /// and `manifest_stream_number`, and its index in `formats`.
    keys: std::collections::HashMap<String, usize>,
    subtitles: ManifestSubtitles,
    /// Upstream `stream_numbers[f['url']]`: video, audio and image
    /// representations count in manifest order per format URL (the MPD, or
    /// an unfragmented representation's own resource), across Periods.
    stream_numbers: BTreeMap<String, usize>,
    total_fragments: usize,
}

impl Periods {
    /// Upstream `_merge_mpd_periods`: a format equal to an earlier one but
    /// for `format_id`, `fragments` and `manifest_stream_number` appends its
    /// fragments to that one (a repeated direct resource adds nothing).
    fn merge(&mut self, mut format: VideoFormat) -> Result<()> {
        let fragments = std::mem::take(&mut format.fragments);
        let format_id = std::mem::take(&mut format.format_id);
        let stream_number = format.manifest_stream_number.take();
        let key = serde_json::to_string(&format)
            .map_err(|error| bad(format!("invalid DASH format: {error}")))?;
        if let Some(&index) = self.keys.get(&key) {
            self.formats[index].fragments.extend(fragments);
            return Ok(());
        }
        if self.formats.len() == 256 {
            return Err(bad("DASH representation limit exceeded"));
        }
        format.fragments = fragments;
        format.format_id = format_id;
        format.manifest_stream_number = stream_number;
        self.keys.insert(key, self.formats.len());
        self.formats.push(format);
        Ok(())
    }

    fn period(
        &mut self,
        root: Node<'_, '_>,
        period: Node<'_, '_>,
        manifest: &Url,
        (root_base, root_explicit): (&Url, bool),
        mut span: Span,
    ) -> Result<()> {
        let (period_base, period_explicit) = base_url(period, root_base, root_explicit)?;
        let address = Address::default().inherit(root)?.inherit(period)?;
        // Upstream `if not period_duration: period_duration = try_get(...)`
        // after each fragmented representation (dynamic MPDs only: static
        // ones need a known duration).
        let infer = |span: &mut Span, seconds: Option<f64>| {
            if span.dynamic && span.exact.is_none() && span.inferred.is_none() {
                span.inferred = seconds;
            }
        };
        let mut representations = 0;
        for adaptation in children(period, "AdaptationSet") {
            let (adaptation_base, adaptation_explicit) =
                base_url(adaptation, &period_base, period_explicit)?;
            let address = address.inherit(adaptation)?;
            for representation in children(adaptation, "Representation") {
                if representations == 256 {
                    return Err(bad("DASH representation limit exceeded"));
                }
                let (base, explicit) =
                    base_url(representation, &adaptation_base, adaptation_explicit)?;
                let address = address.inherit(representation)?;
                let has_drm = protected(adaptation) || protected(representation);
                let mut attrs = BTreeMap::new();
                for node in [root, period, adaptation, representation] {
                    for attr in node.attributes() {
                        if attr.namespace().is_none() {
                            attrs.insert(attr.name(), attr.value());
                        }
                    }
                    if let Some(language) =
                        node.attribute(("http://www.w3.org/XML/1998/namespace", "lang"))
                    {
                        attrs.insert("lang", language);
                    }
                }
                let mime = attrs
                    .get("mimeType")
                    .copied()
                    .ok_or_else(|| bad("DASH representation has no MIME type"))?;
                // Upstream: `contentType`, else the MIME type's major type; other
                // types are classified by their `parse_codecs` result.
                let codecs = crate::codecs::parse(attrs.get("codecs").copied());
                let declared = attrs
                    .get("contentType")
                    .copied()
                    .unwrap_or_else(|| mime.split_once('/').map_or(mime, |(kind, _)| kind));
                let content_type = match declared {
                    "video" | "audio" | "text" => declared,
                    _ if matches!(mime, "image/avif" | "image/jpeg") => {
                        let key = if address.kind.is_none() {
                            &base
                        } else {
                            manifest
                        };
                        *self.stream_numbers.entry(key.to_string()).or_default() += 1;
                        // Storyboards are not formats here, but their
                        // fragments can still give a dynamic Period its
                        // duration (an unplannable one gives none).
                        if address.kind.is_some() {
                            let bandwidth = attrs
                                .get("bandwidth")
                                .and_then(|value| positive(value).ok());
                            let seconds = plans(
                                &address,
                                &base,
                                explicit,
                                span,
                                representation.attribute("id"),
                                bandwidth,
                            )
                            .ok()
                            .and_then(|(_, seconds)| seconds);
                            infer(&mut span, seconds);
                        }
                        continue;
                    }
                    _ => match &codecs {
                        Some(codecs) if codecs.vcodec != "none" => "video",
                        Some(codecs) if codecs.acodec != "none" => "audio",
                        Some(codecs) if codecs.scodec.is_some() => "text",
                        _ if matches!(
                            crate::extractor::direct::mimetype2ext_or(mime, None).as_str(),
                            "tt" | "dfxp" | "ttml" | "xml" | "json"
                        ) =>
                        {
                            "text"
                        }
                        _ => {
                            crate::screen::warning(&format!(
                                "Unknown MIME type {mime} in DASH manifest"
                            ));
                            continue;
                        }
                    },
                };
                if content_type == "text" {
                    let language = attrs.get("lang").map_or("und", |lang| *lang).to_owned();
                    let ext = crate::extractor::direct::mimetype2ext_or(mime, None);
                    if address.kind.is_none() {
                        // "Assuming direct URL to unfragmented media."
                        if explicit && !directory(&base) {
                            let mut extra = text_extra(manifest, None);
                            if has_drm {
                                extra.insert("has_drm".into(), true.into());
                            }
                            self.subtitles.push((
                                language,
                                crate::SubtitleFormat {
                                    extra,
                                    fragments: Vec::new(),
                                    name: None,
                                    url: base.clone(),
                                    ext,
                                    protocol: None,
                                },
                            ));
                        }
                    } else {
                        // Segmented text: the manifest URL with its fragments.
                        let bandwidth = attrs
                            .get("bandwidth")
                            .and_then(|value| positive(value).ok());
                        let (fragments, seconds) = plans(
                            &address,
                            &base,
                            explicit,
                            span,
                            representation.attribute("id"),
                            bandwidth,
                        )?;
                        infer(&mut span, seconds);
                        let mut extra = text_extra(
                            manifest,
                            Some(if explicit {
                                base.clone()
                            } else {
                                base.join(".")?
                            }),
                        );
                        if has_drm {
                            extra.insert("has_drm".into(), true.into());
                        }
                        self.subtitles.push((
                            language,
                            crate::SubtitleFormat {
                                extra,
                                fragments,
                                name: None,
                                url: manifest.clone(),
                                ext,
                                protocol: Some("http_dash_segments".into()),
                            },
                        ));
                    }
                    continue;
                }
                // Selection kinds follow upstream's codec fields (`none` marks a
                // missing stream); without codecs the stream counts as complete.
                let kind = match &codecs {
                    Some(codecs) if codecs.vcodec == "none" => StreamKind::AudioOnly,
                    Some(codecs) if codecs.acodec == "none" => StreamKind::VideoOnly,
                    _ => StreamKind::Standalone,
                };
                let bandwidth = attrs
                    .get("bandwidth")
                    .map(|value| positive(value))
                    .transpose()?;
                // Upstream `mimetype2ext(mime_type)`, also naming the `EXT_dash` container.
                let ext = crate::extractor::direct::mimetype2ext_or(mime, None);
                let tbr = bandwidth.map(|value| value as f64 / 1000.0);
                let dimension = |name| {
                    attrs
                        .get(name)
                        .map(|value| {
                            u32::try_from(positive(value)?)
                                .map_err(|_| bad("DASH dimension overflow"))
                        })
                        .transpose()
                };
                let (fragments, seconds) = plans(
                    &address,
                    &base,
                    explicit,
                    span,
                    representation.attribute("id"),
                    bandwidth,
                )?;
                infer(&mut span, seconds);
                // "Assuming direct URL to unfragmented media": upstream's `url`
                // is the resource itself, without a protocol or fragments.
                let (direct, fragments) = if address.kind.is_none() {
                    (
                        fragments.into_iter().next().map(|fragment| fragment.url),
                        Vec::new(),
                    )
                } else {
                    (None, fragments)
                };
                let stream_number = {
                    let counter = self
                        .stream_numbers
                        .entry(direct.as_ref().unwrap_or(manifest).to_string())
                        .or_default();
                    *counter += 1;
                    *counter - 1
                };
                self.total_fragments = self
                    .total_fragments
                    .checked_add(fragments.len())
                    .filter(|count| *count <= LIMIT)
                    .ok_or_else(|| bad("DASH total fragment limit exceeded"))?;
                // Upstream: the representation ID, else the content type.
                let format_id = match representation.attribute("id") {
                    Some(id) => id.to_owned(),
                    None => content_type.to_owned(),
                };
                // Upstream `**parse_codecs(codecs)`: no codec keys without one.
                // Upstream keys: the MPD as `manifest_url`, `has_drm` for a
                // protected representation, the representation base for
                // relative fragment paths, and `_merge_mpd_periods`'s
                // `is_dash_periods`.
                let mut extra = serde_json::Map::new();
                extra.insert(
                    crate::info_order::FORMAT_SHAPE.into(),
                    if direct.is_some() {
                        "dash-direct"
                    } else {
                        "dash"
                    }
                    .into(),
                );
                extra.insert("manifest_url".into(), manifest.as_str().into());
                if has_drm {
                    extra.insert("has_drm".into(), true.into());
                }
                if address.kind.is_some() {
                    // Without a BaseURL, upstream's base is the MPD's directory.
                    let directory;
                    let fragment_base = if explicit {
                        base.as_str()
                    } else {
                        directory = base.join(".")?;
                        directory.as_str()
                    };
                    extra.insert("fragment_base_url".into(), fragment_base.into());
                }
                extra.insert("is_dash_periods".into(), true.into());
                representations += 1;
                self.merge(VideoFormat {
                    byte_index: None,
                    http_headers: None,
                    extra,
                    protocol: direct.is_none().then(|| "http_dash_segments".into()),
                    vcodec: codecs.as_ref().map(|codecs| codecs.vcodec.clone()),
                    acodec: codecs.as_ref().map(|codecs| codecs.acodec.clone()),
                    dynamic_range: codecs
                        .as_ref()
                        .and_then(|codecs| codecs.dynamic_range)
                        .map(str::to_owned),
                    video_ext: None,
                    audio_ext: None,
                    vbr: None,
                    source_preference: None,
                    quality: None,
                    // Upstream `int_or_none(frameRate)` (fractions stay unknown)
                    // and `int_or_none(audioSamplingRate)`.
                    fps: attrs
                        .get("frameRate")
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .map(crate::info::PyNumber::Int),
                    asr: attrs
                        .get("audioSamplingRate")
                        .and_then(|value| value.trim().parse().ok()),
                    audio_channels: None,
                    resolution: None,
                    format: None,
                    aspect_ratio: None,
                    format_id,
                    url: direct.clone().unwrap_or_else(|| manifest.clone()),
                    // Upstream `mimetype2ext(mime_type)`.
                    ext: ext.clone(),
                    stream_kind: kind,
                    audio_group: (kind != StreamKind::Standalone).then(|| "dash-period-0".into()),
                    // Upstream drops the `mul`/`und`/`zxx`/`mis` language codes.
                    language: attrs
                        .get("lang")
                        .filter(|lang| !matches!(**lang, "mul" | "und" | "zxx" | "mis"))
                        .map(|value| (*value).to_owned()),
                    format_note: Some(format!("DASH {content_type}")),
                    audio_default: false,
                    audio_autoselect: false,
                    downloader_options: None,
                    manifest_stream_number: Some(stream_number),
                    format_index: None,
                    container: Some(format!("{ext}_dash")),
                    abr: if kind == StreamKind::AudioOnly {
                        tbr
                    } else {
                        None
                    },
                    tbr,
                    width: dimension("width")?,
                    height: dimension("height")?,
                    filesize: None,
                    fragments,
                })?;
            }
        }
        Ok(())
    }
}

/// Upstream `_parse_mpd_formats_and_subtitles`: each Period's formats and,
/// from text representations, the subtitles by language (`und` without
/// one), merged across Periods like `_merge_mpd_periods`.
pub(crate) fn parse_with_subtitles(
    text: &str,
    manifest: &Url,
) -> Result<(Vec<VideoFormat>, ManifestSubtitles)> {
    let parsed = parse_manifest(text, manifest)?;
    Ok((parsed.formats, parsed.subtitles))
}

/// A parsed MPD.
pub(crate) struct Manifest {
    pub formats: Vec<VideoFormat>,
    pub subtitles: ManifestSubtitles,
    /// `type="dynamic"` (a live presentation).
    pub dynamic: bool,
}

/// [`parse_with_subtitles`], also telling whether the MPD is dynamic. With
/// `--ignore-dynamic-mpd` a dynamic MPD has no formats or subtitles.
pub(crate) fn parse_manifest(text: &str, manifest: &Url) -> Result<Manifest> {
    if !matches!(manifest.scheme(), "http" | "https")
        || manifest.host_str().is_none()
        || manifest.fragment().is_some()
    {
        return Err(bad(
            "DASH manifest requires an HTTP(S) URL without a fragment",
        ));
    }
    if text.contains("<!DOCTYPE") {
        return Err(bad("DASH DTDs are unsupported"));
    }
    let document = Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|error| bad(format!("invalid DASH XML: {error}")))?;
    let root = document.root_element();
    // Upstream reads the namespace with `(?i)^{([^}]+)?}MPD$`; OK.ru serves
    // `urn:mpeg:DASH:schema:MPD:2011`.
    if root.tag_name().name() != "MPD"
        || !root
            .tag_name()
            .namespace()
            .is_none_or(|namespace| namespace.eq_ignore_ascii_case("urn:mpeg:dash:schema:mpd:2011"))
    {
        return Err(bad("expected a DASH MPD root"));
    }
    let dynamic = match root.attribute("type") {
        None | Some("static") => false,
        Some("dynamic") => true,
        Some(_) => return Err(bad("unsupported DASH MPD type")),
    };
    // `if not self.get_param('dynamic_mpd', True)`: no Periods.
    if dynamic && !DYNAMIC_MPD.load(Ordering::Relaxed) {
        return Ok(Manifest {
            formats: Vec::new(),
            subtitles: Vec::new(),
            dynamic,
        });
    }
    let namespace = root.tag_name().namespace();
    for node in root.descendants().filter(Node::is_element) {
        if node.ancestors().take(66).count() > 65 {
            return Err(bad("DASH XML nesting exceeds 64 element levels"));
        }
        // Upstream's `find(_add_ns(...))` lookups never see elements of other
        // namespaces (`cenc:pssh`, vendor extensions, ...) or their contents.
        if node
            .ancestors()
            .any(|ancestor| ancestor.is_element() && ancestor.tag_name().namespace() != namespace)
        {
            continue;
        }
        // Like upstream, only a ContentProtection element's presence is read
        // (`has_drm`); its scheme data is not interpreted.
        if node
            .ancestors()
            .skip(1)
            .any(|ancestor| ancestor.tag_name().name() == "ContentProtection")
        {
            continue;
        }
        // Upstream never reads `SegmentBase` (or its index ranges and
        // `Initialization`): such a representation is its direct `BaseURL`.
        if node
            .ancestors()
            .any(|ancestor| ancestor.tag_name().name() == "SegmentBase")
        {
            continue;
        }
        // Other elements and attributes upstream does not read (refresh
        // locations, properties, sub- and dependent representations,
        // `RepresentationIndex`, xlink and `xml:base`) are ignored like
        // upstream's; misplaced addressing stays an error.
        let parent = node.parent_element().map(|parent| parent.tag_name().name());
        let valid_parent = match node.tag_name().name() {
            "Period" => parent == Some("MPD"),
            "AdaptationSet" => parent == Some("Period"),
            "Representation" => parent == Some("AdaptationSet"),
            "BaseURL" | "SegmentTemplate" | "SegmentList" => matches!(
                parent,
                Some("MPD" | "Period" | "AdaptationSet" | "Representation")
            ),
            "SegmentTimeline" | "Initialization" => {
                matches!(parent, Some("SegmentTemplate" | "SegmentList"))
            }
            "SegmentURL" => parent == Some("SegmentList"),
            // Upstream reads it on AdaptationSets and Representations only.
            "ContentProtection" => matches!(parent, Some("AdaptationSet" | "Representation")),
            "S" => parent == Some("SegmentTimeline"),
            _ => true,
        };
        if !valid_parent {
            return Err(bad("misplaced DASH addressing or representation element"));
        }
    }
    let periods = children(root, "Period");
    if periods.is_empty() {
        return Err(bad("DASH requires a Period"));
    }
    let durations = if dynamic {
        // Upstream `parse_duration(period.get('duration')) or mpd_duration`
        // (a zero duration is unknown).
        let total = root
            .attribute("mediaPresentationDuration")
            .map(duration)
            .transpose()?
            .filter(|total| *total > 0);
        periods
            .iter()
            .map(|period| {
                Ok(period
                    .attribute("duration")
                    .map(duration)
                    .transpose()?
                    .filter(|length| *length > 0)
                    .or(total))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        period_durations(root, &periods)?
    };
    let (root_base, root_explicit) = base_url(root, manifest, false)?;
    let mut parsed = Periods::default();
    for (period, exact) in periods.into_iter().zip(durations) {
        let earlier = parsed.subtitles.len();
        parsed.period(
            root,
            period,
            manifest,
            (&root_base, root_explicit),
            Span {
                exact,
                dynamic,
                inferred: None,
            },
        )?;
        if earlier > 0 && parsed.subtitles.len() > earlier {
            crate::screen::extractor_warning_once(&format!(
                "[{}] Found subtitles in multiple periods in the DASH manifest; \
                 if part of the subtitles are missing, {}",
                crate::screen::extractor(),
                crate::BUG_REPORTS_MESSAGE
            ));
        }
    }
    if parsed.formats.is_empty() {
        return Err(bad("DASH has no supported representations"));
    }
    Ok(Manifest {
        formats: parsed.formats,
        subtitles: parsed.subtitles,
        dynamic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> Url {
        Url::parse("https://example.com/path/manifest.mpd?token=1").unwrap()
    }
    fn wrap(body: &str) -> String {
        format!(
            "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT2.5S\"><Period>{body}</Period></MPD>"
        )
    }
    #[test]
    fn direct_text_representations_become_subtitles() {
        let text = wrap(&format!(
            "{}<AdaptationSet contentType=\"text\" mimeType=\"text/vtt\" lang=\"en\"><Representation id=\"s\" bandwidth=\"1\"><BaseURL>en.vtt</BaseURL></Representation></AdaptationSet>\
             <AdaptationSet mimeType=\"application/ttml+xml\"><Representation id=\"t\" bandwidth=\"1\"><BaseURL>t.ttml</BaseURL></Representation></AdaptationSet>",
            template("")
                .split("<Period>")
                .nth(1)
                .unwrap()
                .split("</Period>")
                .next()
                .unwrap()
        ));
        let (formats, subtitles) = parse_with_subtitles(&text, &manifest()).unwrap();
        assert_eq!(formats.len(), 1);
        let subtitles: Vec<_> = subtitles
            .iter()
            .map(|(language, format)| (language.as_str(), format.url.as_str(), format.ext.as_str()))
            .collect();
        assert_eq!(
            subtitles,
            [
                ("en", "https://example.com/path/en.vtt", "vtt"),
                ("und", "https://example.com/path/t.ttml", "ttml")
            ]
        );
    }
    fn template(extra: &str) -> String {
        wrap(&format!(
            "<AdaptationSet mimeType=\"video/mp4\" codecs=\"avc1.640028\"><SegmentTemplate timescale=\"10\" duration=\"10\" initialization=\"init-$RepresentationID$.mp4\" media=\"part-$Number%05d$-$Bandwidth$-$$.m4s\" {extra}/><Representation id=\"v\" bandwidth=\"2000000\" width=\"1920\" height=\"1080\"/></AdaptationSet>"
        ))
    }
    fn urls(format: &VideoFormat) -> Vec<&str> {
        format.fragments.iter().map(|f| f.url.as_str()).collect()
    }
    #[test]
    fn duration_template_uses_integer_ceil_tokens_offsets_and_representation_metadata() {
        let formats = parse(
            &template("presentationTimeOffset=\"7\" startNumber=\"4\""),
            &manifest(),
        )
        .unwrap();
        let video = &formats[0];
        // Upstream names DASH formats by representation ID.
        assert_eq!(video.format_id, "v");
        assert_eq!(video.url, manifest());
        assert_eq!(video.stream_kind, StreamKind::VideoOnly);
        assert_eq!(video.audio_group.as_deref(), Some("dash-period-0"));
        assert_eq!(video.tbr, Some(2000.0));
        assert_eq!((video.width, video.height), (Some(1920), Some(1080)));
        assert_eq!(
            urls(video),
            [
                "https://example.com/path/init-v.mp4",
                "https://example.com/path/part-00004-2000000-$.m4s",
                "https://example.com/path/part-00005-2000000-$.m4s",
                "https://example.com/path/part-00006-2000000-$.m4s"
            ]
        );
    }
    #[test]
    fn exact_duration_arithmetic_includes_nanoseconds_days_hours_and_minutes() {
        assert_eq!(
            duration("P1DT2H3M4.123456789S").unwrap(),
            93_784_123_456_789
        );
        assert_eq!(duration("PT0.000000001S").unwrap(), 1);
        let text = template("").replace("PT2.5S", "PT0.300000001S").replace(
            "timescale=\"10\" duration=\"10\"",
            "timescale=\"10\" duration=\"1\"",
        );
        assert_eq!(parse(&text, &manifest()).unwrap()[0].fragments.len(), 5);
        for text in [
            "P",
            "PT",
            "P1Y",
            "P1M",
            "PT1.5H",
            "PT1.1234567890S",
            "PT-1S",
            "PT1S1M",
        ] {
            assert!(duration(text).is_err(), "{text}");
        }
    }
    #[test]
    fn unknown_codecs_and_types_follow_upstream_classification() {
        let text = template("");
        // Unknown codecs only warn; the representation stays a format.
        let formats = parse(&text.replace("avc1.640028", "unknown"), &manifest()).unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].stream_kind, StreamKind::Standalone);
        // application/mp4 is classified by its codecs; text tracks are skipped.
        let application = text.replace("video/mp4", "application/mp4");
        assert_eq!(parse(&application, &manifest()).unwrap()[0].format_id, "v");
        assert!(parse(&text.replace("video/mp4", "text/vtt"), &manifest()).is_err());
    }

    #[test]
    fn base_and_template_inheritance_and_language_are_deterministic() {
        let text = "<MPD><BaseURL>https://cdn.example/root/</BaseURL><SegmentTemplate timescale=\"100\" initialization=\"init-$RepresentationID$.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"10\" d=\"20\" r=\"1\"/></SegmentTimeline></SegmentTemplate><Period><BaseURL>period/</BaseURL><AdaptationSet mimeType=\"audio/mp4\" lang=\"en\"><BaseURL>audio/</BaseURL><Representation id=\"a\" bandwidth=\"128000\"><BaseURL>variant/</BaseURL></Representation></AdaptationSet></Period></MPD>";
        let formats = parse(text, &manifest()).unwrap();
        let audio = &formats[0];
        assert_eq!(audio.ext, "m4a");
        assert_eq!(audio.language.as_deref(), Some("en"));
        // Without codecs upstream knows only the total bitrate.
        assert_eq!(audio.tbr, Some(128.0));
        assert_eq!(audio.abr, None);
        assert_eq!(
            urls(audio),
            [
                "https://cdn.example/root/period/audio/variant/init-a.mp4",
                "https://cdn.example/root/period/audio/variant/10.m4s",
                "https://cdn.example/root/period/audio/variant/30.m4s"
            ]
        );
    }
    #[test]
    fn timelines_expand_finite_and_bounded_negative_repeats_without_float_rounding() {
        let body = "<AdaptationSet mimeType=\"video/webm\"><SegmentTemplate timescale=\"10\" initialization=\"init.webm\" media=\"$Time$.webm\"><SegmentTimeline><S t=\"5\" d=\"5\" r=\"-1\"/><S t=\"20\" d=\"10\" r=\"1\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>";
        let formats = parse(&wrap(body).replace("PT2.5S", "PT4S"), &manifest()).unwrap();
        assert_eq!(formats[0].fragments.len(), 6);
        assert!(urls(&formats[0])[4].ends_with("20.webm"));
        let text = wrap(body)
            .replace("<S t=\"20\" d=\"10\" r=\"1\"/>", "")
            .replace("PT2.5S", "PT1.6S");
        assert_eq!(parse(&text, &manifest()).unwrap()[0].fragments.len(), 4);
        let unbounded = text.replace(" mediaPresentationDuration=\"PT1.6S\"", "");
        assert!(parse(&unbounded, &manifest()).is_err());
        assert!(parse(&wrap(body).replace("t=\"20\"", "t=\"21\""), &manifest()).is_err());
    }
    #[test]
    fn negative_repeat_period_end_honors_presentation_time_offset() {
        let text = wrap(
            "<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate timescale=\"10\" presentationTimeOffset=\"100\" initialization=\"init.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"100\" d=\"10\" r=\"-1\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>",
        );
        let formats = parse(&text, &manifest()).unwrap();
        assert_eq!(formats[0].fragments.len(), 4);
        assert!(urls(&formats[0])[3].ends_with("120.m4s"));
        assert!(parse(&text.replace("<S t=\"100\"", "<S t=\"0\""), &manifest()).is_err());
        assert!(parse(&text.replace("<S t=\"100\" ", "<S "), &manifest()).is_err());
    }
    #[test]
    fn inherited_segment_list_ranges_are_inclusive_and_can_use_resource_base() {
        let text = wrap(
            "<AdaptationSet mimeType=\"audio/mp4\"><BaseURL>combined.mp4</BaseURL><SegmentList><Initialization range=\"0-9\"/><SegmentURL mediaRange=\"10-29\"/><SegmentURL mediaRange=\"30-34\"/></SegmentList><Representation id=\"a\"/></AdaptationSet>",
        );
        let formats = parse(&text, &manifest()).unwrap();
        let fragments = &formats[0].fragments;
        assert_eq!(fragments.len(), 3);
        assert!(
            fragments
                .iter()
                .all(|f| f.url.as_str() == "https://example.com/path/combined.mp4")
        );
        assert_eq!(
            fragments[0].byte_range,
            Some(ByteRange {
                offset: 0,
                length: 10
            })
        );
        assert_eq!(
            fragments[1].byte_range,
            Some(ByteRange {
                offset: 10,
                length: 20
            })
        );
        assert_eq!(
            fragments[2].byte_range,
            Some(ByteRange {
                offset: 30,
                length: 5
            })
        );
        assert!(
            parse(
                &text.replace("<BaseURL>combined.mp4</BaseURL>", ""),
                &manifest()
            )
            .is_err()
        );
        for range in ["1-0", "-1", "0-18446744073709551615", "1-2-3"] {
            assert!(parse(&text.replace("0-9", range), &manifest()).is_err());
        }
    }
    #[test]
    fn direct_resources_require_explicit_non_directory_base_and_never_download_manifest() {
        let text = wrap(
            "<AdaptationSet mimeType=\"application/mp4\" contentType=\"video\" codecs=\"avc1.4,mp4a.40.2\"><Representation><BaseURL>movie.mp4</BaseURL></Representation></AdaptationSet>",
        );
        let formats = parse(&text, &manifest()).unwrap();
        assert_eq!(formats[0].stream_kind, StreamKind::Standalone);
        assert!(formats[0].audio_group.is_none());
        // Upstream's direct format is the resource itself, not a fragment plan.
        assert_eq!(
            formats[0].url.as_str(),
            "https://example.com/path/movie.mp4"
        );
        assert_eq!(formats[0].protocol, None);
        assert!(formats[0].fragments.is_empty());
        assert!(!formats[0].extra.contains_key("fragment_base_url"));
        assert!(
            parse(
                &text.replace("<BaseURL>movie.mp4</BaseURL>", ""),
                &manifest()
            )
            .is_err()
        );
        assert!(parse(&text.replace("movie.mp4", "directory/"), &manifest()).is_err());
    }
    #[test]
    fn okru_style_segment_base_representations_are_direct_resources() {
        // OK.ru: an upper-case namespace, query-only BaseURLs on the host root
        // and SegmentBase index ranges, all of which upstream ignores.
        let text = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?><MPD xmlns=\"urn:mpeg:DASH:schema:MPD:2011\" type=\"static\" mediaPresentationDuration=\"PT10S\"><Period duration=\"PT10S\" start=\"PT0S\"><AdaptationSet mimeType=\"video/webm\"><Representation id=\"0\" codecs=\"vp9\" bandwidth=\"1000\"><BaseURL>?type=4&amp;ct=21</BaseURL><SegmentBase indexRange=\"418-2018\" indexRangeExact=\"true\"><Initialization range=\"0-417\"/></SegmentBase></Representation><Representation id=\"1\" codecs=\"vp9\" bandwidth=\"2000\"><BaseURL>?type=0&amp;ct=21</BaseURL><SegmentBase indexRange=\"418-2018\"/></Representation></AdaptationSet><AdaptationSet mimeType=\"audio/webm\" codecs=\"opus\"><Representation id=\"2\" bandwidth=\"300\"><BaseURL>?type=0&amp;ct=22</BaseURL><SegmentBase indexRange=\"464-1299\"/></Representation></AdaptationSet></Period></MPD>";
        let manifest = Url::parse("https://cdn.example.com/?type=4&ct=6").unwrap();
        let formats = parse(text, &manifest).unwrap();
        let urls: Vec<_> = formats.iter().map(|format| format.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.example.com/?type=4&ct=21",
                "https://cdn.example.com/?type=0&ct=21",
                "https://cdn.example.com/?type=0&ct=22",
            ]
        );
        assert!(formats.iter().all(|format| format.protocol.is_none()));
        assert!(formats.iter().all(|format| format.fragments.is_empty()));
        // `stream_numbers[f['url']]` counts per resource.
        assert!(
            formats
                .iter()
                .all(|format| format.manifest_stream_number == Some(0))
        );
        // A bare directory base is still no resource.
        assert!(parse(&text.replace("?type=4&amp;ct=21", "dir/"), &manifest).is_err());
        // Upstream never reads index ranges outside SegmentBase either.
        assert_eq!(
            parse(
                &text.replace("id=\"0\"", "id=\"0\" indexRange=\"0-1\""),
                &manifest
            )
            .unwrap()
            .len(),
            3
        );
    }
    #[test]
    fn saved_ffmpeg_timeline_duration_list_and_range_manifests_have_two_typed_plans() {
        for text in [
            include_str!("../tests/fixtures/dash/media/manifest.mpd"),
            include_str!("../tests/fixtures/dash/media/number.mpd"),
            include_str!("../tests/fixtures/dash/media/list.mpd"),
            include_str!("../tests/fixtures/dash/media/ranges.mpd"),
        ] {
            let formats = parse(text, &manifest()).unwrap();
            assert_eq!(formats.len(), 2);
            assert_eq!(formats[0].stream_kind, StreamKind::VideoOnly);
            assert_eq!(formats[1].stream_kind, StreamKind::AudioOnly);
            assert!(formats.iter().all(|format| !format.fragments.is_empty()));
            assert_eq!(formats[0].format_id, "0");
            assert_eq!(formats[1].format_id, "1");
        }
    }
    #[test]
    fn unsupported_or_ambiguous_manifests_fail_before_producing_any_formats() {
        let text = template("");
        for changed in [
            text.replace("type=\"static\"", "type=\"event\""),
            // The first Period's end is unknown, so its template has no count.
            text.replace("</MPD>", "<Period/></MPD>"),
            text.replace("<Period>", "<Period><ContentProtection/>"),
            text.replace("<Period>", "<Period/>"),
            text.replace(
                "<Representation ",
                "<BaseURL>a/</BaseURL><BaseURL>b/</BaseURL><Representation ",
            ),
            text.replace("<Representation ", "<SegmentList/><Representation "),
            format!("<!DOCTYPE MPD [<!ENTITY x 'bad'>]>{text}"),
        ] {
            assert!(parse(&changed, &manifest()).is_err(), "{changed}");
        }
        assert!(
            parse(
                &text.replace("init-$RepresentationID$.mp4", "file:///init.mp4"),
                &manifest()
            )
            .is_err()
        );
        // Upstream does not read these either: the same plan results.
        let plain = parse(&text, &manifest()).unwrap();
        for changed in [
            text.replace(
                "<Period>",
                "<Location>https://example.com/next.mpd</Location><Period>",
            ),
            text.replace(
                "/></AdaptationSet>",
                "><RepresentationIndex sourceURL=\"index.sidx\"/></Representation></AdaptationSet>",
            ),
            text.replace(
                "<Period>",
                "<Period xmlns:xlink=\"http://www.w3.org/1999/xlink\" xlink:href=\"other.xml\">",
            ),
            text.replace("<Period>", "<Period xml:base=\"other/\">"),
        ] {
            let formats = parse(&changed, &manifest()).unwrap();
            assert_eq!(urls(&formats[0]), urls(&plain[0]), "{changed}");
        }
    }
    #[test]
    fn initialization_sequence_tokens_and_combined_media_tokens_are_rejected_without_misreading_escapes()
     {
        for token in ["$Number$", "$Number%05d$", "$Time$", "$Time%02d$"] {
            assert!(
                parse(
                    &template("")
                        .replace("init-$RepresentationID$.mp4", &format!("init-{token}.mp4")),
                    &manifest()
                )
                .is_err()
            );
        }
        assert_eq!(
            expand_template(
                "init-$$Number$$-$$Time$$-$RepresentationID$-$Bandwidth$.mp4",
                Some("video"),
                Some(1000),
                0,
                0,
                true
            )
            .unwrap(),
            "init-$Number$-$Time$-video-1000.mp4"
        );
        assert!(expand("$Number$-$Time$", None, None, 1, 2).is_err());
        assert_eq!(
            expand("$$Number$$-$Time$", None, None, 1, 2).unwrap(),
            "$Number$-2"
        );
        let text = template("presentationTimeOffset=\"7\"")
            .replace("part-$Number%05d$-$Bandwidth$-$$.m4s", "part-$Time$.m4s");
        let formats = parse(&text, &manifest()).unwrap();
        assert!(urls(&formats[0])[1].ends_with("part-7.m4s"));
        assert!(urls(&formats[0])[3].ends_with("part-27.m4s"));
    }

    #[test]
    fn malformed_templates_timing_and_arithmetic_overflow_fail_closed() {
        for value in [
            "$Unknown$",
            "$Number%021d$",
            "$Number%5d$",
            "$RepresentationID%02d$",
            "$Number",
            "$Number%00d$",
        ] {
            assert!(expand(value, Some("id"), Some(1), 1, 1).is_err(), "{value}");
        }
        let text = template("");
        for (from, to) in [
            ("duration=\"10\"", "duration=\"0\""),
            ("timescale=\"10\"", "timescale=\"0\""),
            ("duration=\"10\"", "duration=\"bad\""),
            ("id=\"v\"", ""),
            ("bandwidth=\"2000000\"", ""),
            (
                "duration=\"10\"",
                "duration=\"1\" startNumber=\"18446744073709551615\"",
            ),
        ] {
            assert!(parse(&text.replace(from, to), &manifest()).is_err(), "{to}");
        }
        let timeline = wrap(
            "<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate initialization=\"i.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"18446744073709551615\" d=\"1\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>",
        );
        assert!(parse(&timeline, &manifest()).is_err());
    }
    #[test]
    fn dependent_representations_properties_and_locations_are_ignored_like_upstream() {
        let text = template("");
        let plain = parse(&text, &manifest()).unwrap();
        for attribute in [
            "dependencyId=\"base\"",
            "dependencyLevel=\"1\"",
            "scalableEnhancement=\"true\"",
            "indexRange=\"0-10\"",
        ] {
            let formats = parse(
                &text.replace("id=\"v\"", &format!("id=\"v\" {attribute}")),
                &manifest(),
            )
            .unwrap();
            assert_eq!(urls(&formats[0]), urls(&plain[0]), "{attribute}");
        }
        for element in [
            "<EssentialProperty schemeIdUri=\"unknown\"/>",
            "<SupplementalProperty schemeIdUri=\"unknown\"/>",
            "<SubRepresentation/>",
            "<Location>other.mpd</Location>",
            "<PatchLocation>other.mpd</PatchLocation>",
        ] {
            let formats = parse(
                &text.replace("<Representation ", &format!("{element}<Representation ")),
                &manifest(),
            )
            .unwrap();
            assert_eq!(urls(&formats[0]), urls(&plain[0]), "{element}");
        }
        // Misplaced addressing stays an error.
        assert!(
            parse(
                &text.replace(
                    "<Representation ",
                    "<SegmentURL media=\"ignored.m4s\"/><Representation "
                ),
                &manifest()
            )
            .is_err()
        );
        let deep = text
            .replace(
                "<Representation ",
                &format!("{}<Representation ", "<Extension>".repeat(70)),
            )
            .replace(
                "</AdaptationSet>",
                &format!("{}</AdaptationSet>", "</Extension>".repeat(70)),
            );
        assert!(parse(&deep, &manifest()).is_err());
    }

    #[test]
    fn entries_past_a_finite_period_end_and_missing_initialization_are_kept_like_upstream() {
        // Upstream lists every S entry and SegmentURL without Period checks.
        let text = wrap("<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate initialization=\"init.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"0\" d=\"1\" r=\"5\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>").replace("PT2.5S", "PT2S");
        assert_eq!(parse(&text, &manifest()).unwrap()[0].fragments.len(), 7);
        let list = wrap("<AdaptationSet mimeType=\"video/mp4\"><SegmentList timescale=\"10\" duration=\"10\" presentationTimeOffset=\"100\"><Initialization sourceURL=\"init.mp4\"/><SegmentURL media=\"one.m4s\"/><SegmentURL media=\"two.m4s\"/><SegmentURL media=\"three.m4s\"/></SegmentList><Representation/></AdaptationSet>").replace("PT2.5S", "PT2S");
        assert_eq!(parse(&list, &manifest()).unwrap()[0].fragments.len(), 4);
        // Initialization is optional for lists and templates.
        let bare = list.replace("<Initialization sourceURL=\"init.mp4\"/>", "");
        let fragments = &parse(&bare, &manifest()).unwrap()[0].fragments;
        assert_eq!(fragments.len(), 3);
        assert!(fragments[0].url.as_str().ends_with("one.m4s"));
        let template = text.replace(" initialization=\"init.mp4\"", "");
        assert_eq!(parse(&template, &manifest()).unwrap()[0].fragments.len(), 6);
    }

    #[test]
    fn foreign_namespaced_address_attributes_never_become_dash_attributes() {
        for (from, to) in [
            ("media=\"", "x:media=\""),
            ("duration=\"10\"", "x:duration=\"10\""),
            ("initialization=\"", "x:initialization=\""),
        ] {
            let text = template("")
                .replace(
                    "<SegmentTemplate ",
                    "<SegmentTemplate xmlns:x=\"urn:foreign\" ",
                )
                .replace(from, to);
            // Like upstream's `get(name)`, a namespaced attribute is unread:
            // a missing `media`/`duration` fails, a missing initialization
            // only drops the initialization fragment.
            match parse(&text, &manifest()) {
                Err(_) => assert_ne!(from, "initialization=\"", "{to}"),
                Ok(formats) => {
                    assert_eq!(from, "initialization=\"", "{to}");
                    assert!(urls(&formats[0]).iter().all(|url| !url.contains("init")));
                }
            }
        }
        for text in [
            "<SegmentList><Initialization xmlns:x=\"urn:foreign\" x:sourceURL=\"init.mp4\"/><SegmentURL media=\"one.m4s\"/></SegmentList>",
            "<SegmentList><Initialization sourceURL=\"init.mp4\"/><SegmentURL xmlns:x=\"urn:foreign\" x:media=\"one.m4s\"/></SegmentList>",
        ] {
            // Upstream's `attrib['sourceURL']`/`attrib['media']` KeyErrors.
            assert!(parse(&wrap(&format!("<AdaptationSet mimeType=\"video/mp4\">{text}<Representation/></AdaptationSet>")), &manifest()).is_err());
        }
        // `int(s.get('t', 0))`: an unread namespaced `t` is the default.
        let timeline = "<SegmentTemplate initialization=\"init.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S xmlns:x=\"urn:foreign\" x:t=\"5\" d=\"1\"/></SegmentTimeline></SegmentTemplate>";
        let formats = parse(
            &wrap(&format!(
                "<AdaptationSet mimeType=\"video/mp4\">{timeline}<Representation/></AdaptationSet>"
            )),
            &manifest(),
        )
        .unwrap();
        assert!(urls(&formats[0])[1].ends_with("/0.m4s"));
    }

    #[test]
    fn explicit_period_duration_requires_a_valid_consistent_presentation_end() {
        let text = template("")
            .replace("<Period>", "<Period start=\"PT0.1S\" duration=\"PT0.3S\">")
            .replace("PT2.5S", "PT0.4S");
        assert!(parse(&text, &manifest()).is_ok());
        for total in ["invalid", "PT0.3S", "PT0.5S", "PT0.05S"] {
            assert!(
                parse(
                    &text.replace(
                        "mediaPresentationDuration=\"PT0.4S\"",
                        &format!("mediaPresentationDuration=\"{total}\"")
                    ),
                    &manifest()
                )
                .is_err(),
                "{total}"
            );
        }
        assert!(
            parse(
                &text.replace("start=\"PT0.1S\"", "start=\"PT1S\""),
                &manifest()
            )
            .is_err()
        );
        assert!(
            parse(
                &text.replace(" mediaPresentationDuration=\"PT0.4S\"", ""),
                &manifest()
            )
            .is_ok()
        );
    }

    #[test]
    fn aggregate_fragment_and_representation_limits_are_enforced() {
        let rep = "<Representation mimeType=\"video/mp4\"><BaseURL>resource.mp4</BaseURL></Representation>";
        assert!(
            parse(
                &wrap(&format!(
                    "<AdaptationSet>{}</AdaptationSet>",
                    rep.repeat(256)
                )),
                &manifest()
            )
            .is_ok()
        );
        assert!(
            parse(
                &wrap(&format!(
                    "<AdaptationSet>{}</AdaptationSet>",
                    rep.repeat(257)
                )),
                &manifest()
            )
            .is_err()
        );
        let text = template("").replace("PT2.5S", "PT1000000S");
        assert!(parse(&text, &manifest()).is_err());
        let text = text.replace("PT1000000S", "PT500000S").replace(
            "</AdaptationSet>",
            "<Representation id=\"other\" bandwidth=\"2000000\"/></AdaptationSet>",
        );
        assert!(parse(&text, &manifest()).is_err());
    }
    fn periods(body: &str) -> String {
        format!(
            "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT6S\">{body}</MPD>"
        )
    }
    fn paths(format: &VideoFormat) -> Vec<&str> {
        format
            .fragments
            .iter()
            .map(|f| f.url.path().trim_start_matches("/path/"))
            .collect()
    }
    #[test]
    fn periods_merge_equal_formats_and_keep_counting_streams_like_upstream() {
        let video = |period: &str, id: &str| {
            format!(
                "<AdaptationSet mimeType=\"video/mp4\" codecs=\"avc1.42001e\" width=\"640\" height=\"360\"><SegmentTemplate timescale=\"1\" duration=\"2\" initialization=\"{period}/$RepresentationID$/init.mp4\" media=\"{period}/$RepresentationID$/$Number$.m4s\"/><Representation id=\"{id}\" bandwidth=\"400000\"/></AdaptationSet>"
            )
        };
        let audio = |lang: &str, id: &str, file: &str| {
            format!(
                "<AdaptationSet mimeType=\"audio/mp4\" codecs=\"mp4a.40.2\" lang=\"{lang}\"><Representation id=\"{id}\" bandwidth=\"128000\"><BaseURL>{file}</BaseURL></Representation></AdaptationSet>"
            )
        };
        let subtitle = |file: &str| {
            format!(
                "<AdaptationSet contentType=\"text\" mimeType=\"text/vtt\" lang=\"en\"><Representation bandwidth=\"1\"><BaseURL>{file}</BaseURL></Representation></AdaptationSet>"
            )
        };
        let text = periods(&format!(
            "<Period duration=\"PT4S\">{}{}{}</Period><Period duration=\"PT2S\">{}{}{}{}</Period>",
            video("p0", "v0"),
            audio("en", "a0", "a.m4a"),
            subtitle("p0.vtt"),
            video("p1", "v1"),
            audio("en", "a1", "a.m4a"),
            audio("de", "a1de", "de.m4a"),
            subtitle("p1.vtt"),
        ));
        let (formats, subtitles) = parse_with_subtitles(&text, &manifest()).unwrap();
        // Pinned upstream: the first Period's IDs and stream numbers; the
        // second Period's fragments, initialization included, are appended,
        // the repeated direct resource adds nothing and stream numbers count on.
        let summary: Vec<_> = formats
            .iter()
            .map(|f| (f.format_id.as_str(), f.manifest_stream_number, paths(f)))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "v0",
                    Some(0),
                    vec![
                        "p0/v0/init.mp4",
                        "p0/v0/1.m4s",
                        "p0/v0/2.m4s",
                        "p1/v1/init.mp4",
                        "p1/v1/1.m4s"
                    ]
                ),
                ("a0", Some(0), vec![]),
                ("a1de", Some(0), vec![]),
            ]
        );
        assert!(formats.iter().all(|f| f.extra["is_dash_periods"] == true));
        let subtitles: Vec<_> = subtitles
            .iter()
            .map(|(language, format)| (language.as_str(), format.url.path()))
            .collect();
        assert_eq!(subtitles, [("en", "/path/p0.vtt"), ("en", "/path/p1.vtt")]);
        // Formats differing in a merged field (here the fragment base) stay
        // separate, and within one Period equal formats merge too.
        let based = text.replace(
            "<Period duration=\"PT2S\">",
            "<Period duration=\"PT2S\"><BaseURL>other/</BaseURL>",
        );
        let ids: Vec<_> = parse(&based, &manifest())
            .unwrap()
            .into_iter()
            .map(|f| (f.format_id, f.manifest_stream_number))
            .collect();
        let ids: Vec<_> = ids.iter().map(|(id, n)| (id.as_str(), *n)).collect();
        assert_eq!(
            ids,
            [
                ("v0", Some(0)),
                ("a0", Some(0)),
                ("v1", Some(1)),
                ("a1", Some(0)),
                ("a1de", Some(0))
            ]
        );
        let duplicates = wrap(
            "<AdaptationSet mimeType=\"video/mp4\" codecs=\"avc1.42001e\"><SegmentTemplate timescale=\"1\" duration=\"1\" media=\"$RepresentationID$/$Number$.m4s\"/><Representation id=\"x\" bandwidth=\"1000\"/><Representation id=\"y\" bandwidth=\"1000\"/><Representation id=\"z\" bandwidth=\"2000\"/></AdaptationSet>",
        );
        let formats = parse(&duplicates, &manifest()).unwrap();
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].fragments.len(), 6);
        assert_eq!(formats[1].manifest_stream_number, Some(2));
    }
    #[test]
    fn period_spans_come_from_durations_starts_and_the_presentation_end() {
        let numbered = |attributes: &str, prefix: &str| {
            format!(
                "<Period {attributes}><AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate timescale=\"1\" duration=\"2\" media=\"{prefix}-$Number$.m4s\"/><Representation id=\"v\" bandwidth=\"1\"/></AdaptationSet></Period>"
            )
        };
        let fragments = |text: &str| -> Vec<String> {
            parse(text, &manifest()).unwrap()[0]
                .fragments
                .iter()
                .map(|f| f.url.path().trim_start_matches("/path/").to_owned())
                .collect()
        };
        // Pinned upstream gives every Period without a duration the whole
        // presentation (three segments each); this port uses each Period's span.
        let text = periods(&format!(
            "{}{}",
            numbered("start=\"PT0S\"", "p0"),
            numbered("start=\"PT2S\"", "p1")
        ));
        assert_eq!(fragments(&text), ["p0-1.m4s", "p1-1.m4s", "p1-2.m4s"]);
        // A missing start follows the previous Period's duration.
        let text = periods(&format!(
            "{}{}",
            numbered("duration=\"PT4S\"", "p0"),
            numbered("", "p1")
        ));
        assert_eq!(fragments(&text), ["p0-1.m4s", "p0-2.m4s", "p1-1.m4s"]);
        for invalid in [
            // Out of order, after the presentation, a late explicit end, an
            // empty Period, and unknown spans for a duration template.
            format!(
                "{}{}",
                numbered("start=\"PT2S\"", "a"),
                numbered("start=\"PT2S\"", "b")
            ),
            format!("{}{}", numbered("", "a"), numbered("start=\"PT7S\"", "b")),
            format!(
                "{}{}",
                numbered("", "a"),
                numbered("start=\"PT2S\" duration=\"PT5S\"", "b")
            ),
            format!(
                "{}{}",
                numbered("duration=\"PT0S\"", "a"),
                numbered("", "b")
            ),
            format!("{}{}", numbered("", "a"), numbered("", "b")),
        ] {
            assert!(parse(&periods(&invalid), &manifest()).is_err(), "{invalid}");
        }
        // Spans only matter where a count needs them.
        let listed = "<AdaptationSet mimeType=\"video/mp4\"><SegmentList><SegmentURL media=\"one.m4s\"/></SegmentList><Representation id=\"v\" bandwidth=\"1\"/></AdaptationSet>";
        let text = periods(&format!(
            "<Period>{listed}</Period><Period>{listed}</Period>"
        ));
        assert_eq!(fragments(&text), ["one.m4s", "one.m4s"]);
    }
    #[test]
    fn content_protection_marks_formats_has_drm_like_upstream() {
        let text = periods(
            "<Period xmlns:cenc=\"urn:mpeg:cenc:2013\"><AdaptationSet mimeType=\"video/mp4\" codecs=\"avc1.42001e\"><ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\" value=\"cenc\" cenc:default_KID=\"10000000-1000-1000-1000-100000000001\"/><ContentProtection schemeIdUri=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"><cenc:pssh>AAAA</cenc:pssh></ContentProtection><Representation id=\"v\" bandwidth=\"1\"><BaseURL>v.mp4</BaseURL></Representation></AdaptationSet>\
             <AdaptationSet mimeType=\"audio/mp4\" codecs=\"mp4a.40.2\"><Representation id=\"a\" bandwidth=\"1\"><ContentProtection schemeIdUri=\"urn:x\"/><BaseURL>a.m4a</BaseURL></Representation><Representation id=\"clear\" bandwidth=\"1\"><BaseURL>c.m4a</BaseURL></Representation></AdaptationSet>\
             <AdaptationSet contentType=\"text\" mimeType=\"text/vtt\"><ContentProtection schemeIdUri=\"urn:x\"/><Representation bandwidth=\"1\"><BaseURL>s.vtt</BaseURL></Representation></AdaptationSet></Period>",
        );
        let (formats, subtitles) = parse_with_subtitles(&text, &manifest()).unwrap();
        let drm: Vec<_> = formats
            .iter()
            .map(|f| (f.format_id.as_str(), f.extra.get("has_drm").cloned()))
            .collect();
        assert_eq!(
            drm,
            [
                ("v", Some(true.into())),
                ("a", Some(true.into())),
                ("clear", None)
            ]
        );
        assert_eq!(subtitles[0].1.extra["has_drm"], true);
        // Like upstream's namespaced lookups, foreign elements are ignored
        // outside ContentProtection too.
        assert_eq!(
            parse(
                &text.replace(
                    "<cenc:pssh>AAAA</cenc:pssh></ContentProtection>",
                    "</ContentProtection><cenc:pssh>AAAA</cenc:pssh>"
                ),
                &manifest()
            )
            .unwrap()
            .len(),
            formats.len()
        );
    }
    #[test]
    fn representation_limits_count_each_period_and_the_merged_formats() {
        let distinct = |count: usize, offset: usize| {
            (0..count)
                .map(|index| {
                    format!(
                        "<Representation id=\"r{index}\" bandwidth=\"{}\"><BaseURL>r.mp4</BaseURL></Representation>",
                        index + offset + 1
                    )
                })
                .collect::<String>()
        };
        let text = |first: String, second: String| {
            periods(&format!(
                "<Period><AdaptationSet mimeType=\"video/mp4\">{first}</AdaptationSet></Period><Period><AdaptationSet mimeType=\"video/mp4\">{second}</AdaptationSet></Period>"
            ))
        };
        // Equal Periods merge into 200 formats; 400 distinct ones exceed 256.
        assert_eq!(
            parse(&text(distinct(200, 0), distinct(200, 0)), &manifest())
                .unwrap()
                .len(),
            200
        );
        assert!(parse(&text(distinct(200, 0), distinct(200, 200)), &manifest()).is_err());
        assert!(parse(&text(distinct(257, 0), String::new()), &manifest()).is_err());
    }

    fn dynamic(root: &str, body: &str) -> String {
        format!(
            "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"dynamic\" availabilityStartTime=\"2024-01-01T00:00:00Z\" {root}>{body}</MPD>"
        )
    }
    fn video(template: &str) -> String {
        format!(
            "<AdaptationSet mimeType=\"video/mp4\" codecs=\"avc1.640028\">{template}<Representation id=\"v\" bandwidth=\"1000\"/></AdaptationSet>"
        )
    }
    fn audio(template: &str) -> String {
        format!(
            "<AdaptationSet mimeType=\"audio/mp4\" codecs=\"mp4a.40.2\">{template}<Representation id=\"a\" bandwidth=\"1000\"/></AdaptationSet>"
        )
    }
    const NUMBER: &str = "<SegmentTemplate timescale=\"10\" duration=\"20\" initialization=\"init-$RepresentationID$.mp4\" media=\"$RepresentationID$-$Number$.m4s\"/>";
    #[test]
    fn dynamic_number_templates_count_only_a_known_period_duration() {
        let parsed = parse_manifest(
            &dynamic("", &format!("<Period>{}</Period>", video(NUMBER))),
            &manifest(),
        )
        .unwrap();
        assert!(parsed.dynamic);
        // Upstream: `ceil(float_or_none(None, d, default=0))` segments.
        assert_eq!(paths(&parsed.formats[0]), ["init-v.mp4"]);
        // `parse_duration(Period@duration) or mediaPresentationDuration`.
        for (root, period) in [
            ("mediaPresentationDuration=\"PT5S\"", ""),
            ("", "duration=\"PT5S\""),
            ("mediaPresentationDuration=\"PT5S\"", "duration=\"PT0S\""),
        ] {
            let formats = parse(
                &dynamic(
                    root,
                    &format!("<Period {period}>{}</Period>", video(NUMBER)),
                ),
                &manifest(),
            )
            .unwrap();
            assert_eq!(
                paths(&formats[0]),
                ["init-v.mp4", "v-1.m4s", "v-2.m4s", "v-3.m4s"],
                "{root} {period}"
            );
        }
        // Static MPDs keep their exact Period bounds.
        assert!(parse(&dynamic("", "").replace("dynamic", "static"), &manifest()).is_err());
    }
    #[test]
    fn dynamic_timelines_ignore_offsets_and_keep_one_segment_per_negative_repeat() {
        let timeline = "<SegmentTemplate timescale=\"10\" presentationTimeOffset=\"100\" startNumber=\"5\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"40\" d=\"10\" r=\"1\"/><S d=\"20\" r=\"-1\"/><S t=\"100\" d=\"10\"/><S t=\"0\" d=\"5\" r=\"-1\"/></SegmentTimeline></SegmentTemplate>";
        let text = dynamic("", &format!("<Period>{}</Period>", video(timeline)));
        let formats = parse(&text, &manifest()).unwrap();
        assert_eq!(
            paths(&formats[0]),
            ["40.m4s", "50.m4s", "60.m4s", "100.m4s", "110.m4s"]
        );
        let durations: Vec<_> = formats[0].fragments.iter().map(|f| f.duration).collect();
        assert_eq!(
            durations,
            [Some(1.0), Some(1.0), Some(2.0), Some(1.0), Some(0.5)]
        );
        // The same timeline is out of order for a static presentation.
        let fixed = text.replace(
            "type=\"dynamic\"",
            "type=\"static\" mediaPresentationDuration=\"PT20S\"",
        );
        assert!(parse(&fixed, &manifest()).is_err());
    }
    #[test]
    fn dynamic_period_durations_are_inferred_from_earlier_fragments_per_period() {
        let timeline = "<SegmentTemplate timescale=\"10\" media=\"$RepresentationID$-$Time$.m4s\"><SegmentTimeline><S d=\"20\" r=\"1\"/><S d=\"15\"/></SegmentTimeline></SegmentTemplate>";
        // 5.5 seconds of video: ceil(5.5 / 2) audio segments.
        let text = dynamic(
            "",
            &format!(
                "<Period id=\"1\">{}{}</Period><Period id=\"2\">{}</Period>",
                video(timeline),
                audio(NUMBER),
                // Unequal formats, which `_merge_mpd_periods` keeps apart.
                audio(NUMBER).replace("id=\"a\" bandwidth=\"1000\"", "id=\"b\" bandwidth=\"2000\"")
            ),
        );
        let formats = parse(&text, &manifest()).unwrap();
        assert_eq!(
            paths(&formats[1]),
            ["init-a.mp4", "a-1.m4s", "a-2.m4s", "a-3.m4s"]
        );
        // Each Period starts unknown again.
        assert_eq!(paths(&formats[2]), ["init-b.mp4"]);
        // An earlier representation only: the audio comes first here.
        let text = dynamic(
            "",
            &format!("<Period>{}{}</Period>", audio(NUMBER), video(timeline)),
        );
        assert_eq!(
            paths(&parse(&text, &manifest()).unwrap()[0]),
            ["init-a.mp4"]
        );
        // List segments without durations infer nothing (`KeyError`).
        let list = "<SegmentList><SegmentURL media=\"l1.m4s\"/></SegmentList>";
        let text = dynamic(
            "",
            &format!("<Period>{}{}</Period>", video(list), audio(NUMBER)),
        );
        assert_eq!(
            paths(&parse(&text, &manifest()).unwrap()[1]),
            ["init-a.mp4"]
        );
        // Storyboards are not formats here, but upstream reads their fragments.
        let storyboard = format!(
            "<AdaptationSet mimeType=\"image/jpeg\">{timeline}<Representation id=\"i\" bandwidth=\"1\"/></AdaptationSet>"
        );
        let text = dynamic(
            "",
            &format!("<Period>{storyboard}{}</Period>", audio(NUMBER)),
        );
        let formats = parse(&text, &manifest()).unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(paths(&formats[0]).len(), 4);
    }
    #[test]
    fn dynamic_segment_lists_take_one_url_per_repeat_and_drop_the_rest() {
        let list = |urls: usize| {
            format!(
                "<SegmentList timescale=\"10\"><SegmentTimeline><S t=\"50\" d=\"10\" r=\"1\"/><S d=\"20\" r=\"-1\"/><S d=\"5\"/></SegmentTimeline>{}</SegmentList>",
                (1..=urls)
                    .map(|index| format!("<SegmentURL media=\"l{index}.m4s\"/>"))
                    .collect::<String>()
            )
        };
        let formats = parse(
            &dynamic("", &format!("<Period>{}</Period>", video(&list(4)))),
            &manifest(),
        )
        .unwrap();
        assert_eq!(paths(&formats[0]), ["l1.m4s", "l2.m4s", "l3.m4s"]);
        let durations: Vec<_> = formats[0].fragments.iter().map(|f| f.duration).collect();
        assert_eq!(durations, [Some(1.0), Some(1.0), Some(0.5)]);
        // Upstream indexes past the URLs (`IndexError`).
        assert!(
            parse(
                &dynamic("", &format!("<Period>{}</Period>", video(&list(2)))),
                &manifest()
            )
            .is_err()
        );
    }
    #[test]
    fn locations_and_availability_hints_are_ignored() {
        let hinted = NUMBER.replace(
            "duration=",
            "availabilityTimeOffset=\"1.5\" availabilityTimeComplete=\"false\" duration=",
        );
        let body = format!(
            "<Location>https://example.com/next.mpd</Location><PatchLocation>patch.mpp</PatchLocation><Period duration=\"PT2S\">{}</Period>",
            video(&hinted)
        );
        let formats = parse(&dynamic("", &body), &manifest()).unwrap();
        assert_eq!(paths(&formats[0]), ["init-v.mp4", "v-1.m4s"]);
        // Upstream reads neither in static MPDs either.
        let fixed = dynamic("", &body).replace("type=\"dynamic\"", "type=\"static\"");
        assert!(parse(&fixed, &manifest()).is_ok());
        // Nor a Location elsewhere.
        let misplaced = dynamic(
            "",
            &format!(
                "<Period><Location>next.mpd</Location>{}</Period>",
                video(NUMBER)
            ),
        );
        assert!(parse(&misplaced, &manifest()).is_ok());
        // Without a timeline upstream needs `$Number$` (`KeyError: 's'`).
        let time = NUMBER.replace("$Number$", "$Time$");
        assert!(
            parse(
                &dynamic("", &format!("<Period>{}</Period>", video(&time))),
                &manifest()
            )
            .is_err()
        );
    }
}
