//! Parser for the Lua files WoW writes for SavedVariables.
//!
//! Those files are a list of `Name = value` assignments where values are
//! strings, numbers, booleans, nil or tables. Tables become JSON arrays when
//! their keys are exactly 1..n, and JSON objects otherwise.

use serde_json::{Map, Number, Value};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parses a whole SavedVariables file into `{ variableName: value }`.
pub fn parse_saved_variables(src: &str) -> Result<Map<String, Value>, ParseError> {
    let mut p = Parser { src: src.as_bytes(), pos: 0 };
    let mut vars = Map::new();
    loop {
        p.skip_trivia()?;
        if p.at_end() {
            return Ok(vars);
        }
        let name = p.ident().ok_or_else(|| p.error("expected a variable name"))?;
        p.skip_trivia()?;
        p.expect(b'=')?;
        let value = p.value()?;
        vars.insert(name, value);
        p.skip_trivia()?;
        p.eat(b';');
    }
}

/// Parses a single Lua value (used by tests).
pub fn parse_value(src: &str) -> Result<Value, ParseError> {
    let mut p = Parser { src: src.as_bytes(), pos: 0 };
    let v = p.value()?;
    p.skip_trivia()?;
    if !p.at_end() {
        return Err(p.error("unexpected trailing input"));
    }
    Ok(v)
}

