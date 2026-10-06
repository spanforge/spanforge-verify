//! Strict JSON with duplicate key rejection and bounded, exact decimal values.
//! Numbers normalize coefficient/exponent without floating point or exponent expansion.
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    String(String),
    Number(Decimal),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    negative: bool,
    coefficient: String,
    exponent: i32,
}
impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        write!(f, "{}e{}", self.coefficient, self.exponent)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Limit(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "Invalid JSON: {message}"),
            Self::Limit(message) => write!(f, "JSON input limit: {message}"),
        }
    }
}
impl std::error::Error for Error {}

pub fn parse(bytes: &[u8]) -> Result<Value, Error> {
    std::str::from_utf8(bytes).map_err(|_| Error::Invalid("UTF-8"))?;
    let mut parser = Parser { bytes, position: 0 };
    let value = parser.value(0)?;
    parser.whitespace();
    if parser.position != bytes.len() {
        return Err(Error::Invalid("trailing data"));
    }
    Ok(value)
}
struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl Parser<'_> {
    fn whitespace(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.position += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn literal(&mut self, literal: &[u8], value: Value) -> Result<Value, Error> {
        if self.bytes[self.position..].starts_with(literal) {
            self.position += literal.len();
            Ok(value)
        } else {
            Err(Error::Invalid("literal"))
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value, Error> {
        self.whitespace();
        match self.bytes.get(self.position).copied() {
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[' | b'{') if depth >= 128 => Err(Error::Limit("nesting exceeds 128")),
            Some(b'[') => {
                self.position += 1;
                let mut values = Vec::new();
                self.whitespace();
                if self.take(b']') {
                    return Ok(Value::Array(values));
                }
                loop {
                    values.push(self.value(depth + 1)?);
                    self.whitespace();
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(Error::Invalid("array separator"));
                    }
                }
                Ok(Value::Array(values))
            }
            Some(b'{') => {
                self.position += 1;
                let mut values = BTreeMap::new();
                self.whitespace();
                if self.take(b'}') {
                    return Ok(Value::Object(values));
                }
                loop {
                    self.whitespace();
                    let key = self.string()?;
                    self.whitespace();
                    if !self.take(b':') {
                        return Err(Error::Invalid("object colon"));
                    }
                    let value = self.value(depth + 1)?;
                    if values.insert(key, value).is_some() {
                        return Err(Error::Invalid("duplicate object key"));
                    }
                    self.whitespace();
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(Error::Invalid("object separator"));
                    }
                }
                Ok(Value::Object(values))
            }
            Some(b'-' | b'0'..=b'9') => self.number().map(Value::Number),
            _ => Err(Error::Invalid("expected value")),
        }
    }
    fn string(&mut self) -> Result<String, Error> {
        let start = self.position;
        if !self.take(b'"') {
            return Err(Error::Invalid("expected string"));
        }
        while let Some(byte) = self.bytes.get(self.position).copied() {
            self.position += 1;
            if byte == b'"' {
                return serde_json::from_slice(&self.bytes[start..self.position])
                    .map_err(|_| Error::Invalid("string escape or Unicode"));
            }
            if byte == b'\\' {
                if self.position == self.bytes.len() {
                    break;
                }
                self.position += 1;
            } else if byte < 0x20 {
                return Err(Error::Invalid("unescaped control character"));
            }
        }
        Err(Error::Invalid("unterminated string"))
    }
    fn digits(&mut self) -> usize {
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
        }
        self.position - start
    }
    fn number(&mut self) -> Result<Decimal, Error> {
        let start = self.position;
        let negative = self.take(b'-');
        let integer = self.position;
        if self.take(b'0') {
            if self
                .bytes
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                return Err(Error::Invalid("leading zero"));
            }
        } else if self.digits() == 0 {
            return Err(Error::Invalid("number integer"));
        }
        let integer_end = self.position;
        let mut fraction = None;
        if self.take(b'.') {
            let begin = self.position;
            if self.digits() == 0 {
                return Err(Error::Invalid("number fraction"));
            }
            fraction = Some((begin, self.position));
        }
        let mut exponent = 0i32;
        if self.take(b'e') || self.take(b'E') {
            let minus = self.take(b'-');
            if !minus {
                self.take(b'+');
            }
            let begin = self.position;
            if self.digits() == 0 {
                return Err(Error::Invalid("number exponent"));
            }
            for digit in &self.bytes[begin..self.position] {
                exponent = exponent
                    .saturating_mul(10)
                    .saturating_add((digit - b'0') as i32);
                if exponent > 10_000 {
                    return Err(Error::Limit("decimal exponent exceeds 10000"));
                }
            }
            if minus {
                exponent = -exponent;
            }
        }
        if self.position - start > 1024 {
            return Err(Error::Limit("number exceeds 1024 characters"));
        }
        let mut coefficient = String::from_utf8(self.bytes[integer..integer_end].to_vec()).unwrap();
        if let Some((begin, end)) = fraction {
            coefficient.push_str(std::str::from_utf8(&self.bytes[begin..end]).unwrap());
            exponent -= (end - begin) as i32;
        }
        let coefficient = coefficient.trim_start_matches('0');
        if coefficient.is_empty() {
            return Ok(Decimal {
                negative: false,
                coefficient: "0".into(),
                exponent: 0,
            });
        }
        let trimmed = coefficient.trim_end_matches('0');
        exponent += (coefficient.len() - trimmed.len()) as i32;
        Ok(Decimal {
            negative,
            coefficient: trimmed.into(),
            exponent,
        })
    }
}
pub fn valid_pointer(pointer: &str) -> bool {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}
impl Value {
    pub fn pointer(&self, pointer: &str) -> Option<&Self> {
        if !valid_pointer(pointer) {
            return None;
        }
        if pointer.is_empty() {
            return Some(self);
        }
        let mut current = self;
        for token in pointer[1..].split('/') {
            let token = token.replace("~1", "/").replace("~0", "~");
            current = match current {
                Self::Object(values) => values.get(&token)?,
                Self::Array(values) => {
                    if token.is_empty()
                        || (token.len() > 1 && token.starts_with('0'))
                        || !token.bytes().all(|b| b.is_ascii_digit())
                    {
                        return None;
                    }
                    values.get(token.parse::<usize>().ok()?)?
                }
                _ => return None,
            };
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_decimals() {
        for (a, b) in [
            ("1", "1.0"),
            ("1", "1e0"),
            ("1000", "1e3"),
            ("-0", "0.000e10000"),
            ("0.00120", "12e-4"),
        ] {
            assert_eq!(parse(a.as_bytes()), parse(b.as_bytes()));
        }
        assert_ne!(parse(b"9007199254740992"), parse(b"9007199254740993"));
        assert_ne!(parse(b"1e10000"), parse(b"1e9999"));
    }
    #[test]
    fn strict_syntax_and_keys() {
        for bad in [
            r#"{"a":1,"\u0061":2}"#,
            "[1,]",
            "01",
            "1.",
            "1e",
            "NaN",
            "true false",
            r#""\uD800""#,
        ] {
            assert!(parse(bad.as_bytes()).is_err(), "{bad}");
        }
        assert_eq!(parse(br#"{"a":1,"b":2}"#), parse(br#"{"b":2,"a":1}"#));
        assert_ne!(parse(b"[1,2]"), parse(b"[2,1]"));
    }
    #[test]
    fn parser_boundaries() {
        assert!(parse(format!("{}0{}", "[".repeat(128), "]".repeat(128)).as_bytes()).is_ok());
        assert!(matches!(
            parse(format!("{}0{}", "[".repeat(129), "]".repeat(129)).as_bytes()),
            Err(Error::Limit(_))
        ));
        assert!(parse("1".repeat(1024).as_bytes()).is_ok());
        assert!(matches!(
            parse("1".repeat(1025).as_bytes()),
            Err(Error::Limit(_))
        ));
        assert!(parse(b"1e-10000").is_ok());
        assert!(matches!(parse(b"1e10001"), Err(Error::Limit(_))));
    }
    #[test]
    fn pointers_distinguish_missing_and_null() {
        let value = parse(br#"{"a/b":{"~key":null},"list":[1]}"#).unwrap();
        assert_eq!(value.pointer("/a~1b/~0key"), Some(&Value::Null));
        assert_eq!(value.pointer("/missing"), None);
        assert!(value.pointer("/list/0").is_some());
        assert!(value.pointer("/list/00").is_none());
        assert!(value.pointer("").is_some());
        assert!(!valid_pointer("/~2"));
    }
}
