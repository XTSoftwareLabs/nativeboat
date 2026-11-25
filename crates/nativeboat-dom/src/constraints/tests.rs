use super::*;

fn document(html: &str) -> Dom {
    Dom::from_document(&nativeboat_html::parse_document(html))
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    dom.get_element_by_id(Dom::DOCUMENT, id).unwrap()
}

/// Digits only, or no pattern for `(`, which does not compile.
fn digits(pattern: &str, value: &str) -> Option<bool> {
    (pattern != "(").then(|| value.bytes().all(|byte| byte.is_ascii_digit()))
}

#[test]
fn numbers_parse_as_html_defines_them() {
    for (text, number) in [
        ("0", 0.0),
        ("-1.5", -1.5),
        (".5", 0.5),
        ("1e3", 1000.0),
        ("2E-1", 0.2),
        ("3e+2", 300.0),
    ] {
        assert_eq!(parse_number(text), Some(number), "{text}");
    }
    for text in ["", "-", "+1", "1.", "1e", "1e+", "a", "1 ", "1e999", "."] {
        assert_eq!(parse_number(text), None, "{text}");
    }
    assert_eq!((floor(-1.5), floor(2.0), round(2.5)), (-2.0, 2.0, 3.0));
    assert_eq!(format_number(-0.0001, 2), "0.00");
    assert_eq!(serialize_bound("010.50"), "10.50");
    assert_eq!(serialize_bound("x"), "x");
}

#[test]
fn email_addresses_and_urls_are_checked_as_chromium_checks_them() {
    assert_eq!(email_problem("a.b+c@example.co"), None);
    for (address, problem) in [
        (
            "plain",
            "Please include an '@' in the email address. 'plain' is missing an '@'.",
        ),
        (
            "@x.com",
            "Please enter a part followed by '@'. '@x.com' is incomplete.",
        ),
        (
            "a@",
            "Please enter a part following '@'. 'a@' is incomplete.",
        ),
        (
            "a b@x.com",
            "A part followed by '@' should not contain the symbol ' '.",
        ),
        (
            "a@x_y.com",
            "A part following '@' should not contain the symbol '_'.",
        ),
        ("a@x..com", "'.' is used at a wrong position in 'x..com'."),
        ("a@-x.com", "'.' is used at a wrong position in '-x.com'."),
    ] {
        assert_eq!(
            email_problem(address).as_deref(),
            Some(problem),
            "{address}"
        );
    }
    for url in [
        "https://example.com/path?q#f",
        "http:example.com",
        "HTTP://user:pass@host:8080/",
        "mailto:someone",
        "file:///C:/x",
        " https://example.com ",
        "http://[::1]:80/",
    ] {
        assert!(valid_url(url), "{url}");
    }
    for url in [
        "example.com",
        "1http://x",
        "ht tp://x",
        "http://",
        "https://exa mple.com",
        "https://host:port/",
        "http://a%b/",
    ] {
        assert!(!valid_url(url), "{url}");
    }
}

#[test]
fn dates_and_times_compare_once_their_parts_are_filled_in() {
    assert_eq!(
        date_key("date", "2024-02-29").as_deref(),
        Some("002024-02-29")
    );
    assert_eq!(date_key("date", "2023-02-29"), None);
    assert_eq!(date_key("date", "1900-02-29"), None);
    assert_eq!(
        date_key("date", "2000-02-29").as_deref(),
        Some("002000-02-29")
    );
    assert_eq!(date_key("date", "2024-04-31"), None);
    assert_eq!(date_key("date", "2024-13-01"), None);
    assert_eq!(date_key("date", "0000-01-01"), None);
    assert_eq!(date_key("date", "2024-01"), None);
    assert_eq!(date_key("month", "2024-07").as_deref(), Some("002024-07"));
    assert_eq!(date_key("month", "2024-7"), None);
    assert_eq!(date_key("week", "2024-W09").as_deref(), Some("002024-W09"));
    assert_eq!(date_key("week", "2024-W54"), None);
    assert_eq!(date_key("time", "09:30").as_deref(), Some("09:30:00.000"));
    assert_eq!(
        date_key("time", "23:59:59.5").as_deref(),
        Some("23:59:59.500")
    );
    assert_eq!(date_key("time", "24:00"), None);
    assert_eq!(date_key("time", "09:30:00.1234"), None);
    assert_eq!(date_key("time", "9"), None);
    assert_eq!(
        date_key("datetime-local", "2024-01-05T09:30").as_deref(),
        Some("002024-01-05T09:30:00.000")
    );
    assert_eq!(
        date_key("datetime-local", "2024-01-05 09:30").as_deref(),
        Some("002024-01-05T09:30:00.000")
    );
    assert_eq!(date_key("datetime-local", "2024-01-05"), None);
}

