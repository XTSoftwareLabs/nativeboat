//! Opt-in diagnostics for compiled programs. With `NATIVEBOAT_TRACE_EXIT`
//! set, a program reports on standard error each reason it may end: an
//! explicit exit or quit, an uncaught exception (handled or not), and the
//! host loop's end. Compiled code has no source positions, so these name
//! what happened rather than where.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{JsStr, platform};
use alloc::collections::BTreeMap;

/// Whether exit tracing is on.
pub fn enabled() -> bool {
    platform::environment_value(&JsStr::from("NATIVEBOAT_TRACE_EXIT")).is_some()
}

/// Reports one exit reason when tracing is on.
pub fn exit(reason: &str) {
    if enabled() {
        platform::write_stderr(&format!("nativeboat: {reason}\n"));
    }
}

/// Whether `NATIVEBOAT_TRACE_THROWS` asks for every exception thrown,
/// including those a script catches, to be reported.
pub fn throws_traced() -> bool {
    use core::sync::atomic::{AtomicU8, Ordering};
    /// Zero until read, then one when tracing is off and two when it is on.
    static TRACING: AtomicU8 = AtomicU8::new(0);
    let mut tracing = TRACING.load(Ordering::Relaxed);
    if tracing == 0 {
        tracing = if platform::environment_value(&JsStr::from("NATIVEBOAT_TRACE_THROWS")).is_some()
        {
            2
        } else {
            1
        };
        TRACING.store(tracing, Ordering::Relaxed);
    }
    tracing == 2
}

/// Whether `NATIVEBOAT_PROFILE_CALLS` asks for the calls compiled code
/// makes to be counted and timed by call site.
pub fn calls_profiled() -> bool {
    platform::environment_value(&JsStr::from("NATIVEBOAT_PROFILE_CALLS")).is_some()
}

/// Calls by call site, named as the source writes the callee: how many,
/// how long they took including what they called, and how long without the
/// profiled calls they made. Every few seconds the sites that took longest
/// by themselves so far are reported on standard error.
#[derive(Debug, Default)]
pub struct CallProfile {
    sites: BTreeMap<usize, Site>,
    /// The time the profiled calls in progress spent in profiled calls
    /// they made, innermost last.
    nested: Vec<f64>,
    reported: f64,
}

/// What a call site's calls took.
#[derive(Debug, Default, Clone)]
struct Site {
    name: String,
    calls: u64,
    total: f64,
    own: f64,
}

impl CallProfile {
    /// Starts timing a call.
    pub fn enter(&mut self) {
        self.nested.push(0.0);
    }

    /// Counts a call of `site` that took `elapsed` milliseconds, the call
    /// [`CallProfile::enter`] last started.
    pub fn record(&mut self, site: &str, elapsed: f64) {
        let inner = self.nested.pop().unwrap_or(0.0);
        if let Some(outer) = self.nested.last_mut() {
            *outer += elapsed;
        }
        let entry = self
            .sites
            .entry(site.as_ptr() as usize ^ site.len())
            .or_insert_with(|| Site {
                name: site.to_owned(),
                ..Site::default()
            });
        entry.calls += 1;
        entry.total += elapsed;
        entry.own += (elapsed - inner).max(0.0);
        let now = platform::now();
        if now - self.reported >= 5000.0 {
            self.reported = now;
            platform::write_stderr(&self.report(40));
        }
    }

    /// The `count` sites that took longest by themselves, longest first.
    pub fn report(&self, count: usize) -> String {
        let mut sites: Vec<&Site> = self.sites.values().collect();
        sites.sort_by(|a, b| b.own.total_cmp(&a.own));
        let mut text = String::from("nativeboat: calls by site (own ms, total ms, count)\n");
        for site in sites.into_iter().take(count) {
            text.push_str(&format!(
                "  {:10.1} {:10.1} {:8} {}\n",
                site.own, site.total, site.calls, site.name
            ));
        }
        text
    }
}

/// Reports a thrown value when [`throws_traced`]: an error's own `name`
/// and `message` data, or a primitive's value. Reading them runs no
/// script code.
pub fn thrown(value: &crate::Value) {
    if throws_traced() {
        platform::write_stderr(&format!("nativeboat: thrown {}\n", describe_thrown(value)));
    }
}

/// A thrown value as [`thrown`] reports it.
pub(crate) fn describe_thrown(value: &crate::Value) -> String {
    let text = |value: Option<crate::Property>| match value.map(|property| property.slot) {
        Some(crate::object::Slot::Data {
            value: crate::Value::String(text),
            ..
        }) => text.to_rust_lossy(),
        _ => String::new(),
    };
    match value {
        crate::Value::Object(object) => {
            let lookup = |name: &str| {
                let key = crate::Key::name(name);
                let mut current = Some(object.clone());
                while let Some(holder) = current {
                    if let Some(property) = holder.own_property(&key) {
                        return Some(property);
                    }
                    current = holder.data().prototype.clone();
                }
                None
            };
            format!("{}: {}", text(lookup("name")), text(lookup("message")))
        }
        crate::Value::String(text) => format!("string {:?}", text.to_rust_lossy()),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests;
