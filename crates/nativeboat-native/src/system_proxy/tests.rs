use super::*;

#[test]
fn one_server_serves_every_scheme_with_its_default_port() {
    assert_eq!(fixed("https://youtube.com/", None, None), "DIRECT");
    assert_eq!(fixed("https://youtube.com/", Some(" "), None), "DIRECT");
    assert_eq!(
        fixed("https://youtube.com/watch", Some("proxy.lan:8080"), None),
        "PROXY proxy.lan:8080"
    );
    assert_eq!(
        fixed("http://example.com", Some("http://proxy.lan"), None),
        "PROXY proxy.lan:80"
    );
    assert_eq!(
        fixed("http://example.com", Some("[::1]"), None),
        "PROXY [::1]:80"
    );
    assert_eq!(
        fixed("http://example.com", Some("[::1]:3128"), None),
        "PROXY [::1]:3128"
    );
}

#[test]
fn scheme_entries_choose_by_scheme_and_fall_back_to_socks() {
    let rules = "http=web.lan:3128;https=secure.lan:3129;socks=socks.lan";
    assert_eq!(
        fixed("http://a.com", Some(rules), None),
        "PROXY web.lan:3128"
    );
    assert_eq!(
        fixed("HTTPS://a.com", Some(rules), None),
        "PROXY secure.lan:3129"
    );
    assert_eq!(
        fixed("ftp://a.com", Some(rules), None),
        "SOCKS socks.lan:1080"
    );
    assert_eq!(fixed("ftp://a.com", Some("http=web.lan:1"), None), "DIRECT");
}

#[test]
fn the_bypass_list_exempts_local_and_matching_hosts() {
    let bypass = "<local>;*.corp.example;10.*;http://exact.example";
    let via = |url: &str| fixed(url, Some("proxy.lan:8080"), Some(bypass));
    assert_eq!(via("http://intranet/page"), "DIRECT");
    assert_eq!(via("https://build.corp.example/x"), "DIRECT");
    assert_eq!(via("https://10.1.2.3:8443/"), "DIRECT");
    assert_eq!(via("https://EXACT.example/"), "DIRECT");
    assert_eq!(via("https://user@other.example/"), "PROXY proxy.lan:8080");
    assert_eq!(via("https://corp.example/"), "PROXY proxy.lan:8080");
    assert_eq!(via("http://[::1]/"), "PROXY proxy.lan:8080");
}

#[test]
fn urls_split_into_lowercase_schemes_and_hosts() {
    assert_eq!(
        scheme_and_host("HTTPS://User:pw@Example.COM:8443/a?b#c"),
        ("https".to_owned(), "example.com".to_owned())
    );
    assert_eq!(
        scheme_and_host("http://[2001:db8::1]:80/"),
        ("http".to_owned(), "2001:db8::1".to_owned())
    );
    assert_eq!(
        scheme_and_host("example.com"),
        (String::new(), "example.com".to_owned())
    );
}

#[test]
fn wildcards_match_runs_of_characters() {
    assert!(wildcard("*", "anything"));
    assert!(wildcard("a*c", "abbbc"));
    assert!(wildcard("*.b.*", "a.b.c"));
    assert!(!wildcard("a*c", "abd"));
    assert!(!wildcard("*.b.*.d", "a.b.c"));
    assert!(!wildcard("x*", "ab"));
    assert!(wildcard("exact", "exact"));
}

#[test]
fn script_answers_become_proxy_lists() {
    assert_eq!(
        proxy_list("a.lan:1; b.lan  c.lan:3"),
        "PROXY a.lan:1;PROXY b.lan:80;PROXY c.lan:3"
    );
    assert_eq!(proxy_list(" ; "), "");
}

#[test]
fn direct_sessions_and_the_live_settings_resolve() {
    assert_eq!(resolve("https://youtube.com", true), "DIRECT");
    let live = resolve("https://youtube.com", false);
    assert!(
        live == "DIRECT"
            || live
                .split(';')
                .all(|entry| entry.starts_with("PROXY ") || entry.starts_with("SOCKS ")),
        "{live}"
    );
}
