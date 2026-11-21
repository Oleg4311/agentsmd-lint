//! Небольшой разборщик JSON без зависимостей: нужен для package.json и конфигурации. Строгий по RFC 8259.
//! Сверяется с serde_json на случайных документах (tests/json_diff.rs).

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Как у serde_json: не глубже 127 уровней вложенности.
const MAX_DEPTH: usize = 127;

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(items) => items.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Obj(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }
}

pub fn parse(src: &str) -> Result<Json, String> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut p = Parser {
        b: src.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(p.err("лишние данные после значения"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> String {
        format!("{msg} (позиция {})", self.i)
    }

    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.b[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("слишком глубокая вложенность"));
        }
        match self.b.get(self.i) {
            None => Err(self.err("неожиданный конец")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("неожиданный символ")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Json::Obj(items));
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return Err(self.err("ожидался ключ-строка"));
            }
            let k = self.string()?;
            self.ws();
            if self.b.get(self.i) != Some(&b':') {
                return Err(self.err("ожидалось ':'"));
            }
            self.i += 1;
            self.ws();
            let v = self.value(depth + 1)?;
            items.push((k, v));
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(items));
                }
                _ => return Err(self.err("ожидалось ',' или '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(self.err("ожидалось ',' или ']'")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let end = self.i + 4;
        if end > self.b.len() {
            return Err(self.err("обрыв в \\u-последовательности"));
        }
        let mut v = 0u32;
        for &c in &self.b[self.i..end] {
            let d = (c as char)
                .to_digit(16)
                .ok_or_else(|| self.err("неверная \\u-последовательность"))?;
            v = v * 16 + d;
        }
        self.i = end;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // открывающая кавычка
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.i) else {
                return Err(self.err("строка не закрыта"));
            };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else {
                        return Err(self.err("обрыв после '\\'"));
                    };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            if (0xD800..0xDC00).contains(&hi) {
                                if !self.eat("\\u") {
                                    return Err(self.err("одиночный суррогат"));
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("неверная пара суррогатов"));
                                }
                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                    .ok_or_else(|| self.err("неверный символ"))?
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("одиночный суррогат"));
                            } else {
                                char::from_u32(hi).ok_or_else(|| self.err("неверный символ"))?
                            }
                        }
                        _ => return Err(self.err("неверное экранирование")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0..=0x1f => return Err(self.err("управляющий символ в строке")),
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("неверный UTF-8"))
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return Err(self.err("неверное число")),
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                return Err(self.err("после '.' нужна цифра"));
            }
            while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                return Err(self.err("в порядке числа нужна цифра"));
            }
            while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        std::str::from_utf8(&self.b[start..self.i])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(Json::Num)
            .ok_or_else(|| self.err("неверное число"))
    }
}

/// Экранирует строку для вывода в JSON.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_json_like_documents() {
        let j = parse(r#"{"name":"x","scripts":{"test":"node --test","build":"tsc"},"n":[1,2.5e1,-0,true,null]}"#).unwrap();
        assert_eq!(j.get("name").and_then(Json::as_str), Some("x"));
        let s = j.get("scripts").and_then(Json::as_obj).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(j.get("n").and_then(Json::as_arr).unwrap().len(), 5);
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":}",
            "{'a':1}",
            "01",
            "1.",
            "\"\\x\"",
            "\"a\nb\"",
            "[1] 2",
            "nul",
            "{\"a\":1,}",
            "\"\\ud800\"",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} должен отвергаться");
        }
    }

    #[test]
    fn handles_unicode_escapes_and_bom() {
        assert_eq!(
            parse("\"\\u00e9\\ud83d\\ude00\"").unwrap(),
            Json::Str("é😀".into())
        );
        assert_eq!(
            parse("\u{feff}{\"a\":1}").unwrap().get("a"),
            Some(&Json::Num(1.0))
        );
    }

    #[test]
    fn depth_is_limited_without_overflowing_the_stack() {
        let deep = "[".repeat(100_000);
        assert!(parse(&deep).is_err());
        let ok = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn last_duplicate_key_wins() {
        assert_eq!(
            parse(r#"{"a":1,"a":2}"#).unwrap().get("a"),
            Some(&Json::Num(2.0))
        );
    }

    #[test]
    fn escape_roundtrips() {
        let s = "a\"b\\c\nd\u{1}é";
        let j = parse(&format!("\"{}\"", escape(s))).unwrap();
        assert_eq!(j, Json::Str(s.into()));
    }
}
