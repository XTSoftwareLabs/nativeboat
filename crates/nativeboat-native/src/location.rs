//! A renderer page's `location`: the read-only parts of the HTML `Location`
//! interface for the URL its window loaded, such as the query a page reads
//! its start-up arguments from. A compiled build runs only the pages it
//! compiled, so navigation (`assign`, `replace`, `reload`) is not provided.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Key, Property, Realm, Value};
use url::Url;

/// The URL a window loaded: a URL, or a file path as `loadFile` gives one.
/// A drive letter parses as a one-letter scheme, so that is a path too.
fn parse(target: &str) -> Url {
    if let Ok(url) = Url::parse(target)
        && url.scheme().len() > 1
    {
        return url;
    }
    let path = target.replace('\\', "/");
    let path = path.trim_start_matches('/');
    Url::parse(&format!("file:///{path}")).expect("a file URL with any path parses")
}

/// A component with its prefix, or empty when the URL has none.
fn part(prefix: &str, value: Option<&str>) -> String {
    match value {
        Some(value) if !value.is_empty() => format!("{prefix}{value}"),
        _ => String::new(),
    }
}

impl Realm {
    /// Gives a renderer page `location` and `document.location` for the URL
    /// its window loaded.
    pub fn set_renderer_location(&mut self, target: &str) {
        let url = parse(target);
        let origin = match url.scheme() {
            // Chromium reports file pages' origin by scheme alone.
            "file" => "file://".to_owned(),
            _ => url.origin().ascii_serialization(),
        };
        let host = match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_owned(),
            _ => String::new(),
        };
        let href = url.as_str().to_owned();
        let location = self.object();
        for (name, value) in [
            ("href", href.clone()),
            ("origin", origin),
            ("protocol", format!("{}:", url.scheme())),
            ("host", host),
            ("hostname", url.host_str().unwrap_or_default().to_owned()),
            (
                "port",
                url.port().map(|port| port.to_string()).unwrap_or_default(),
            ),
            ("pathname", url.path().to_owned()),
            ("search", part("?", url.query())),
            ("hash", part("#", url.fragment())),
        ] {
            location.define(Key::name(name), Property::data(Value::str(&value)));
        }
        let to_string = self.function("toString", 0, false, move |_, _, _| Ok(Value::str(&href)));
        location.define(
            Key::name("toString"),
            Property::data(Value::Object(to_string)),
        );
        let location = Value::Object(location);
        self.set_global("location", location.clone());
        let global = Value::Object(self.global.clone());
        if let Ok(Value::Object(document)) = crate::ops::get(self, &global, &Key::name("document"))
        {
            document.define(Key::name("location"), Property::data(location));
        }
    }
}

#[cfg(test)]
mod tests;
