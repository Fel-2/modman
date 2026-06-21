//! Minimal Valve KeyValues (VDF) parser.
//!
//! Handles the quoted-token + brace-block subset used by Steam's
//! `libraryfolders.vdf` and `appmanifest_*.acf`. Not a general VDF
//! implementation: no `#include`, no conditionals, no unquoted keys.

use crate::error::{Error, Result};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Get a child value by key (first match) when this is an object.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, Vdf)]> {
        match self {
            Vdf::Obj(pairs) => Some(pairs),
            Vdf::Str(_) => None,
        }
    }

    /// Flatten an object into a key->string map (string children only).
    pub fn str_map(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        if let Vdf::Obj(pairs) = self {
            for (k, v) in pairs {
                if let Vdf::Str(s) = v {
                    m.insert(k.clone(), s.clone());
                }
            }
        }
        m
    }
}

/// Parse a full VDF document. The top level is treated as an object body
/// (one or more `"key" value` pairs).
pub fn parse(input: &str) -> Result<Vdf> {
    let mut p = Parser {
        chars: input.as_bytes(),
        pos: 0,
    };
    let pairs = p.parse_body(true)?;
    Ok(Vdf::Obj(pairs))
}

struct Parser<'a> {
    chars: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> Error {
        Error::Vdf(format!("{msg} at byte {}", self.pos))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == b'/' && self.peek(1) == Some(b'/') {
                // line comment
                while self.pos < self.chars.len() && self.chars[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.chars.get(self.pos + off).copied()
    }

    /// Parse a sequence of `key value` pairs until EOF (top level) or `}`.
    fn parse_body(&mut self, top: bool) -> Result<Vec<(String, Vdf)>> {
        let mut pairs = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                if top {
                    return Ok(pairs);
                }
                return Err(self.err("unexpected EOF, expected '}'"));
            }
            if self.chars[self.pos] == b'}' {
                if top {
                    return Err(self.err("unexpected '}' at top level"));
                }
                self.pos += 1; // consume '}'
                return Ok(pairs);
            }
            let key = self.parse_string()?;
            self.skip_ws();
            match self.chars.get(self.pos) {
                Some(b'{') => {
                    self.pos += 1; // consume '{'
                    let child = self.parse_body(false)?;
                    pairs.push((key, Vdf::Obj(child)));
                }
                Some(b'"') => {
                    let val = self.parse_string()?;
                    pairs.push((key, Vdf::Str(val)));
                }
                _ => return Err(self.err("expected '{' or quoted value after key")),
            }
        }
    }

    fn parse_string(&mut self) -> Result<String> {
        self.skip_ws();
        if self.chars.get(self.pos) != Some(&b'"') {
            return Err(self.err("expected '\"'"));
        }
        self.pos += 1; // opening quote
        let mut buf: Vec<u8> = Vec::new();
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(String::from_utf8_lossy(&buf).into_owned());
                }
                b'\\' => {
                    self.pos += 1;
                    match self.chars.get(self.pos) {
                        Some(b'n') => buf.push(b'\n'),
                        Some(b't') => buf.push(b'\t'),
                        Some(b'\\') => buf.push(b'\\'),
                        Some(b'"') => buf.push(b'"'),
                        Some(&other) => buf.push(other),
                        None => return Err(self.err("EOF in escape")),
                    }
                    self.pos += 1;
                }
                _ => {
                    // Collect raw bytes; VDF strings are UTF-8, decoded at close quote.
                    buf.push(c);
                    self.pos += 1;
                }
            }
        }
        Err(self.err("unterminated string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested() {
        let doc = r#"
            "libraryfolders"
            {
                "0"
                {
                    "path"  "/games"
                    "apps"
                    {
                        "489830"  "123"
                    }
                }
            }
        "#;
        let v = parse(doc).unwrap();
        let lf = v.get("libraryfolders").unwrap();
        let path = lf.get("0").unwrap().get("path").unwrap().as_str().unwrap();
        assert_eq!(path, "/games");
        let apps = lf.get("0").unwrap().get("apps").unwrap();
        assert_eq!(apps.get("489830").unwrap().as_str(), Some("123"));
    }
}
