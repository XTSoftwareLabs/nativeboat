//! The proxy for a URL as Chromium resolves it from the Windows settings, for
//! Electron's `session.resolveProxy`: the user's proxy server and bypass
//! list, or their configuration script and automatic detection, written as
//! Chromium writes proxy lists (`DIRECT`, `PROXY host:port`, `SOCKS
//! host:port`).

use crate::prelude::*;
use alloc::vec::Vec;
use core::ffi::c_void;

type Handle = *mut c_void;

/// `WINHTTP_CURRENT_USER_IE_PROXY_CONFIG`.
#[repr(C)]
struct UserConfig {
    auto_detect: i32,
    auto_config_url: *mut u16,
    proxy: *mut u16,
    bypass: *mut u16,
}

/// `WINHTTP_AUTOPROXY_OPTIONS`.
#[repr(C)]
struct AutoProxyOptions {
    flags: u32,
    auto_detect_flags: u32,
    auto_config_url: *const u16,
    reserved: *mut c_void,
    reserved_size: u32,
    auto_logon: i32,
}

/// `WINHTTP_PROXY_INFO`.
#[repr(C)]
struct ProxyInfo {
    access_type: u32,
    proxy: *mut u16,
    bypass: *mut u16,
}

#[link(name = "winhttp", kind = "raw-dylib")]
unsafe extern "system" {
    fn WinHttpGetIEProxyConfigForCurrentUser(config: *mut UserConfig) -> i32;
    fn WinHttpOpen(
        agent: *const u16,
        access: u32,
        proxy: *const u16,
        bypass: *const u16,
        flags: u32,
    ) -> Handle;
    fn WinHttpGetProxyForUrl(
        session: Handle,
        url: *const u16,
        options: *const AutoProxyOptions,
        info: *mut ProxyInfo,
    ) -> i32;
    fn WinHttpCloseHandle(handle: Handle) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GlobalFree(memory: *mut c_void) -> *mut c_void;
}

const NO_PROXY: u32 = 1;
const NAMED_PROXY: u32 = 3;
const AUTO_DETECT: u32 = 1;
const CONFIG_URL: u32 = 2;
const DETECT_DHCP: u32 = 1;
const DETECT_DNS_A: u32 = 2;

/// Takes a string WinHTTP allocated, freeing it.
fn take(text: *mut u16) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let mut length = 0;
    // SAFETY: WinHTTP returns null-terminated strings.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the string has `length` units before its terminator.
    let value = String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(text, length) });
    // SAFETY: WinHTTP allocates these strings with GlobalAlloc.
    unsafe { GlobalFree(text.cast()) };
    Some(value)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// The user's proxy settings: whether settings are detected automatically,
/// the configuration script's URL, the proxy servers and the bypass list.
pub(crate) struct Settings {
    pub(crate) auto_detect: bool,
    pub(crate) script: Option<String>,
    pub(crate) proxy: Option<String>,
    pub(crate) bypass: Option<String>,
}

/// The user's proxy settings, or nothing when Windows gives none.
pub(crate) fn settings() -> Option<Settings> {
    let mut config = UserConfig {
        auto_detect: 0,
        auto_config_url: core::ptr::null_mut(),
        proxy: core::ptr::null_mut(),
        bypass: core::ptr::null_mut(),
    };
    // SAFETY: the structure is valid for writing; its strings are taken below.
    if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) } == 0 {
        return None;
    }
    Some(Settings {
        auto_detect: config.auto_detect != 0,
        script: take(config.auto_config_url),
        proxy: take(config.proxy),
        bypass: take(config.bypass),
    })
}

/// The proxy list for `url` from the user's settings. This can wait on the
/// network for automatic detection, so it runs off the realm thread.
pub(crate) fn resolve(url: &str, direct: bool) -> String {
    if direct {
        return "DIRECT".into();
    }
    let mut config = UserConfig {
        auto_detect: 0,
        auto_config_url: core::ptr::null_mut(),
        proxy: core::ptr::null_mut(),
        bypass: core::ptr::null_mut(),
    };
    // SAFETY: the structure is valid for writing; its strings are taken below.
    if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) } == 0 {
        return "DIRECT".into();
    }
    let script = take(config.auto_config_url);
    let proxy = take(config.proxy);
    let bypass = take(config.bypass);
    if (config.auto_detect != 0 || script.is_some())
        && let Some(list) = automatic(url, config.auto_detect != 0, script.as_deref())
    {
        return list;
    }
    // Chromium falls back to the fixed settings when the script fails.
    fixed(url, proxy.as_deref(), bypass.as_deref())
}

