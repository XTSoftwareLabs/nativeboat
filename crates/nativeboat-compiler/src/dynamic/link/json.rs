//! JSON is parsed during compilation and emitted as native data construction.
//! No parser, original JSON payload or executable source ships in the app.
use super::*;

pub(super) fn scan(path: &Path, text: String) -> Result<Record, Vec<Diagnostic>> {
    let source = Source { path, text: &text };
    let mut parser = Data {
        source: &source,
        position: if text.starts_with('\u{feff}') { 3 } else { 0 },
    };
    let value = parser.value(0).map_err(|error| vec![error])?;
    parser.space();
    if parser.position != text.len() {
        return Err(vec![parser.error("unexpected trailing text")]);
    }
    Ok(Record {
        data: Some(value),
        source: text,
        locals: vec![("default".into(), Local::Default)],
        placeholder: false,
        ..placeholder(path)
    })
}

/// A bounded compile-time JSON decoder, retaining UTF-16 and binary64 values.
struct Data<'s> {
    source: &'s Source<'s>,
    position: usize,
}

impl Data<'_> {
    fn byte(&self) -> Option<u8> {
        self.source.text.as_bytes().get(self.position).copied()
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.byte() == Some(byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn space(&mut self) {
        while matches!(self.byte(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.position += 1;
        }
    }

    fn error(&self, reason: &str) -> Diagnostic {
        let length = self.source.text[self.position..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        self.source.error(
            Span::new(self.position as u32, (self.position + length) as u32),
            &format!("invalid JSON: {reason}"),
        )
    }

    fn expect(&mut self, byte: u8) -> Result<(), Diagnostic> {
        self.space();
        if self.take(byte) {
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", char::from(byte))))
        }
    }

    fn value(&mut self, depth: usize) -> Result<String, Diagnostic> {
        self.space();
        if depth >= 64 && matches!(self.byte(), Some(b'[' | b'{')) {
            let mut error = self.error("nesting limit");
            error.message = "JSON nesting beyond 64 is not compiled yet".into();
            return Err(error);
        }
        match self.byte() {
            Some(b'"') => Ok(emit::units_value(&self.string()?)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b't') => self.literal("true", "Value::Boolean(true)"),
            Some(b'f') => self.literal("false", "Value::Boolean(false)"),
            Some(b'n') => self.literal("null", "Value::Null"),
            Some(b'[') => self.array(depth + 1),
            Some(b'{') => self.object(depth + 1),
            _ => Err(self.error("expected a data value")),
        }
    }

    fn literal(&mut self, text: &str, value: &str) -> Result<String, Diagnostic> {
        if self.source.text[self.position..].starts_with(text) {
            self.position += text.len();
            Ok(value.into())
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn digits(&mut self) -> bool {
        let start = self.position;
        while matches!(self.byte(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        self.position != start
    }

    fn number(&mut self) -> Result<String, Diagnostic> {
        let start = self.position;
        self.take(b'-');
        match self.byte() {
            Some(b'0') => {
                self.position += 1;
            }
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.error("expected an integer digit")),
        }
        if self.take(b'.') && !self.digits() {
            return Err(self.error("expected a fractional digit"));
        }
        if self.take(b'e') || self.take(b'E') {
            if !self.take(b'+') {
                self.take(b'-');
            }
            if !self.digits() {
                return Err(self.error("expected an exponent digit"));
            }
        }
        let value = self.source.text[start..self.position]
            .parse::<f64>()
            .expect("validated JSON number");
        Ok(emit::number(value))
    }

    fn string(&mut self) -> Result<Vec<u16>, Diagnostic> {
        self.expect(b'"')?;
        let mut units = Vec::new();
        loop {
            match self.byte() {
                Some(b'"') => {
                    self.position += 1;
                    return Ok(units);
                }
                Some(b'\\') => {
                    self.position += 1;
                    let unit = match self.byte() {
                        Some(b'"' | b'/' | b'\\') => u16::from(self.byte().unwrap()),
                        Some(b'b') => 8,
                        Some(b'f') => 12,
                        Some(b'n') => 10,
                        Some(b'r') => 13,
                        Some(b't') => 9,
                        Some(b'u') => {
                            self.position += 1;
                            let mut unit = 0;
                            for _ in 0..4 {
                                let Some(digit) =
                                    self.byte().and_then(|byte| char::from(byte).to_digit(16))
                                else {
                                    return Err(
                                        self.error("expected four hexadecimal escape digits")
                                    );
                                };
                                unit = unit * 16 + digit as u16;
                                self.position += 1;
                            }
                            units.push(unit);
                            continue;
                        }
                        _ => return Err(self.error("invalid string escape")),
                    };
                    self.position += 1;
                    units.push(unit);
                }
                Some(0..=31) => return Err(self.error("unescaped control character")),
                Some(_) => {
                    let character = self.source.text[self.position..].chars().next().unwrap();
                    let mut buffer = [0; 2];
                    units.extend_from_slice(character.encode_utf16(&mut buffer));
                    self.position += character.len_utf8();
                }
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<String, Diagnostic> {
        self.position += 1;
        self.space();
        let mut values = Vec::new();
        if !self.take(b']') {
            loop {
                values.push(self.value(depth)?);
                self.space();
                if self.take(b']') {
                    break;
                }
                self.expect(b',')?;
            }
        }
        Ok(format!(
            "Value::Object(realm.array(vec![{}]))",
            values.join(",")
        ))
    }

    fn object(&mut self, depth: usize) -> Result<String, Diagnostic> {
        self.position += 1;
        self.space();
        let mut code = String::from("{ let data = realm.object(); ");
        if !self.take(b'}') {
            loop {
                let key = self.string()?;
                self.expect(b':')?;
                let value = self.value(depth)?;
                let units = emit::units_value(&key);
                let _ = write!(
                    code,
                    "{{ let key = {units}; let Value::String(key) = key else {{ unreachable!() }}; ops::define_data(&data, Key::from_js(&key), {value}); }} "
                );
                self.space();
                if self.take(b'}') {
                    break;
                }
                self.expect(b',')?;
            }
        }
        code.push_str("Value::Object(data) }");
        Ok(code)
    }
}

pub(super) fn factory(index: usize, cell: usize, data: &str) -> String {
    format!(
        "\nfn m{index}(realm: &mut Realm, cells: &::alloc::rc::Rc<Vec<binding::Binding>>, _cache: &::alloc::rc::Rc<Vec<::core::cell::RefCell<Option<Object>>>>) -> Result<(), Thrown> {{\n\
        let factory = realm.function(\"\", 5, false, |realm, _, args| {{\n\
        let value = {data};\n\
        ops::set(realm, &args[2], &Key::name(\"exports\"), value)?;\n\
        Ok(Value::Undefined)\n}});\n\
        binding::initialize(&cells[{cell}], Value::Object(factory));\nOk(())\n}}\n"
    )
}
