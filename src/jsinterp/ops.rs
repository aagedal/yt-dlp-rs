//! Upstream's `_OPERATORS` table: the `_js_*` helpers over Python's own
//! operators, `int_to_int32` and `js_number_to_string`.
use super::value::{
    JsError, JsValue, Num, check_hashable, num, py_eq, py_is, py_str, py_truthy, type_name,
};
use std::cmp::Ordering;

/// Upstream `int_to_int32(n)`: a signed 32-bit integer.
pub fn int_to_int32(n: i128) -> i64 {
    let n = n & 0xFFFF_FFFF;
    (if n & 0x8000_0000 != 0 {
        n - 0x1_0000_0000
    } else {
        n
    }) as i64
}

fn unsupported(op: &str, a: &JsValue, b: &JsValue) -> JsError {
    JsError::type_error(format!(
        "unsupported operand type(s) for {op}: '{}' and '{}'",
        type_name(a),
        type_name(b)
    ))
}

/// `a or 0`.
fn or_zero(value: &JsValue) -> JsValue {
    if py_truthy(value) {
        value.clone()
    } else {
        JsValue::Int(0)
    }
}

fn int_or_float(value: Option<i64>, fallback: f64) -> JsValue {
    // Python integers do not overflow; past `i64` this keeps a float.
    value.map_or(JsValue::Float(fallback), JsValue::Int)
}

/// Python `float(text)`.
pub(super) fn py_float_parse(text: &str) -> Option<f64> {
    let text = crate::batch::py_strip(text);
    let lower = text.to_ascii_lowercase();
    let unsigned = lower.trim_start_matches(['+', '-']);
    if matches!(unsigned, "inf" | "infinity" | "nan") {
        return lower.parse().ok();
    }
    if !text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-' | '_'))
    {
        return None;
    }
    if text.starts_with('_') || text.ends_with('_') || text.contains("__") {
        return None;
    }
    text.replace('_', "").parse().ok()
}

/// Python `int(float)`, kept to 128 bits (only its low 32 bits are used
/// by the bitwise operators; larger floats are multiples of 2**32).
fn float_to_int(value: f64) -> Result<i128, JsError> {
    if value.is_nan() {
        return Err(JsError::python(
            "ValueError",
            "cannot convert float NaN to integer",
        ));
    }
    if value.is_infinite() {
        return Err(JsError::python(
            "OverflowError",
            "cannot convert float infinity to integer",
        ));
    }
    let value = value.trunc();
    Ok(if value.abs() < 2f64.powi(126) {
        value as i128
    } else {
        0
    })
}

/// `_js_bit_op`'s `zeroise`: `0` for `None`, `JS_Undefined` and NaN, else
/// `int(float(x))`.
fn zeroise(value: &JsValue) -> Result<i128, JsError> {
    let float = match value {
        JsValue::Null | JsValue::Undefined => return Ok(0),
        JsValue::Float(f) if f.is_nan() => return Ok(0),
        JsValue::Str(text) => py_float_parse(text).ok_or_else(|| {
            JsError::python(
                "ValueError",
                format!(
                    "could not convert string to float: {}",
                    crate::batch::py_repr(text)
                ),
            )
        })?,
        other => match num(other) {
            Some(n) => n.float(),
            None => {
                return Err(JsError::type_error(format!(
                    "float() argument must be a string or a real number, not '{}'",
                    type_name(other)
                )));
            }
        },
    };
    float_to_int(float)
}

fn bit_op(op: &str, a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    let a = i128::from(int_to_int32(zeroise(a)?));
    let b = i128::from(int_to_int32(zeroise(b)?));
    let shift = |b: i128| {
        if b < 0 {
            Err(JsError::python("ValueError", "negative shift count"))
        } else {
            Ok(b)
        }
    };
    let result = match op {
        "|" => a | b,
        "^" => a ^ b,
        "&" => a & b,
        // The low 32 bits of a longer left shift are zero.
        "<<" => {
            let b = shift(b)?;
            if b >= 64 { 0 } else { a << b }
        }
        ">>" => a >> shift(b)?.min(127),
        _ => unreachable!("bitwise operator"),
    };
    Ok(JsValue::Int(int_to_int32(result)))
}

