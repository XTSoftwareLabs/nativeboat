//! Private URL parsing operations for compiled Node library sources.
//! The parser and IDNA tables are pinned no_std libraries, not an engine.
use crate::prelude::*;
use crate::typed_array_builtin::{self, Element, View};
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value};
use url::{Position, Url};

fn string(realm: &mut Realm, args: &[Value], index: usize) -> Result<String, Thrown> {
    match args.get(index) {
        Some(Value::String(text)) => Ok(text.to_rust_lossy()),
        _ => Err(realm.throw(ErrorKind::Type, "URL binding requires a string")),
    }
}

fn invalid(realm: &mut Realm, input: Option<&str>, base: Option<&str>) -> Thrown {
    let error = crate::node_error::object(realm, ErrorKind::Type, "ERR_INVALID_URL", "Invalid URL");
    for (name, value) in [("input", input), ("base", base)] {
        if let Some(value) = value {
            error.define(Key::name(name), Property::data(Value::str(value)));
        }
    }
    Thrown(Value::Object(error))
}

fn parse(input: &str, base: Option<&str>) -> Option<Url> {
    let base = match base {
        Some(base) => Some(parse(base, None)?),
        None => None,
    };
    let mut url = match &base {
        Some(base) => base.join(input).ok()?,
        None => Url::parse(input).ok()?,
    };
    // rust-url clears a file host when a Windows drive starts the path. Node
    // retains that network host, including one inherited through a base URL.
    if url.scheme() == "file" && url.host_str().is_none() {
        let input: String = input
            .trim_matches(|c: char| c <= ' ')
            .chars()
            .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
            .map(|c| if c == '\\' { '/' } else { c })
            .collect();
        let input = if input
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
        {
            &input[5..]
        } else {
            input.as_str()
        };
        let hostname = if let Some(authority) = input.strip_prefix("//") {
            Some(authority.split(['/', '?', '#']).next().unwrap())
        } else {
            base.as_ref()
                .filter(|base| base.scheme() == "file")
                .and_then(Url::host_str)
        };
        if let Some(hostname) = hostname
            && !hostname.is_empty()
            && !hostname.eq_ignore_ascii_case("localhost")
        {
            // A drive-letter authority (file://C:/...) is a path, not a host.
            if url::Host::parse(hostname).is_ok() {
                host(&mut url, hostname, true);
            }
        }
    }
    Some(url)
}

fn components(url: &Url) -> [u32; 9] {
    let offset = |position| url[..position].len() as u32;
    let host_start = offset(Position::BeforeHost)
        - u32::from(!url.username().is_empty() || url.password().is_some());
    [
        url.scheme().len() as u32 + 1,
        offset(Position::AfterUsername),
        host_start,
        offset(Position::AfterHost),
        url.port().map_or(u32::MAX, u32::from),
        offset(Position::BeforePath),
        url.query()
            .map_or(u32::MAX, |_| offset(Position::BeforeQuery) - 1),
        url.fragment()
            .map_or(u32::MAX, |_| offset(Position::BeforeFragment) - 1),
        match url.scheme() {
            "http" => 0,
            "https" => 2,
            "ws" => 3,
            "ftp" => 4,
            "wss" => 5,
            "file" => 6,
            _ => 1,
        },
    ]
}

fn publish(view: &View, url: &Url) -> Value {
    for (index, value) in components(url).into_iter().enumerate() {
        view.set(index as f64, &Value::Number(f64::from(value)));
    }
    Value::str(url.as_str())
}

// A bad port does not undo a changed host; the hostname setter rejects ports.
fn host(url: &mut Url, value: &str, hostname_only: bool) -> bool {
    if url.cannot_be_a_base() {
        return false;
    }
    let value: String = value
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let special = matches!(
        url.scheme(),
        "http" | "https" | "ws" | "wss" | "ftp" | "file"
    );
    let value = value
        .split(|c| matches!(c, '/' | '?' | '#') || (special && c == '\\'))
        .next()
        .unwrap();
    let (hostname, port) = if value.starts_with('[') {
        let Some(end) = value.find(']') else {
            return false;
        };
        if !value[end + 1..].is_empty() && !value[end + 1..].starts_with(':') {
            return false;
        }
        (&value[..=end], value[end + 1..].strip_prefix(':'))
    } else {
        value
            .split_once(':')
            .map_or((value, None), |(host, port)| (host, Some(port)))
    };
    if (hostname_only || url.scheme() == "file") && port.is_some() {
        return false;
    }
    // An empty host is forbidden while credentials or a port are present.
    // rust-url otherwise admits a record whose port colon is mistaken for a
    // password delimiter by its accessors. Reject it before mutating the URL.
    if hostname.is_empty()
        && (!url.username().is_empty() || url.password().is_some() || url.port().is_some())
    {
        return false;
    }
    if url.scheme() == "file" && hostname.is_empty() {
        return url.set_host(None).is_ok();
    }
    if url.set_host(Some(hostname)).is_err() {
        return false;
    }
    if url.scheme() == "file" && url.host_str() == Some("localhost") {
        url.set_host(None).unwrap();
    }
    if !hostname_only
        && let Some(port) = port
        && !port.is_empty()
    {
        set_port(url, port);
    }
    true
}