/// The configuration script's or detected script's answer, or `None` when
/// neither gives one.
fn automatic(url: &str, detect: bool, script: Option<&str>) -> Option<String> {
    let agent = wide("nativeboat");
    // SAFETY: the agent string is terminated; the session is closed below.
    let session = unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            NO_PROXY,
            core::ptr::null(),
            core::ptr::null(),
            0,
        )
    };
    if session.is_null() {
        return None;
    }
    let script = script.map(wide);
    let options = AutoProxyOptions {
        flags: if detect { AUTO_DETECT } else { 0 } | if script.is_some() { CONFIG_URL } else { 0 },
        auto_detect_flags: if detect {
            DETECT_DHCP | DETECT_DNS_A
        } else {
            0
        },
        auto_config_url: script
            .as_ref()
            .map_or(core::ptr::null(), |script| script.as_ptr()),
        reserved: core::ptr::null_mut(),
        reserved_size: 0,
        auto_logon: 1,
    };
    let target = wide(url);
    let mut info = ProxyInfo {
        access_type: 0,
        proxy: core::ptr::null_mut(),
        bypass: core::ptr::null_mut(),
    };
    // SAFETY: the session, strings and structures stay valid for the call.
    let found = unsafe { WinHttpGetProxyForUrl(session, target.as_ptr(), &options, &mut info) };
    // SAFETY: the session handle is owned here.
    unsafe { WinHttpCloseHandle(session) };
    if found == 0 {
        return None;
    }
    let proxies = take(info.proxy);
    drop(take(info.bypass));
    match (info.access_type, proxies) {
        (NAMED_PROXY, Some(proxies)) => {
            let list = proxy_list(&proxies);
            Some(if list.is_empty() {
                "DIRECT".into()
            } else {
                list
            })
        }
        _ => Some("DIRECT".into()),
    }
}

/// A script's `host:port` list as Chromium's proxy list.
fn proxy_list(proxies: &str) -> String {
    proxies
        .split([';', ' ', '\t'])
        .filter(|server| !server.is_empty())
        .map(|server| server_entry("PROXY", server))
        .collect::<Vec<_>>()
        .join(";")
}

/// One proxy server in Chromium's notation, with the scheme's default port
/// when the setting gives none.
fn server_entry(kind: &str, server: &str) -> String {
    let server = server
        .split_once("://")
        .map_or(server, |(_, address)| address)
        .trim_end_matches('/');
    let has_port = match server.rfind(']') {
        Some(end) => server[end..].contains(':'),
        None => server.contains(':'),
    };
    if has_port {
        format!("{kind} {server}")
    } else {
        let port = if kind == "SOCKS" { 1080 } else { 80 };
        format!("{kind} {server}:{port}")
    }
}

/// The scheme and host of a URL, in lowercase.
fn scheme_and_host(url: &str) -> (String, String) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(bracketed) = host.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or_default()
    };
    (scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
}

/// Whether `text` matches a bypass pattern, where `*` stands for any run.
fn wildcard(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            match rest.strip_prefix(part) {
                Some(after) => rest = after,
                None => return false,
            }
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => return false,
            }
        }
    }
    true
}

/// Whether the bypass list exempts `host`: `<local>` names hosts without a
/// dot, and other entries are host patterns, optionally with a scheme.
fn bypassed(host: &str, bypass: &str) -> bool {
    bypass
        .split([';', ',', ' ', '\t', '\r', '\n'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            if entry.eq_ignore_ascii_case("<local>") {
                return !host.contains('.') && !host.contains(':');
            }
            let pattern = entry.split_once("://").map_or(entry, |(_, rest)| rest);
            let pattern = pattern.to_ascii_lowercase();
            wildcard(&pattern, host)
        })
}

/// The fixed settings' proxy for `url`: one server for every scheme, or
/// `scheme=server` entries with `socks` as the fallback.
fn fixed(url: &str, proxy: Option<&str>, bypass: Option<&str>) -> String {
    let Some(proxy) = proxy.map(str::trim).filter(|proxy| !proxy.is_empty()) else {
        return "DIRECT".into();
    };
    let (scheme, host) = scheme_and_host(url);
    if bypass.is_some_and(|bypass| bypassed(&host, bypass)) {
        return "DIRECT".into();
    }
    if !proxy.contains('=') {
        return server_entry("PROXY", proxy);
    }
    let entries: Vec<(String, &str)> = proxy
        .split([';', ' '])
        .filter_map(|entry| entry.split_once('='))
        .map(|(name, server)| (name.trim().to_ascii_lowercase(), server.trim()))
        .filter(|(_, server)| !server.is_empty())
        .collect();
    if let Some((_, server)) = entries.iter().find(|(name, _)| *name == scheme) {
        return server_entry("PROXY", server);
    }
    if let Some((_, server)) = entries.iter().find(|(name, _)| name == "socks") {
        return server_entry("SOCKS", server);
    }
    "DIRECT".into()
}

#[cfg(test)]
mod tests;
