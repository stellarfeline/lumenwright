//! Small helpers for poking at untyped NBT (`fastnbt::Value`).

use std::collections::HashMap;

use fastnbt::Value;

/// An NBT compound.
pub type Compound = HashMap<String, Value>;

/// The value as a compound, if it is one.
pub fn compound(v: &Value) -> Option<&Compound> {
    match v {
        Value::Compound(c) => Some(c),
        _ => None,
    }
}

/// Looks up a dotted path (`a.b.c`) through nested compounds.
pub fn get<'a>(c: &'a Compound, path: &str) -> Option<&'a Value> {
    let mut cur: &Compound = c;
    let mut parts = path.split('.').peekable();
    while let Some(p) = parts.next() {
        let v = cur.get(p)?;
        if parts.peek().is_none() {
            return Some(v);
        }
        cur = compound(v)?;
    }
    None
}

/// Any numeric value as `f64`.
pub fn as_f64(v: &Value) -> Option<f64> {
    Some(match v {
        Value::Byte(x) => *x as f64,
        Value::Short(x) => *x as f64,
        Value::Int(x) => *x as f64,
        Value::Long(x) => *x as f64,
        Value::Float(x) => *x as f64,
        Value::Double(x) => *x,
        _ => return None,
    })
}

/// Any numeric value as `i64`, truncated.
pub fn as_i64(v: &Value) -> Option<i64> {
    as_f64(v).map(|f| f as i64)
}

/// The value as a string, if it is one.
pub fn as_str(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s),
        _ => None,
    }
}

/// Reads a list (or int array) of numbers.
pub fn as_f64_list(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::List(l) => l.iter().map(as_f64).collect(),
        Value::IntArray(a) => Some(a.iter().map(|x| *x as f64).collect()),
        _ => None,
    }
}

/// A numeric value at a dotted path.
pub fn get_f64(c: &Compound, path: &str) -> Option<f64> {
    get(c, path).and_then(as_f64)
}

/// A string value at a dotted path.
pub fn get_str<'a>(c: &'a Compound, path: &str) -> Option<&'a str> {
    get(c, path).and_then(as_str)
}

/// Whether a numeric value at a dotted path is present and non-zero.
pub fn get_bool(c: &Compound, path: &str) -> bool {
    get(c, path).and_then(as_i64).unwrap_or(0) != 0
}