pub(super) fn py_add(a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return Ok(match (x, y) {
            (Num::I(x), Num::I(y)) => int_or_float(x.checked_add(y), x as f64 + y as f64),
            (x, y) => JsValue::Float(x.float() + y.float()),
        });
    }
    match (a, b) {
        (JsValue::Str(x), JsValue::Str(y)) => Ok(JsValue::str(format!("{x}{y}"))),
        (JsValue::List(x), JsValue::List(y)) => {
            let mut items = x.borrow().clone();
            items.extend(y.borrow().iter().cloned());
            Ok(JsValue::list(items))
        }
        _ => Err(unsupported("+", a, b)),
    }
}

fn py_sub(a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    match (num(a), num(b)) {
        (Some(Num::I(x)), Some(Num::I(y))) => {
            Ok(int_or_float(x.checked_sub(y), x as f64 - y as f64))
        }
        (Some(x), Some(y)) => Ok(JsValue::Float(x.float() - y.float())),
        _ => Err(unsupported("-", a, b)),
    }
}

/// A sequence repeated `count` times.
fn repeat(count: i64, length: usize) -> Result<usize, JsError> {
    let count = usize::try_from(count.max(0)).unwrap_or(usize::MAX);
    count
        .checked_mul(length)
        .filter(|total| *total < 1 << 32)
        .map(|_| count)
        .ok_or_else(|| JsError::python("MemoryError", ""))
}

fn py_mul(a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    match (num(a), num(b)) {
        (Some(Num::I(x)), Some(Num::I(y))) => {
            return Ok(int_or_float(x.checked_mul(y), x as f64 * y as f64));
        }
        (Some(x), Some(y)) => return Ok(JsValue::Float(x.float() * y.float())),
        _ => {}
    }
    let (sequence, other) = match (a, b) {
        (JsValue::Str(_) | JsValue::List(_), other) => (a, other),
        (other, JsValue::Str(_) | JsValue::List(_)) => (b, other),
        _ => return Err(unsupported("*", a, b)),
    };
    let count = match other {
        JsValue::Int(i) => *i,
        JsValue::Bool(b) => i64::from(*b),
        other => {
            return Err(JsError::type_error(format!(
                "can't multiply sequence by non-int of type '{}'",
                type_name(other)
            )));
        }
    };
    match sequence {
        JsValue::Str(text) => Ok(JsValue::str(text.repeat(repeat(count, text.len())?))),
        JsValue::List(items) => {
            let items = items.borrow();
            let count = repeat(count, items.len())?;
            let mut out = Vec::with_capacity(items.len() * count);
            for _ in 0..count {
                out.extend(items.iter().cloned());
            }
            Ok(JsValue::list(out))
        }
        _ => unreachable!("sequence"),
    }
}

fn py_truediv(a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    match (num(a), num(b)) {
        (Some(x), Some(y)) => {
            if y.float() == 0.0 {
                Err(JsError::python("ZeroDivisionError", "division by zero"))
            } else {
                Ok(JsValue::Float(x.float() / y.float()))
            }
        }
        _ => Err(unsupported("/", a, b)),
    }
}

/// Python `%` of numbers: the result has the divisor's sign.
fn py_mod(a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    match (num(a), num(b)) {
        (Some(Num::I(x)), Some(Num::I(y))) => {
            if y == 0 {
                return Err(JsError::python(
                    "ZeroDivisionError",
                    "integer modulo by zero",
                ));
            }
            Ok(match x.checked_rem(y) {
                Some(r) if r != 0 && (r < 0) != (y < 0) => JsValue::Int(r + y),
                Some(r) => JsValue::Int(r),
                None => JsValue::Int(0),
            })
        }
        (Some(x), Some(y)) => {
            let (x, y) = (x.float(), y.float());
            if y == 0.0 {
                return Err(JsError::python("ZeroDivisionError", "float modulo"));
            }
            let mut r = x % y;
            if r != 0.0 {
                if (y < 0.0) != (r < 0.0) {
                    r += y;
                }
            } else {
                r = 0f64.copysign(y);
            }
            Ok(JsValue::Float(r))
        }
        // `str % args` is string formatting upstream; not supported here.
        _ => Err(JsError::type_error(format!(
            "unsupported operand type(s) for %: '{}' and '{}'",
            type_name(a),
            type_name(b)
        ))),
    }
}

