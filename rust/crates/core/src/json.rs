//! A small JSON reader and writer that behaves like Python's `json` module.
//!
//! The Python version of Real-Hi-Res saves results with `json.dumps`, which
//! writes `NaN` and `Infinity` (standard JSON cannot hold them) and Python's own
//! float notation (`25.0`, `1e-05`).  Results and settings must survive a trip
//! between the two programs, and `settings_key()` must hash the very same text,
//! so this module reads and writes exactly that dialect.  Objects keep their
//! key order, as Python dicts do.

use std::fmt::Write as _;

use crate::pyfmt::repr_float;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The number as a float (ints included), or None.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// The number as an integer (a float with no fraction too), or None.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) if f.is_finite() && f.fract() == 0.0 => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Vec<(String, Value)>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Python truthiness (`bool(value)`).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}
impl From<u32> for Value {
    fn from(i: u32) -> Self {
        Value::Int(i as i64)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Self {
        o.map(Into::into).unwrap_or(Value::Null)
    }
}

/// Builds an object while keeping the order of its keys.
#[derive(Default)]
pub struct Obj(pub Vec<(String, Value)>);

impl Obj {
    pub fn new() -> Self {
        Obj(Vec::new())
    }
    pub fn put(mut self, key: &str, v: impl Into<Value>) -> Self {
        self.0.push((key.to_string(), v.into()));
        self
    }
    pub fn push(&mut self, key: &str, v: impl Into<Value>) {
        self.0.push((key.to_string(), v.into()));
    }
    pub fn build(self) -> Value {
        Value::Object(self.0)
    }
}

// ------------------------------------------------------------------ writing ---

/// How to lay out the text, mirroring `json.dumps` arguments.
#[derive(Clone, Copy)]
pub struct Style {
    /// `indent=` (None = everything on one line).
    pub indent: Option<usize>,
    /// `separators=(",", ":")` when true, else Python's defaults.
    pub compact: bool,
    /// `ensure_ascii=` (escape every non-ASCII character).
    pub ensure_ascii: bool,
    /// `sort_keys=`.
    pub sort_keys: bool,
}

impl Style {
    /// `json.dumps(v)`.
    pub const DEFAULT: Style = Style { indent: None, compact: false, ensure_ascii: true, sort_keys: false };
    /// `json.dumps(v, separators=(",", ":"))`.
    pub const COMPACT: Style = Style { indent: None, compact: true, ensure_ascii: true, sort_keys: false };
    /// `json.dump(v, f, indent=2, ensure_ascii=False)`.
    pub const PRETTY: Style = Style { indent: Some(2), compact: false, ensure_ascii: false, sort_keys: false };
}

pub fn dumps(v: &Value, style: Style) -> String {
    let mut out = String::new();
    write_value(&mut out, v, style, 0);
    out
}

fn write_value(out: &mut String, v: &Value, style: Style, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Float(f) => out.push_str(&float_text(*f)),
        Value::Str(s) => write_string(out, s, style.ensure_ascii),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep(style));
                }
                newline(out, style, level + 1);
                write_value(out, item, style, level + 1);
            }
            newline(out, style, level);
            out.push(']');
        }
        Value::Object(items) => {
            if items.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut order: Vec<&(String, Value)> = items.iter().collect();
            if style.sort_keys {
                order.sort_by(|a, b| a.0.cmp(&b.0));
            }
            out.push('{');
            for (i, (k, item)) in order.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep(style));
                }
                newline(out, style, level + 1);
                write_string(out, k, style.ensure_ascii);
                out.push_str(if style.compact { ":" } else { ": " });
                write_value(out, item, style, level + 1);
            }
            newline(out, style, level);
            out.push('}');
        }
    }
}

fn item_sep(style: Style) -> &'static str {
    if style.compact || style.indent.is_some() {
        ","
    } else {
        ", "
    }
}

fn newline(out: &mut String, style: Style, level: usize) {
    if let Some(n) = style.indent {
        out.push('\n');
        for _ in 0..n * level {
            out.push(' ');
        }
    }
}