enum Key {
    Positional,
    Int(i64),
    Str(String),
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", c as char)))
        }
    }

    fn error(&self, message: &str) -> ParseError {
        let end = self.pos.min(self.src.len());
        let line = self.src[..end].iter().filter(|&&b| b == b'\n').count() + 1;
        ParseError { line, message: message.to_string() }
    }

    /// Skips whitespace and `--` / `--[[ ]]` / `--[==[ ]==]` comments.
    fn skip_trivia(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'-') if self.peek_at(1) == Some(b'-') => {
                    self.pos += 2;
                    if let Some(level) = self.long_bracket_level() {
                        self.long_bracket_body(level)?;
                    } else {
                        while let Some(c) = self.peek() {
                            if c == b'\n' {
                                break;
                            }
                            self.pos += 1;
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// If at `[[` or `[=*[`, consumes it and returns the number of `=`.
    fn long_bracket_level(&mut self) -> Option<usize> {
        if self.peek() != Some(b'[') {
            return None;
        }
        let mut i = 1;
        while self.peek_at(i) == Some(b'=') {
            i += 1;
        }
        if self.peek_at(i) == Some(b'[') {
            self.pos += i + 1;
            Some(i - 1)
        } else {
            None
        }
    }

    /// Consumes up to and including the matching `]=*]`, returning the body.
    fn long_bracket_body(&mut self, level: usize) -> Result<&'a [u8], ParseError> {
        let start = self.pos;
        while !self.at_end() {
            if self.peek() == Some(b']') {
                let mut i = 1;
                while self.peek_at(i) == Some(b'=') {
                    i += 1;
                }
                if i - 1 == level && self.peek_at(i) == Some(b']') {
                    let body = &self.src[start..self.pos];
                    self.pos += i + 1;
                    return Ok(body);
                }
            }
            self.pos += 1;
        }
        Err(self.error("unterminated long bracket"))
    }

    fn ident(&mut self) -> Option<String> {
        let start = self.pos;
        match self.peek() {
            Some(c) if c == b'_' || c.is_ascii_alphabetic() => self.pos += 1,
            _ => return None,
        }
        while let Some(c) = self.peek() {
            if c == b'_' || c.is_ascii_alphanumeric() {
                self.pos += 1;
            } else {
                break;
            }
        }
        Some(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        self.skip_trivia()?;
        match self.peek() {
            Some(b'{') => self.table(),
            Some(b'"' | b'\'') => Ok(Value::String(self.string()?)),
            Some(b'[') => {
                let level = self.long_bracket_level().ok_or_else(|| self.error("unexpected '['"))?;
                let mut body = self.long_bracket_body(level)?;
                // A newline right after the opening bracket is skipped.
                if body.first() == Some(&b'\r') {
                    body = &body[1..];
                }
                if body.first() == Some(&b'\n') {
                    body = &body[1..];
                }
                Ok(Value::String(String::from_utf8_lossy(body).into_owned()))
            }
            Some(c) if c == b'-' || c == b'.' || c.is_ascii_digit() => self.number(),
            Some(_) => {
                let start = self.pos;
                match self.ident().as_deref() {
                    Some("true") => Ok(Value::Bool(true)),
                    Some("false") => Ok(Value::Bool(false)),
                    Some("nil") => Ok(Value::Null),
                    Some("inf") => Ok(Value::Null),
                    Some("nan") => Ok(Value::Null),
                    _ => {
                        self.pos = start;
                        Err(self.error("expected a value"))
                    }
                }
            }
            None => Err(self.error("unexpected end of file")),
        }
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        self.eat(b'-');
        // `-inf` / `-nan` as written by some Lua versions.
        if matches!(self.peek(), Some(b'i' | b'n')) {
            self.ident();
            return Ok(Value::Null);
        }
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'X')) {
            self.pos += 2;
            let hex_start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                self.pos += 1;
            }
            let digits = std::str::from_utf8(&self.src[hex_start..self.pos]).unwrap_or("");
            let n = i64::from_str_radix(digits, 16).map_err(|_| self.error("bad hex number"))?;
            let negative = self.src[start] == b'-';
            return Ok(Value::Number(Number::from(if negative { -n } else { n })));
        }
        while let Some(c) = self.peek() {
            let exponent_sign = matches!(c, b'+' | b'-') && matches!(self.src[self.pos - 1], b'e' | b'E');
            if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E') || exponent_sign {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        if let Ok(i) = text.parse::<i64>() {
            return Ok(Value::Number(Number::from(i)));
        }
        let f: f64 = text.parse().map_err(|_| self.error(&format!("bad number '{text}'")))?;
        Ok(Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        let quote = self.peek().unwrap();
        self.pos += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or_else(|| self.error("unterminated string"))?;
            self.pos += 1;
            if c == quote {
                break;
            }
            if c == b'\n' {
                return Err(self.error("newline in string"));
            }
            if c != b'\\' {
                out.push(c);
                continue;
            }
            let e = self.peek().ok_or_else(|| self.error("unterminated string"))?;
            self.pos += 1;
            match e {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(0x07),
                b'b' => out.push(0x08),
                b'f' => out.push(0x0c),
                b'v' => out.push(0x0b),
                b'\\' | b'"' | b'\'' => out.push(e),
                b'\n' => out.push(b'\n'),
                b'\r' => {
                    self.eat(b'\n');
                    out.push(b'\n');
                }
                b'0'..=b'9' => {
                    let mut n = u32::from(e - b'0');
                    for _ in 0..2 {
                        match self.peek() {
                            Some(d @ b'0'..=b'9') => {
                                n = n * 10 + u32::from(d - b'0');
                                self.pos += 1;
                            }
                            _ => break,
                        }
                    }
                    out.push(u8::try_from(n).map_err(|_| self.error("escape out of range"))?);
                }
                other => {
                    // Unknown escape: keep it as written.
                    out.push(b'\\');
                    out.push(other);
                }
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn table(&mut self) -> Result<Value, ParseError> {
        self.expect(b'{')?;
        let mut entries: Vec<(Key, Value)> = Vec::new();
        loop {
            self.skip_trivia()?;
            if self.eat(b'}') {
                break;
            }
            let key = if self.peek() == Some(b'[') && !matches!(self.peek_at(1), Some(b'[' | b'=')) {
                self.pos += 1;
                let k = self.value()?;
                self.skip_trivia()?;
                self.expect(b']')?;
                self.skip_trivia()?;
                self.expect(b'=')?;
                match k {
                    Value::String(s) => Key::Str(s),
                    Value::Number(n) => match n.as_i64() {
                        Some(i) => Key::Int(i),
                        None => Key::Str(n.to_string()),
                    },
                    Value::Bool(b) => Key::Str(b.to_string()),
                    _ => return Err(self.error("unsupported table key")),
                }
            } else {
                let save = self.pos;
                match self.ident() {
                    Some(name) => {
                        self.skip_trivia()?;
                        if self.eat(b'=') {
                            Key::Str(name)
                        } else {
                            self.pos = save;
                            Key::Positional
                        }
                    }
                    None => Key::Positional,
                }
            };
            let value = self.value()?;
            entries.push((key, value));
            self.skip_trivia()?;
            if !(self.eat(b',') || self.eat(b';')) {
                self.skip_trivia()?;
                self.expect(b'}')?;
                break;
            }
        }
        Ok(build_table(entries))
    }
}

fn build_table(entries: Vec<(Key, Value)>) -> Value {
    // Resolve positional entries to 1-based indices.
    let mut next_index = 1i64;
    let mut resolved: Vec<(Result<i64, String>, Value)> = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let k = match key {
            Key::Positional => {
                let i = next_index;
                next_index += 1;
                Ok(i)
            }
            Key::Int(i) => Ok(i),
            Key::Str(s) => Err(s),
        };
        resolved.push((k, value));
    }

    // Exactly the keys 1..n (nil values dropped, as Lua would) -> array.
    let present: Vec<&(Result<i64, String>, Value)> =
        resolved.iter().filter(|(_, v)| !v.is_null()).collect();
    let all_ints = present.iter().all(|(k, _)| k.is_ok());
    if all_ints {
        let mut ints: Vec<(i64, &Value)> = present.iter().map(|(k, v)| (*k.as_ref().unwrap(), v)).collect();
        ints.sort_by_key(|(k, _)| *k);
        if ints.iter().enumerate().all(|(i, (k, _))| *k == i as i64 + 1) {
            return Value::Array(ints.into_iter().map(|(_, v)| v.clone()).collect());
        }
    }

    let mut map = Map::new();
    for (k, v) in resolved {
        if v.is_null() {
            continue;
        }
        let key = match k {
            Ok(i) => i.to_string(),
            Err(s) => s,
        };
        map.insert(key, v);
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_a_wow_style_file() {
        let src = r#"
WowTrackerDB = {
["settings"] = {
["endpoint"] = "192.168.0.117:1337",
["trail"] = true,
["token"] = "",
},
["characters"] = {
["Thrall-Firemaw"] = {
["level"] = 12,
["x"] = 0.4123,
["zone"] = "Elwynn Forest",
["trail"] = {
{
1730000000, -- [1]
1429, -- [2]
0.41, -- [3]
0.65, -- [4]
}, -- [1]
},
},
},
["schema"] = 1,
}
OtherVar = nil
"#;
        let vars = parse_saved_variables(src).unwrap();
        let db = &vars["WowTrackerDB"];
        assert_eq!(db["settings"]["endpoint"], "192.168.0.117:1337");
        assert_eq!(db["settings"]["trail"], true);
        let ch = &db["characters"]["Thrall-Firemaw"];
        assert_eq!(ch["level"], 12);
        assert_eq!(ch["x"], 0.4123);
        assert_eq!(ch["trail"], json!([[1730000000, 1429, 0.41, 0.65]]));
        assert_eq!(vars["OtherVar"], Value::Null);
    }

    #[test]
    fn string_escapes() {
        assert_eq!(parse_value(r#""a\"b\\c\nd""#).unwrap(), json!("a\"b\\c\nd"));
        assert_eq!(parse_value(r#""\65\066""#).unwrap(), json!("AB"));
        assert_eq!(parse_value("'single'").unwrap(), json!("single"));
        assert_eq!(parse_value("[[long\nstring]]").unwrap(), json!("long\nstring"));
        // UTF-8 names survive.
        assert_eq!(parse_value("\"Zul'jïn\"").unwrap(), json!("Zul'jïn"));
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_value("-12").unwrap(), json!(-12));
        assert_eq!(parse_value("1.5e3").unwrap(), json!(1500.0));
        assert_eq!(parse_value("0.0001").unwrap(), json!(0.0001));
        assert_eq!(parse_value("0x1F").unwrap(), json!(31));
    }

    #[test]
    fn tables() {
        assert_eq!(parse_value("{}").unwrap(), json!([]));
        assert_eq!(parse_value("{ 1, 2; 3 }").unwrap(), json!([1, 2, 3]));
        assert_eq!(parse_value("{ [2] = 'b', [1] = 'a' }").unwrap(), json!(["a", "b"]));
        assert_eq!(parse_value("{ [1] = 'a', [3] = 'c' }").unwrap(), json!({"1": "a", "3": "c"}));
        assert_eq!(parse_value("{ a = 1, ['b'] = 2 }").unwrap(), json!({"a": 1, "b": 2}));
        assert_eq!(parse_value("{ 'x', key = 'y' }").unwrap(), json!({"1": "x", "key": "y"}));
        assert_eq!(parse_value("{ a = nil, b = 1 }").unwrap(), json!({"b": 1}));
    }

    #[test]
    fn comments() {
        let v = parse_value("{ --[[ block ]] 1, -- line\n 2 --[==[ x ]==] }").unwrap();
        assert_eq!(v, json!([1, 2]));
    }

    #[test]
    fn reports_line_of_error() {
        let err = parse_saved_variables("A = {\n1,\n@\n}").unwrap_err();
        assert_eq!(err.line, 3);
    }
}