/// Python `a ** b` of numbers.
pub(super) fn py_pow(a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    let zero_negative = || {
        JsError::python(
            "ZeroDivisionError",
            "0.0 cannot be raised to a negative power",
        )
    };
    match (num(a), num(b)) {
        (Some(Num::I(x)), Some(Num::I(y))) => {
            if y >= 0 {
                let exact = u32::try_from(y).ok().and_then(|y| x.checked_pow(y));
                return Ok(int_or_float(exact, (x as f64).powf(y as f64)));
            }
            if x == 0 {
                return Err(zero_negative());
            }
            Ok(JsValue::Float((x as f64).powf(y as f64)))
        }
        (Some(x), Some(y)) => {
            let (x, y) = (x.float(), y.float());
            if x == 0.0 && y < 0.0 {
                return Err(zero_negative());
            }
            if x < 0.0 && y.is_finite() && y.fract() != 0.0 {
                // Python returns a complex number here.
                return Err(JsError::type_error("complex results are not supported"));
            }
            let result = x.powf(y);
            if result.is_infinite() && x.is_finite() && y.is_finite() {
                return Err(JsError::python(
                    "OverflowError",
                    "(34, 'Numerical result out of range')",
                ));
            }
            Ok(JsValue::Float(result))
        }
        _ => Err(JsError::type_error(format!(
            "unsupported operand type(s) for ** or pow(): '{}' and '{}'",
            type_name(a),
            type_name(b)
        ))),
    }
}

/// Python's ordering of two values (`None` when unordered, like NaN).
fn py_cmp(op: &str, a: &JsValue, b: &JsValue) -> Result<Option<Ordering>, JsError> {
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        return Ok(match (x, y) {
            (Num::I(x), Num::I(y)) => Some(x.cmp(&y)),
            (x, y) => x.float().partial_cmp(&y.float()),
        });
    }
    match (a, b) {
        (JsValue::Str(x), JsValue::Str(y)) => Ok(Some(x.cmp(y))),
        (JsValue::List(x), JsValue::List(y)) => {
            let (x, y) = (x.borrow().clone(), y.borrow().clone());
            for (a, b) in x.iter().zip(y.iter()) {
                if !(py_is(a, b) || py_eq(a, b)) {
                    return py_cmp(op, a, b);
                }
            }
            Ok(Some(x.len().cmp(&y.len())))
        }
        _ => Err(JsError::type_error(format!(
            "'{op}' not supported between instances of '{}' and '{}'",
            type_name(a),
            type_name(b)
        ))),
    }
}

fn compare(op: &str, a: &JsValue, b: &JsValue) -> Result<bool, JsError> {
    let ordering = py_cmp(op, a, b)?;
    Ok(ordering.is_some_and(|ordering| match op {
        "<" => ordering == Ordering::Less,
        "<=" => ordering != Ordering::Greater,
        ">" => ordering == Ordering::Greater,
        ">=" => ordering != Ordering::Less,
        _ => unreachable!("comparison"),
    }))
}

/// Applies a binary operator of `_OPERATORS` that has a function.
pub(super) fn apply(op: &str, a: &JsValue, b: &JsValue) -> Result<JsValue, JsError> {
    let undefined = matches!(a, JsValue::Undefined) || matches!(b, JsValue::Undefined);
    let nan = JsValue::Float(f64::NAN);
    match op {
        "|" | "^" | "&" | ">>" | "<<" => bit_op(op, a, b),
        "===" => Ok(JsValue::Bool(py_is(a, b))),
        "!==" => Ok(JsValue::Bool(!py_is(a, b))),
        // `_js_eq_op`: `{a, b} <= {None, JS_Undefined}` compares `a` with itself.
        "==" | "!=" => {
            check_hashable(a)?;
            check_hashable(b)?;
            let nullish = |v: &JsValue| matches!(v, JsValue::Null | JsValue::Undefined);
            let equal = (nullish(a) && nullish(b)) || py_eq(a, b);
            Ok(JsValue::Bool(equal == (op == "==")))
        }
        // `_js_comp_op`: strings compare as `str(a or 0)`.
        "<=" | ">=" | "<" | ">" => {
            if undefined {
                return Ok(JsValue::Bool(false));
            }
            let (a, b) = (or_zero(a), or_zero(b));
            if matches!(a, JsValue::Str(_)) || matches!(b, JsValue::Str(_)) {
                return Ok(JsValue::Bool(compare(
                    op,
                    &JsValue::str(py_str(&a)),
                    &JsValue::str(py_str(&b)),
                )?));
            }
            Ok(JsValue::Bool(compare(op, &a, &b)?))
        }
        // `_js_arith_op`.
        "+" | "-" | "*" => {
            if undefined {
                return Ok(nan);
            }
            let (a, b) = (or_zero(a), or_zero(b));
            match op {
                "+" => py_add(&a, &b),
                "-" => py_sub(&a, &b),
                _ => py_mul(&a, &b),
            }
        }
        // `_js_mod`.
        "%" => {
            if undefined || !py_truthy(b) {
                return Ok(nan);
            }
            py_mod(&or_zero(a), b)
        }
        // `_js_div`: `inf` (never `-inf`) for a falsy divisor.
        "/" => {
            let either = if py_truthy(a) { a } else { b };
            if undefined || !py_truthy(either) {
                return Ok(nan);
            }
            if py_truthy(b) {
                py_truediv(&or_zero(a), b)
            } else {
                Ok(JsValue::Float(f64::INFINITY))
            }
        }
        // `_js_exp`: `1` for a falsy exponent, even `0 ** 0`.
        "**" => {
            if !py_truthy(b) {
                return Ok(JsValue::Int(1));
            }
            if undefined {
                return Ok(nan);
            }
            py_pow(&or_zero(a), b)
        }
        _ => unreachable!("operator {op}"),
    }
}