/// A float as `json.dumps` writes it.
pub fn float_text(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f == f64::INFINITY {
        "Infinity".into()
    } else if f == f64::NEG_INFINITY {
        "-Infinity".into()
    } else {
        repr_float(f)
    }
}

fn write_string(out: &mut String, s: &str, ensure_ascii: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if ensure_ascii && (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{:04x}", unit);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ------------------------------------------------------------------ reading ---

#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid JSON: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("extra data after the value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> ParseError {
        ParseError(format!("{msg} at byte {}", self.i))
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > 200 {
            return Err(self.err("nesting too deep"));
        }
        let Some(&c) = self.s.get(self.i) else {
            return Err(self.err("unexpected end"));
        };
        match c {
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("}") {
                    return Ok(Value::Object(items));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(self.err("expected a key"));
                    }
                    let k = self.string()?;
                    self.ws();
                    if !self.eat(":") {
                        return Err(self.err("expected ':'"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    items.push((k, v));
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("}") {
                        return Ok(Value::Object(items));
                    }
                    return Err(self.err("expected ',' or '}'"));
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat("]") {
                    return Ok(Value::Array(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(",") {
                        continue;
                    }
                    if self.eat("]") {
                        return Ok(Value::Array(items));
                    }
                    return Err(self.err("expected ',' or ']'"));
                }
            }
            b'"' => Ok(Value::Str(self.string()?)),
            b't' if self.eat("true") => Ok(Value::Bool(true)),
            b'f' if self.eat("false") => Ok(Value::Bool(false)),
            b'n' if self.eat("null") => Ok(Value::Null),
            b'N' if self.eat("NaN") => Ok(Value::Float(f64::NAN)),
            b'I' if self.eat("Infinity") => Ok(Value::Float(f64::INFINITY)),
            b'-' if self.eat("-Infinity") => Ok(Value::Float(f64::NEG_INFINITY)),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.i;
        let mut float = false;
        while let Some(&c) = self.s.get(self.i) {
            match c {
                b'0'..=b'9' | b'-' | b'+' => {}
                b'.' | b'e' | b'E' => float = true,
                _ => break,
            }
            self.i += 1;
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("bad number"))?;
        if !float {
            if let Ok(i) = text.parse::<i64>() {
                return Ok(Value::Int(i));
            }
        }
        text.parse::<f64>().map(Value::Float).map_err(|_| self.err("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("short \\u escape"))?;
        let h = std::str::from_utf8(h).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1; // opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else {
                return Err(self.err("unterminated string"));
            };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else {
                        return Err(self.err("unterminated escape"));
                    };
                    self.i += 1;
                    let ch: char = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{08}',
                        b'f' => '\u{0c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                } else {
                                    self.i = save;
                                }
                            }
                            char::from_u32(cp).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.err("bad escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_python_dialect() {
        let text = r#"{"a":NaN,"b":[1,2.5,-Infinity,Infinity],"c":"\u00e9\ud83c\udfb5","d":null,"e":1e-05}"#;
        let v = parse(text).unwrap();
        assert!(v.get("a").unwrap().as_f64().unwrap().is_nan());
        assert_eq!(v.get("c").unwrap().as_str().unwrap(), "é🎵");
        assert_eq!(v.get("e").unwrap().as_f64().unwrap(), 1e-5);
        let again = dumps(&v, Style::COMPACT);
        assert_eq!(again, r#"{"a":NaN,"b":[1,2.5,-Infinity,Infinity],"c":"\u00e9\ud83c\udfb5","d":null,"e":1e-05}"#);
    }

    #[test]
    fn pretty_matches_python_indent() {
        let v = Obj::new().put("x", Value::Array(vec![])).put("y", Value::Array(vec![1i64.into(), 2.0.into()])).build();
        assert_eq!(dumps(&v, Style::PRETTY), "{\n  \"x\": [],\n  \"y\": [\n    1,\n    2.0\n  ]\n}");
        assert_eq!(dumps(&v, Style::DEFAULT), "{\"x\": [], \"y\": [1, 2.0]}");
    }
}