fn set_port(url: &mut Url, value: &str) -> bool {
    if url.cannot_be_a_base() || url.host_str().is_none_or(str::is_empty) || url.scheme() == "file"
    {
        return false;
    }
    if value.is_empty() {
        return url.set_port(None).is_ok();
    }
    let value: String = value
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    value[..end]
        .parse::<u16>()
        .is_ok_and(|port| url.set_port(Some(port)).is_ok())
}

fn update(url: &mut Url, action: u32, value: &str) -> bool {
    match action {
        0 => {
            let value: String = value
                .chars()
                .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
                .collect();
            let scheme = value.split(':').next().unwrap().to_ascii_lowercase();
            let special = |s: &str| matches!(s, "http" | "https" | "ws" | "wss" | "ftp" | "file");
            // Ada reports a successful no-op when a special URL is assigned a
            // valid non-special protocol. File URLs with no host cannot change.
            if url.scheme() == "file" && url.host_str().is_none_or(str::is_empty) {
                return false;
            }
            if scheme == "file" && special(url.scheme()) {
                if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
                    return false;
                }
                // rust-url's setter rejects every authority-to-file transition;
                // the URL standard permits one without credentials or a port.
                // A parsed special URL with a valid host and neither credentials
                // nor port remains valid when its scheme is replaced with file.
                *url = parse(&format!("file{}", &url[Position::AfterScheme..]), None).unwrap();
                return true;
            }
            if special(url.scheme())
                && !special(&scheme)
                && scheme
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphabetic)
                && scheme
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
            {
                return true;
            }
            url.set_scheme(&scheme).is_ok()
        }
        1 => host(url, value, false),
        2 => host(url, value, true),
        3 => set_port(url, value),
        4 => url.set_username(value).is_ok(),
        5 => url.set_password(Some(value)).is_ok(),
        6 if !url.cannot_be_a_base() => {
            url.set_path(value);
            true
        }
        7 => {
            url.set_query((!value.is_empty()).then(|| value.strip_prefix('?').unwrap_or(value)));
            true
        }
        8 => {
            url.set_fragment((!value.is_empty()).then(|| value.strip_prefix('#').unwrap_or(value)));
            true
        }
        9 => match parse(value, None) {
            Some(parsed) => {
                *url = parsed;
                true
            }
            None => false,
        },
        _ => false,
    }
}

fn domain(input: &str, unicode: bool) -> String {
    if input.is_empty() {
        return String::new();
    }
    let mut url = Url::parse("ws://x").unwrap();
    if !host(&mut url, input, true) {
        return String::new();
    }
    let host = url.host_str().unwrap();
    if unicode {
        idna::domain_to_unicode(host).0
    } else {
        host.to_owned()
    }
}

// Ada's IDNA conversion permits ASCII-only decoded A-labels. The URL parser's
// stricter host validation is separate from this private encoding operation.
fn ascii_idna(input: &str) -> String {
    if !input.is_ascii() {
        return idna::domain_to_ascii(input).unwrap_or_default();
    }
    let input = input.to_ascii_lowercase();
    for label in input.split('.') {
        if let Some(encoded) = label.strip_prefix("xn--") {
            let Some(decoded) = idna::punycode::decode_to_string(encoded) else {
                return String::new();
            };
            let (normalized, status) = idna::domain_to_unicode(&decoded);
            if decoded.is_empty() || status.is_err() || normalized != decoded {
                return String::new();
            }
        }
    }
    input
}

/// The IDNA operation used by Node's legacy URL parser.
pub(crate) fn encoding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let function = realm.function("toASCII", 1, false, |realm, _, args| {
        Ok(Value::str(&ascii_idna(&string(realm, args, 0)?)))
    });
    exports.define(
        Key::name("toASCII"),
        Property::data(Value::Object(function)),
    );
    crate::text_encoding_builtin::install_binding(realm, &exports);
    exports
}