/// Upstream `js_number_to_string(val, radix=10)`: JavaScript's
/// `Number.prototype.toString(radix)` without radix 10's special cases.
///
/// # Panics
///
/// Like upstream's assertion, when `radix` is not in `2..=36`.
pub fn js_number_to_string(value: f64, radix: Option<u32>) -> String {
    let radix = radix.unwrap_or(10);
    assert!(
        (2..=36).contains(&radix),
        "radix must be an integer at least 2 and no greater than 36"
    );
    if value.is_nan() {
        return "NaN".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz.-";
    // Indexes into ALPHABET: 36 is `.`, 37 is `-`.
    let mut result: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    let sign = value < 0.0;
    let value = value.abs();
    let mut integer = value.trunc();
    let mut fraction = value - integer;
    let radix_f = f64::from(radix);
    // `max(math.nextafter(.0, math.inf), math.ulp(val) / 2)`.
    let next = f64::from_bits(value.to_bits() + 1);
    let ulp = if next.is_finite() {
        next - value
    } else {
        value - f64::from_bits(value.to_bits() - 1)
    };
    let mut delta = f64::max(f64::from_bits(1), ulp / 2.0);
    if fraction >= delta {
        result.push_back(36);
    }
    while fraction >= delta {
        delta *= radix_f;
        let scaled = fraction * radix_f;
        let digit = scaled.trunc();
        fraction = scaled - digit;
        let digit = digit as u32;
        result.push_back(digit);
        // If we need to round, propagate potential carry through fractional part.
        let needs_rounding = fraction > 0.5 || (fraction == 0.5 && digit & 1 == 1);
        if needs_rounding && fraction + delta > 1.0 {
            let mut carried = false;
            for index in (1..result.len()).rev() {
                if result[index] + 1 < radix {
                    result[index] += 1;
                    carried = true;
                    break;
                }
                result.pop_back();
            }
            if !carried {
                integer += 1.0;
            }
            break;
        }
    }
    // `int(integer)` exactly, as base-2**32 limbs (least significant first).
    let mut limbs = float_limbs(integer);
    loop {
        let mut remainder = 0u64;
        for limb in limbs.iter_mut().rev() {
            let current = (remainder << 32) | u64::from(*limb);
            *limb = (current / u64::from(radix)) as u32;
            remainder = current % u64::from(radix);
        }
        result.push_front(remainder as u32);
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        if limbs.is_empty() {
            break;
        }
    }
    if sign {
        result.push_front(37);
    }
    result
        .into_iter()
        .map(|digit| char::from(ALPHABET[digit as usize]))
        .collect()
}

/// The exact integer value of a non-negative integral float.
fn float_limbs(value: f64) -> Vec<u32> {
    if value < 1.0 {
        return vec![0];
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64 - 1075;
    let mantissa = (bits & ((1 << 52) - 1)) | (1 << 52);
    if exponent < 0 {
        let integer = mantissa >> (-exponent);
        return vec![integer as u32, (integer >> 32) as u32];
    }
    let shift = exponent as usize;
    let mut limbs = vec![0u32; shift / 32 + 3];
    let wide = u128::from(mantissa) << (shift % 32);
    for (offset, limb) in limbs[shift / 32..].iter_mut().enumerate().take(3) {
        *limb = (wide >> (32 * offset)) as u32;
    }
    limbs
}