#[test]
fn candidates_exclude_disabled_read_only_hidden_and_listed_controls() {
    let dom = document(
        "<form id=f><input id=text><input id=hidden type=hidden><input id=reset type=reset>\
         <input id=button type=button><input id=readonly readonly><input id=box type=checkbox readonly>\
         <input id=disabled disabled><textarea id=area></textarea><textarea id=fixed readonly></textarea>\
         <select id=list></select><button id=submit></button><button id=plain type=button></button>\
         <fieldset disabled><legend><input id=legend></legend><legend><input id=second></legend>\
         <input id=inside></fieldset><datalist><input id=listed></datalist><output id=out></output>\
         <input id=unknown type=fancy></form>",
    );
    for id in ["text", "box", "area", "list", "submit", "legend", "unknown"] {
        assert!(dom.will_validate(by_id(&dom, id)), "{id}");
    }
    for id in [
        "hidden", "reset", "button", "readonly", "disabled", "fixed", "plain", "second", "inside",
        "listed", "out", "f",
    ] {
        assert!(!dom.will_validate(by_id(&dom, id)), "{id}");
    }
    assert!(!dom.will_validate(Dom::DOCUMENT));
}

#[test]
fn controls_report_what_they_fail_and_chromium_messages() {
    let mut dom = document(
        "<input id=text required><textarea id=area required></textarea>\
         <input id=box type=checkbox required><input id=file type=file required>\
         <input id=files type=file multiple required><input id=range type=range required>\
         <input id=email type=email value=plain><input id=emails type=email multiple value='a@b.c, x'>\
         <input id=url type=url value=nowhere><input id=code pattern='[0-9]+' value=12a>\
         <input id=broken pattern='(' value=x><input id=number type=number value=abc>\
         <input id=low type=number min=010.50 value=3><input id=high type=number max=5 value=9>\
         <input id=step type=number min=0 step=0.25 value=1.1><input id=edge type=number min=0 max=1 step=0.75 value=1>\
         <input id=any type=number step=any value=1.234><input id=badstep type=number min=0 step=-1 value=1.5>\
         <input id=based type=number value=0.5 step=1><input id=date type=date min=2024-01-01 max=2024-12-31 value=2023-06-01>\
         <input id=late type=date max=2024-12-31 value=2025-01-01><input id=baddate type=date min=2024-01-01 value=soon>\
         <input id=fine value=ok>",
    );
    let check = |dom: &Dom, id: &str, flags: u16, message: &str| {
        let node = by_id(dom, id);
        assert_eq!(dom.validity(node), flags, "{id}");
        assert_eq!(dom.validation_message(node), message, "{id}");
    };
    check(&dom, "text", VALUE_MISSING, "Please fill out this field.");
    check(&dom, "area", VALUE_MISSING, "Please fill out this field.");
    check(
        &dom,
        "box",
        VALUE_MISSING,
        "Please check this box if you want to proceed.",
    );
    check(&dom, "file", VALUE_MISSING, "Please select a file.");
    check(
        &dom,
        "files",
        VALUE_MISSING,
        "Please select one or more files.",
    );
    check(&dom, "range", 0, "");
    check(
        &dom,
        "email",
        TYPE_MISMATCH,
        "Please include an '@' in the email address. 'plain' is missing an '@'.",
    );
    check(
        &dom,
        "emails",
        TYPE_MISMATCH,
        "Please enter a comma separated list of email addresses.",
    );
    check(&dom, "url", TYPE_MISMATCH, "Please enter a URL.");
    // Without a matcher patterns set no constraint.
    check(&dom, "code", 0, "");
    dom.set_pattern_matcher(digits);
    check(
        &dom,
        "code",
        PATTERN_MISMATCH,
        "Please match the requested format.",
    );
    check(&dom, "broken", 0, "");
    check(&dom, "number", BAD_INPUT, "Please enter a number.");
    check(
        &dom,
        "low",
        RANGE_UNDERFLOW | STEP_MISMATCH,
        "Value must be greater than or equal to 10.50.",
    );
    check(
        &dom,
        "high",
        RANGE_OVERFLOW,
        "Value must be less than or equal to 5.",
    );
    check(
        &dom,
        "step",
        STEP_MISMATCH,
        "Please enter a valid value. The two nearest valid values are 1.00 and 1.25.",
    );
    check(
        &dom,
        "edge",
        STEP_MISMATCH,
        "Please enter a valid value. The nearest valid value is 0.75.",
    );
    check(&dom, "any", 0, "");
    check(
        &dom,
        "badstep",
        STEP_MISMATCH,
        "Please enter a valid value. The two nearest valid values are 1 and 2.",
    );
    check(&dom, "based", 0, "");
    check(
        &dom,
        "date",
        RANGE_UNDERFLOW,
        "Value must be 2024-01-01 or later.",
    );
    check(
        &dom,
        "late",
        RANGE_OVERFLOW,
        "Value must be 2024-12-31 or earlier.",
    );
    check(&dom, "baddate", 0, "");
    check(&dom, "fine", 0, "");
    // Filled in, the required controls are valid.
    for id in ["text", "area"] {
        let node = by_id(&dom, id);
        dom.set_value(node, "x");
        assert_eq!(dom.validity(node), 0);
    }
    let node = by_id(&dom, "box");
    dom.set_checked(node, true);
    assert_eq!(dom.validity(node), 0);
}

