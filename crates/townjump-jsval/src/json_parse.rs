// JS: V8 `JsonParser` (`JSON.parse`): ECMA-404 grammar, order of checks and V8 error texts.
// Related: json_error.rs (messages), json.rs (facade). Works on UTF-16, like V8.
use crate::{
    error::JsError,
    json_error::{At, positioned, unexpected},
    value::JsValue,
};

type Res<T> = Result<T, JsError>;

pub fn parse(text: &str) -> Res<JsValue> {
    let src: Vec<u16> = text.encode_utf16().collect();
    let mut p = Parser { src: &src, pos: 0 };
    let value = p.value()?;
    p.skip_ws();
    match p.peek() {
        None => Ok(value),
        Some(_) => Err(p.error("Unexpected non-whitespace character after JSON")),
    }
}

struct Parser<'a> {
    src: &'a [u16],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.src.get(self.pos).copied()
    }

    fn at(&self) -> At<'_> {
        At {
            src: self.src,
            pos: self.pos,
        }
    }

    fn error(&self, what: &str) -> JsError {
        positioned(&self.at(), what)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(0x20 | 0x09 | 0x0a | 0x0d)) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, unit: char) -> bool {
        let hit = self.peek() == Some(unit as u16);
        self.pos += usize::from(hit);
        hit
    }

    fn value(&mut self) -> Res<JsValue> {
        self.skip_ws();
        match self.peek().and_then(|u| char::from_u32(u32::from(u))) {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => self.string().map(JsValue::String),
            Some('t') => self.literal("true", JsValue::Bool(true)),
            Some('f') => self.literal("false", JsValue::Bool(false)),
            Some('n') => self.literal("null", JsValue::Null),
            Some('-' | '0'..='9') => self.number(),
            _ => Err(unexpected(&self.at())),
        }
    }

    fn literal(&mut self, word: &str, value: JsValue) -> Res<JsValue> {
        for expected in word.encode_utf16() {
            if self.peek() != Some(expected) {
                return Err(unexpected(&self.at()));
            }
            self.pos += 1;
        }
        Ok(value)
    }

    fn array(&mut self) -> Res<JsValue> {
        self.pos += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.eat(']') {
            return Ok(JsValue::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            if self.eat(']') {
                return Ok(JsValue::Array(items));
            }
            if !self.eat(',') {
                return Err(self.error("Expected ',' or ']' after array element"));
            }
        }
    }

    fn object(&mut self) -> Res<JsValue> {
        self.pos += 1;
        self.skip_ws();
        let mut pairs = Vec::new();
        if self.eat('}') {
            return Ok(JsValue::Object(pairs));
        }
        let first = "Expected property name or '}'";
        self.member(&mut pairs, first)?;
        loop {
            self.skip_ws();
            if self.eat('}') {
                return Ok(JsValue::Object(pairs));
            }
            if !self.eat(',') {
                return Err(self.error("Expected ',' or '}' after property value"));
            }
            self.member(&mut pairs, "Expected double-quoted property name")?;
        }
    }

    /// `"key": value`; a repeated key keeps the first position and the last value.
    fn member(&mut self, pairs: &mut Vec<(String, JsValue)>, missing_key: &str) -> Res<()> {
        self.skip_ws();
        if self.peek() != Some(0x22) {
            return Err(self.error(missing_key));
        }
        let key = self.string()?;
        self.skip_ws();
        if !self.eat(':') {
            return Err(self.error("Expected ':' after property name"));
        }
        let value = self.value()?;
        match pairs.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => pairs.push((key, value)),
        }
        Ok(())
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(0x30..=0x39)) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Res<JsValue> {
        let start = self.pos;
        if self.eat('-') && !matches!(self.peek(), Some(0x30..=0x39)) {
            return Err(self.error("No number after minus sign"));
        }
        if self.eat('0') {
            if matches!(self.peek(), Some(0x30..=0x39)) {
                return Err(self.error("Unexpected number"));
            }
        } else {
            self.digits();
        }
        self.fraction()?;
        self.exponent()?;
        let text = String::from_utf16_lossy(&self.src[start..self.pos]);
        Ok(JsValue::Number(text.parse().unwrap_or(f64::NAN)))
    }

    fn fraction(&mut self) -> Res<()> {
        if self.eat('.') && self.digits() == 0 {
            return Err(self.error("Unterminated fractional number"));
        }
        Ok(())
    }

    fn exponent(&mut self) -> Res<()> {
        if !(self.eat('e') || self.eat('E')) {
            return Ok(());
        }
        let _ = self.eat('+') || self.eat('-');
        if self.digits() == 0 {
            return Err(self.error("Exponent part is missing a number"));
        }
        Ok(())
    }

    fn string(&mut self) -> Res<String> {
        self.pos += 1;
        let mut units: Vec<u16> = Vec::new();
        loop {
            let Some(unit) = self.peek() else {
                return Err(self.error("Unterminated string"));
            };
            self.pos += 1;
            match unit {
                0x22 => return Ok(String::from_utf16_lossy(&units)),
                0x5c => units.push(self.escape()?),
                0..=0x1f => {
                    self.pos -= 1;
                    return Err(self.error("Bad control character in string literal"));
                }
                u => units.push(u),
            }
        }
    }

    fn escape(&mut self) -> Res<u16> {
        let Some(unit) = self.peek() else {
            return Err(self.error("Unterminated string"));
        };
        let simple = match char::from_u32(u32::from(unit)) {
            Some('u') => return self.unicode(),
            Some('"') => 0x22,
            Some('\\') => 0x5c,
            Some('/') => 0x2f,
            Some('b') => 0x08,
            Some('f') => 0x0c,
            Some('n') => 0x0a,
            Some('r') => 0x0d,
            Some('t') => 0x09,
            _ => return Err(self.error("Bad escaped character")),
        };
        self.pos += 1;
        Ok(simple)
    }

    fn unicode(&mut self) -> Res<u16> {
        self.pos += 1;
        let mut code = 0u16;
        for _ in 0..4 {
            let digit = self
                .peek()
                .and_then(|u| char::from_u32(u32::from(u))?.to_digit(16));
            let Some(d) = digit else {
                return Err(self.error("Bad Unicode escape"));
            };
            code = code * 16 + d as u16;
            self.pos += 1;
        }
        Ok(code)
    }
}