/// Private native URL operations consumed by compiled Node library sources.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let array = typed_array_builtin::new_typed_array(realm, Element::Uint32, 9);
    let view = typed_array_builtin::view_of(&Value::Object(array.clone())).unwrap();
    exports.define(
        Key::name("urlComponents"),
        Property::data(Value::Object(array)),
    );
    let parse_view = view.clone();
    let function = realm.function("parse", 1, false, move |realm, _, args| {
        let input = string(realm, args, 0)?;
        let base = match args.get(1) {
            Some(Value::String(base)) => Some(base.to_rust_lossy()),
            _ => None,
        };
        match parse(&input, base.as_deref()) {
            Some(url) => Ok(publish(&parse_view, &url)),
            None if matches!(args.get(2), Some(Value::Boolean(true))) => {
                Err(invalid(realm, Some(&input), base.as_deref()))
            }
            None => Ok(Value::Undefined),
        }
    });
    exports.define(Key::name("parse"), Property::data(Value::Object(function)));
    let function = realm.function("canParse", 1, false, |realm, _, args| {
        let input = string(realm, args, 0)?;
        let base = match args.get(1) {
            Some(Value::String(base)) => Some(base.to_rust_lossy()),
            _ => None,
        };
        Ok(Value::Boolean(parse(&input, base.as_deref()).is_some()))
    });
    exports.define(
        Key::name("canParse"),
        Property::data(Value::Object(function)),
    );
    let function = realm.function("getOrigin", 1, false, |realm, _, args| {
        let input = string(realm, args, 0)?;
        let url = parse(&input, None).ok_or_else(|| invalid(realm, None, None))?;
        Ok(Value::str(&url.origin().ascii_serialization()))
    });
    exports.define(
        Key::name("getOrigin"),
        Property::data(Value::Object(function)),
    );
    let update_view = view.clone();
    let function = realm.function("update", 3, false, move |realm, _, args| {
        let input = string(realm, args, 0)?;
        let value = string(realm, args, 2)?;
        let action = crate::ops::to_number(realm, args.get(1).unwrap_or(&Value::Undefined))?;
        let mut url = parse(&input, None).ok_or_else(|| invalid(realm, None, None))?;
        if update(&mut url, crate::ops::wrap32(action), &value) {
            Ok(publish(&update_view, &url))
        } else {
            Ok(Value::Boolean(false))
        }
    });
    exports.define(Key::name("update"), Property::data(Value::Object(function)));
    for (name, unicode) in [("domainToASCII", false), ("domainToUnicode", true)] {
        let function = realm.function(name, 1, false, move |realm, _, args| {
            Ok(Value::str(&domain(&string(realm, args, 0)?, unicode)))
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let function = realm.function("format", 5, false, |realm, _, args| {
        let input = string(realm, args, 0)?;
        let mut url = parse(&input, None).ok_or_else(|| invalid(realm, None, None))?;
        let flag = |index| matches!(args.get(index), Some(Value::Boolean(true)));
        if !flag(1) {
            url.set_fragment(None);
        }
        if !flag(3) {
            url.set_query(None);
        }
        if !flag(4) {
            let _ = url.set_username("");
            let _ = url.set_password(None);
        }
        let href = if flag(2) && url.host_str().is_some() {
            format!(
                "{}{}{}",
                &url[..Position::BeforeHost],
                idna::domain_to_unicode(url.host_str().unwrap()).0,
                &url[Position::AfterHost..]
            )
        } else {
            url.to_string()
        };
        Ok(Value::str(&href))
    });
    exports.define(Key::name("format"), Property::data(Value::Object(function)));
    let function = realm.function("pathToFileURL", 2, false, move |realm, _, args| {
        let input = string(realm, args, 0)?;
        if input.is_empty() {
            return Err(invalid(realm, Some(&input), None));
        }
        let windows = matches!(args.get(1), Some(Value::Boolean(true)));
        let mut encoded = String::from("file://");
        for c in input.chars() {
            if windows && c == '\\' {
                encoded.push('/');
            } else if matches!(
                c,
                '\0' | '\t'
                    | '\n'
                    | '\r'
                    | ' '
                    | '"'
                    | '#'
                    | '%'
                    | '?'
                    | '['
                    | '\\'
                    | ']'
                    | '^'
                    | '|'
                    | '~'
            ) {
                encoded.push_str(&format!("%{:02X}", c as u32));
            } else {
                encoded.push(c);
            }
        }
        let mut url = Url::parse(&encoded).map_err(|_| invalid(realm, Some(&input), None))?;
        if windows
            && let Some(Value::String(hostname)) = args.get(2)
            && !host(&mut url, &hostname.to_rust_lossy(), true)
        {
            return Err(invalid(realm, Some(&input), None));
        }
        Ok(publish(&view, &url))
    });
    exports.define(
        Key::name("pathToFileURL"),
        Property::data(Value::Object(function)),
    );
    exports
}

#[cfg(test)]
mod tests;