#[test]
fn custom_messages_come_first_and_change_the_document_only_when_new() {
    let mut dom = document("<input id=text required><input id=off disabled>");
    let text = by_id(&dom, "text");
    let version = dom.version();
    dom.set_custom_validity(text, "Pick a name");
    assert!(dom.version() > version);
    assert_eq!(dom.validity(text), VALUE_MISSING | CUSTOM_ERROR);
    assert_eq!(dom.validation_message(text), "Pick a name");
    let version = dom.version();
    dom.set_custom_validity(text, "Pick a name");
    assert_eq!(dom.version(), version);
    dom.set_custom_validity(text, "");
    assert!(dom.version() > version);
    let version = dom.version();
    dom.set_custom_validity(text, "");
    assert_eq!(dom.version(), version);
    assert_eq!(dom.validation_message(text), "Please fill out this field.");
    // Controls that are not validated report nothing.
    let off = by_id(&dom, "off");
    dom.set_custom_validity(off, "ignored");
    assert_eq!(
        (dom.validity(off), dom.validation_message(off)),
        (0, String::new())
    );
}

#[test]
fn radio_groups_and_select_placeholders_count_as_missing() {
    let mut dom = document(
        "<form><input id=a type=radio name=g required><input id=b type=radio name=g></form>\
         <input id=c type=radio name=g><input id=alone type=radio required><input id=blank type=radio name='' required>\
         <select id=pick required><option value=''>Choose</option><option>One</option></select>\
         <select id=grouped required><optgroup><option value=''>Choose</option></optgroup></select>\
         <select id=many required multiple><option value=''>Choose</option></select>\
         <select id=tall required size=3><option value=''>Choose</option></select>\
         <select id=empty required></select><select id=valued required><option>One</option></select>",
    );
    let (a, b, c) = (by_id(&dom, "a"), by_id(&dom, "b"), by_id(&dom, "c"));
    assert_eq!(dom.validity(b), VALUE_MISSING);
    assert_eq!(
        dom.validation_message(b),
        "Please select one of these options."
    );
    // The radio outside the form belongs to another group.
    assert_eq!(dom.validity(c), 0);
    dom.set_checked(b, true);
    assert_eq!((dom.validity(a), dom.validity(b)), (0, 0));
    for id in ["alone", "blank"] {
        assert_eq!(dom.validity(by_id(&dom, id)), VALUE_MISSING, "{id}");
    }
    let pick = by_id(&dom, "pick");
    assert_eq!(dom.validity(pick), VALUE_MISSING);
    assert_eq!(
        dom.validation_message(pick),
        "Please select an item in the list."
    );
    dom.set_value(pick, "One");
    assert_eq!(dom.validity(pick), 0);
    for (id, flags) in [
        ("grouped", 0),
        ("many", VALUE_MISSING),
        ("tall", VALUE_MISSING),
        ("empty", VALUE_MISSING),
        ("valued", 0),
    ] {
        assert_eq!(dom.validity(by_id(&dom, id)), flags, "{id}");
    }
}

#[test]
fn forms_and_fieldsets_are_invalid_while_a_control_in_them_is() {
    let mut dom = document(
        "<form><input id=nameless required></form><form id=f><fieldset id=set><input id=name required></fieldset>\
         <fieldset id=other><input id=note></fieldset><button id=go></button></form>\
         <input id=outside required form=f><p id=text></p><input id=off required disabled>",
    );
    let state = |dom: &Dom, id: &str| dom.validation_state(by_id(dom, id));
    assert_eq!(state(&dom, "f"), Some(false));
    let nameless = dom.form_owner(by_id(&dom, "nameless")).unwrap();
    assert_eq!(dom.validation_state(nameless), Some(false));
    assert_eq!(state(&dom, "set"), Some(false));
    assert_eq!(state(&dom, "other"), Some(true));
    assert_eq!(state(&dom, "name"), Some(false));
    assert_eq!(state(&dom, "go"), Some(true));
    assert_eq!(state(&dom, "text"), None);
    assert_eq!(state(&dom, "off"), None);
    assert_eq!(dom.validation_state(Dom::DOCUMENT), None);
    let name = by_id(&dom, "name");
    dom.set_value(name, "Ann");
    assert_eq!(state(&dom, "set"), Some(true));
    // The control outside the form still fails, until it is filled in.
    assert_eq!(state(&dom, "f"), Some(false));
    let outside = by_id(&dom, "outside");
    dom.set_value(outside, "x");
    assert_eq!(state(&dom, "f"), Some(true));
    dom.set_value(outside, "");
    // The flags selectors test follow.
    use nativeboat_css::selector::state;
    let flags =
        |dom: &Dom, id: &str| dom.element_state(by_id(dom, id)) & (state::VALID | state::INVALID);
    assert_eq!(flags(&dom, "name"), state::VALID);
    assert_eq!(flags(&dom, "outside"), state::INVALID);
    assert_eq!(flags(&dom, "text"), 0);
    assert!(dom.matches(name, "input:valid").unwrap());
    assert!(dom.matches(by_id(&dom, "outside"), ":invalid").unwrap());
    assert!(!dom.matches(by_id(&dom, "off"), ":valid, :invalid").unwrap());
}
