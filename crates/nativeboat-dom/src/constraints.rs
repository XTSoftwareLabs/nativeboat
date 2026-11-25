//! Constraint validation: the constraints a form control's value fails, the
//! messages Chromium gives for them, and the `:valid` and `:invalid` states
//! of controls, forms and fieldsets.

use crate::prelude::*;

use crate::{Dom, NodeId};

/// The constraints a value fails, as `ValidityState` names them.
pub mod validity {
    /// `valueMissing`: a required control has no value.
    pub const VALUE_MISSING: u16 = 1;
    /// `typeMismatch`: the value is not an email address or URL.
    pub const TYPE_MISMATCH: u16 = 1 << 1;
    /// `patternMismatch`: the value does not match the `pattern`.
    pub const PATTERN_MISMATCH: u16 = 1 << 2;
    /// `tooLong`: a value the user typed is longer than `maxlength`.
    pub const TOO_LONG: u16 = 1 << 3;
    /// `tooShort`: a value the user typed is shorter than `minlength`.
    pub const TOO_SHORT: u16 = 1 << 4;
    /// `rangeUnderflow`: the value is below `min`.
    pub const RANGE_UNDERFLOW: u16 = 1 << 5;
    /// `rangeOverflow`: the value is above `max`.
    pub const RANGE_OVERFLOW: u16 = 1 << 6;
    /// `stepMismatch`: the value is not on a `step` from its base.
    pub const STEP_MISMATCH: u16 = 1 << 7;
    /// `badInput`: what was typed is not a value of the control's type.
    pub const BAD_INPUT: u16 = 1 << 8;
    /// `customError`: a script set a validation message.
    pub const CUSTOM_ERROR: u16 = 1 << 9;
}

use validity::*;

/// Whether a value matches an `<input>`'s `pattern`, compiled as HTML
/// compiles it; `None` when the pattern does not compile, which sets no
/// constraint. The runtime gives one with its regular expressions.
pub type PatternMatcher = fn(pattern: &str, value: &str) -> Option<bool>;

/// The input types `readonly` bars from validation.
const READ_ONLY_TYPES: &[&str] = &[
    "text",
    "search",
    "url",
    "tel",
    "email",
    "password",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "number",
];

/// The input types `pattern` applies to.
const PATTERN_TYPES: &[&str] = &["text", "search", "url", "tel", "email", "password"];

/// The input types whose values are dates or times.
const DATE_TYPES: &[&str] = &["date", "month", "week", "time", "datetime-local"];

/// Input types the DOM knows; any other behaves as `text`.
const INPUT_TYPES: &[&str] = &[
    "hidden",
    "text",
    "search",
    "tel",
    "url",
    "email",
    "password",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "number",
    "range",
    "color",
    "checkbox",
    "radio",
    "file",
    "submit",
    "image",
    "reset",
    "button",
];

impl Dom {
    /// Gives `pattern` constraints their regular expressions; without a
    /// matcher, patterns set no constraint.
    pub fn set_pattern_matcher(&mut self, matcher: PatternMatcher) {
        self.pattern_matcher = Some(matcher);
        self.touch();
    }

    /// `setCustomValidity`: an empty message clears the custom error.
    pub fn set_custom_validity(&mut self, id: NodeId, message: &str) {
        let changed = if message.is_empty() {
            self.custom_validity.remove(&id).is_some()
        } else {
            self.custom_validity
                .insert(id, message.to_owned())
                .as_deref()
                != Some(message)
        };
        if changed {
            self.touch();
        }
    }

    /// An input's type as it behaves: its `type` attribute when the DOM
    /// knows it, else `text`.
    fn behaving_type(&self, id: NodeId) -> String {
        let kind = self.input_type(id);
        if INPUT_TYPES.contains(&kind.as_str()) {
            kind
        } else {
            "text".to_owned()
        }
    }

    /// `willValidate`: whether the element is a candidate for constraint
    /// validation. Disabled controls, read-only text controls, hidden
    /// inputs, buttons that do not submit and controls in a `<datalist>` are
    /// not.
    pub fn will_validate(&self, id: NodeId) -> bool {
        let candidate = match self.local_name(id) {
            Some("input") => {
                let kind = self.behaving_type(id);
                !matches!(kind.as_str(), "hidden" | "reset" | "button")
                    && !(self.has_attribute(id, "readonly")
                        && READ_ONLY_TYPES.contains(&kind.as_str()))
            }
            Some("textarea") => !self.has_attribute(id, "readonly"),
            Some("select") => true,
            Some("button") => !matches!(
                self.get_attribute(id, "type")
                    .map(|kind| kind.trim().to_ascii_lowercase())
                    .as_deref(),
                Some("reset" | "button")
            ),
            _ => false,
        };
        candidate && !self.disabled_control(id) && !self.in_data_list(id)
    }

    /// Whether a control is disabled by its attribute or by a disabled
    /// `<fieldset>` around it, outside that fieldset's first `<legend>`.
    fn disabled_control(&self, id: NodeId) -> bool {
        if self.has_attribute(id, "disabled") {
            return true;
        }
        let mut child = id;
        while let Some(parent) = self.parent(child) {
            if self.local_name(parent) == Some("fieldset") && self.has_attribute(parent, "disabled")
            {
                let legend = self
                    .children(parent)
                    .into_iter()
                    .find(|node| self.local_name(*node) == Some("legend"));
                if legend != Some(child) {
                    return true;
                }
            }
            child = parent;
        }
        false
    }

    fn in_data_list(&self, id: NodeId) -> bool {
        let mut current = self.parent(id);
        while let Some(node) = current {
            if self.local_name(node) == Some("datalist") {
                return true;
            }
            current = self.parent(node);
        }
        false
    }

    /// The constraints the element's value fails, from [`validity`]; none
    /// for elements that are not validated.
    pub fn validity(&self, id: NodeId) -> u16 {
        if !self.will_validate(id) {
            return 0;
        }
        let custom = if self.custom_validity.contains_key(&id) {
            CUSTOM_ERROR
        } else {
            0
        };
        let required = self.has_attribute(id, "required");
        custom
            | match self.local_name(id) {
                Some("input") => self.input_validity(id, required),
                Some("textarea") if required && self.value(id).is_empty() => VALUE_MISSING,
                Some("select") if required && self.select_missing(id) => VALUE_MISSING,
                _ => 0,
            }
    }

    fn input_validity(&self, id: NodeId, required: bool) -> u16 {
        let kind = self.behaving_type(id);
        let missing = |missing: bool| if missing { VALUE_MISSING } else { 0 };
        match kind.as_str() {
            "checkbox" => missing(required && !self.checked(id)),
            "radio" => {
                let group = self.radio_group(id);
                missing(
                    group
                        .iter()
                        .any(|node| self.has_attribute(*node, "required"))
                        && !group.iter().any(|node| self.checked(*node)),
                )
            }
            "range" | "color" | "submit" | "image" => 0,
            _ => {
                let value = self.value(id);
                if value.is_empty() {
                    return missing(required);
                }
                let values = self.input_values(id, &kind, &value);
                let mut flags = 0;
                if (kind == "email" && values.iter().any(|value| email_problem(value).is_some()))
                    || (kind == "url" && !valid_url(&value))
                {
                    flags |= TYPE_MISMATCH;
                }
                if PATTERN_TYPES.contains(&kind.as_str())
                    && let (Some(pattern), Some(matcher)) =
                        (self.get_attribute(id, "pattern"), self.pattern_matcher)
                    && values
                        .iter()
                        .any(|value| matcher(pattern, value) == Some(false))
                {
                    flags |= PATTERN_MISMATCH;
                }
                if kind == "number" {
                    flags |= match parse_number(&value) {
                        Some(number) => self.number_range(id, number).0,
                        None => BAD_INPUT,
                    };
                } else if DATE_TYPES.contains(&kind.as_str()) {
                    flags |= self.date_range(id, &kind, &value);
                }
                flags
            }
        }
    }

    /// An email input's addresses: each of a `multiple` input's comma
    /// separated values, else its whole value.
    fn input_values(&self, id: NodeId, kind: &str, value: &str) -> Vec<String> {
        if kind == "email" && self.has_attribute(id, "multiple") {
            value
                .split(',')
                .map(|part| part.trim_matches(is_space).to_owned())
                .collect()
        } else {
            vec![value.to_owned()]
        }
    }

    /// The radio buttons of a radio button's group: those with its name in
    /// its tree and form, or itself alone without a name.
    fn radio_group(&self, id: NodeId) -> Vec<NodeId> {
        let Some(name) = self
            .get_attribute(id, "name")
            .filter(|name| !name.is_empty())
        else {
            return vec![id];
        };
        let owner = self.form_owner(id);
        let mut root = id;
        while let Some(parent) = self.parent(root) {
            root = parent;
        }
        self.descendants(root)
            .into_iter()
            .filter(|node| {
                self.local_name(*node) == Some("input")
                    && self.behaving_type(*node) == "radio"
                    && self.get_attribute(*node, "name") == Some(name)
                    && self.form_owner(*node) == owner
            })
            .collect()
    }

    /// Whether a required `<select>` has no option selected, or only its
    /// placeholder: an empty first option of a one-line list.
    fn select_missing(&self, id: NodeId) -> bool {
        let selected = self.selected_options(id);
        let Some(first) = selected.first() else {
            return true;
        };
        let listed = !self.has_attribute(id, "multiple")
            && self
                .get_attribute(id, "size")
                .and_then(|size| size.trim().parse::<u32>().ok())
                .is_none_or(|size| size <= 1);
        selected.len() == 1
            && listed
            && self.options(id).first() == Some(first)
            && self.parent(*first) == Some(id)
            && self.value(*first).is_empty()
    }

    /// A number's range and step constraints, with the step's nearest
    /// valid values below and above when it is off its step.
    fn number_range(&self, id: NodeId, number: f64) -> (u16, Option<(f64, f64)>) {
        let bound = |name| self.get_attribute(id, name).and_then(parse_number);
        let (min, max) = (bound("min"), bound("max"));
        let mut flags = 0;
        if min.is_some_and(|min| number < min) {
            flags |= RANGE_UNDERFLOW;
        }
        if max.is_some_and(|max| number > max) {
            flags |= RANGE_OVERFLOW;
        }
        let step = match self.get_attribute(id, "step") {
            Some(step) if step.trim().eq_ignore_ascii_case("any") => None,
            Some(step) => Some(parse_number(step).filter(|step| *step > 0.0).unwrap_or(1.0)),
            None => Some(1.0),
        };
        let Some(step) = step else {
            return (flags, None);
        };
        let base = min.or_else(|| bound("value")).unwrap_or(0.0);
        let steps = (number - base) / step;
        if (steps - round(steps)).abs() <= 1e-9 * steps.abs().max(1.0) {
            return (flags, None);
        }
        let below = base + floor(steps) * step;
        (flags | STEP_MISMATCH, Some((below, below + step)))
    }

    /// A date or time's range constraints. Values of one type compare as
    /// text once their optional parts are filled in.
    fn date_range(&self, id: NodeId, kind: &str, value: &str) -> u16 {
        let Some(value) = date_key(kind, value) else {
            return 0;
        };
        let bound = |name| {
            self.get_attribute(id, name)
                .and_then(|bound| date_key(kind, bound))
        };
        let mut flags = 0;
        if bound("min").is_some_and(|min| value < min) {
            flags |= RANGE_UNDERFLOW;
        }
        if bound("max").is_some_and(|max| value > max) {
            flags |= RANGE_OVERFLOW;
        }
        flags
    }

    /// `validationMessage`: the message Chromium shows, in English, for the
    /// first constraint the value fails.
    pub fn validation_message(&self, id: NodeId) -> String {
        let flags = self.validity(id);
        if flags & CUSTOM_ERROR != 0 {
            return self.custom_validity[&id].clone();
        }
        let kind = self.behaving_type(id);
        let value = self.value(id);
        let attribute = |name| self.get_attribute(id, name).unwrap_or("").trim();
        if flags & BAD_INPUT != 0 {
            "Please enter a number.".to_owned()
        } else if flags & VALUE_MISSING != 0 {
            match (self.local_name(id), kind.as_str()) {
                (Some("select"), _) => "Please select an item in the list.",
                (_, "checkbox") => "Please check this box if you want to proceed.",
                (_, "radio") => "Please select one of these options.",
                (_, "file") if self.has_attribute(id, "multiple") => {
                    "Please select one or more files."
                }
                (_, "file") => "Please select a file.",
                _ => "Please fill out this field.",
            }
            .to_owned()
        } else if flags & TYPE_MISMATCH != 0 {
            if kind == "url" {
                "Please enter a URL.".to_owned()
            } else if self.has_attribute(id, "multiple") {
                "Please enter a comma separated list of email addresses.".to_owned()
            } else {
                email_problem(&value).unwrap_or_default()
            }
        } else if flags & PATTERN_MISMATCH != 0 {
            "Please match the requested format.".to_owned()
        } else if flags & RANGE_UNDERFLOW != 0 {
            match kind.as_str() {
                "number" => format!(
                    "Value must be greater than or equal to {}.",
                    serialize_bound(attribute("min"))
                ),
                _ => format!("Value must be {} or later.", attribute("min")),
            }
        } else if flags & RANGE_OVERFLOW != 0 {
            match kind.as_str() {
                "number" => format!(
                    "Value must be less than or equal to {}.",
                    serialize_bound(attribute("max"))
                ),
                _ => format!("Value must be {} or earlier.", attribute("max")),
            }
        } else if flags & STEP_MISMATCH != 0 {
            let number = parse_number(&value).unwrap_or_default();
            let (below, above) = self.number_range(id, number).1.unwrap_or_default();
            let base = [attribute("min"), attribute("value")]
                .into_iter()
                .find(|text| parse_number(text).is_some())
                .unwrap_or("");
            let places = decimal_places(attribute("step")).max(decimal_places(base));
            let max = parse_number(attribute("max"));
            if max.is_some_and(|max| above > max) {
                format!(
                    "Please enter a valid value. The nearest valid value is {}.",
                    format_number(below, places)
                )
            } else {
                format!(
                    "Please enter a valid value. The two nearest valid values are {} and {}.",
                    format_number(below, places),
                    format_number(above, places)
                )
            }
        } else {
            String::new()
        }
    }

    /// Whether `:valid` (`Some(true)`) or `:invalid` (`Some(false)`) matches
    /// the element: controls by their own constraints, forms and fieldsets
    /// by those of the controls in them; `None` for other elements and
    /// controls that are not validated.
    pub fn validation_state(&self, id: NodeId) -> Option<bool> {
        match self.local_name(id)? {
            "input" | "textarea" | "select" | "button" => {
                self.will_validate(id).then(|| self.validity(id) == 0)
            }
            "form" => {
                // Controls elsewhere join a form with an id through their
                // `form` attribute.
                let scope = if self.has_attribute(id, "id") {
                    Dom::DOCUMENT
                } else {
                    id
                };
                Some(
                    self.descendants(scope)
                        .into_iter()
                        .filter(|node| self.form_owner(*node) == Some(id))
                        .all(|node| self.validity(node) == 0),
                )
            }
            "fieldset" => Some(
                self.descendants(id)
                    .into_iter()
                    .all(|node| self.validity(node) == 0),
            ),
            _ => None,
        }
    }
}

fn is_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\u{c}' | '\r')
}

/// A valid floating-point number as HTML defines it: an optional `-`,
/// digits with an optional fraction, or a fraction alone, and an optional
/// exponent.
pub(crate) fn parse_number(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut index = usize::from(bytes.first() == Some(&b'-'));
    let digits = |index: &mut usize| {
        let start = *index;
        while bytes.get(*index).is_some_and(u8::is_ascii_digit) {
            *index += 1;
        }
        *index > start
    };
    let whole = digits(&mut index);
    // A decimal point needs digits after it.
    let point = bytes.get(index) == Some(&b'.');
    if point {
        index += 1;
    }
    let fraction = point && digits(&mut index);
    if point != fraction || (!whole && !fraction) {
        return None;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'-' | b'+')) {
            index += 1;
        }
        if !digits(&mut index) {
            return None;
        }
    }
    if index != bytes.len() {
        return None;
    }
    text.parse::<f64>().ok().filter(|number| number.is_finite())
}

fn floor(number: f64) -> f64 {
    let truncated = number as i64 as f64;
    if truncated > number {
        truncated - 1.0
    } else {
        truncated
    }
}

fn round(number: f64) -> f64 {
    floor(number + 0.5)
}

/// The digits after a number's decimal point, which messages keep.
fn decimal_places(text: &str) -> usize {
    text.split_once('.').map_or(0, |(_, fraction)| {
        fraction.bytes().take_while(u8::is_ascii_digit).count()
    })
}

fn format_number(number: f64, places: usize) -> String {
    let text = format!("{number:.places$}");
    if text
        .trim_start_matches('-')
        .bytes()
        .all(|byte| matches!(byte, b'0' | b'.'))
    {
        format!("{:.places$}", 0.0)
    } else {
        text
    }
}

/// A `min` or `max` as Chromium writes it in a message: the number it
/// parses to, with its decimal places.
fn serialize_bound(text: &str) -> String {
    parse_number(text).map_or_else(
        || text.to_owned(),
        |number| format_number(number, decimal_places(text)),
    )
}

/// What is wrong with an email address, in Chromium's words; `None` for a
/// valid address.
fn email_problem(address: &str) -> Option<String> {
    let Some((local, domain)) = address.split_once('@') else {
        return Some(format!(
            "Please include an '@' in the email address. '{address}' is missing an '@'."
        ));
    };
    if local.is_empty() {
        return Some(format!(
            "Please enter a part followed by '@'. '{address}' is incomplete."
        ));
    }
    if domain.is_empty() {
        return Some(format!(
            "Please enter a part following '@'. '{address}' is incomplete."
        ));
    }
    if let Some(symbol) = local.chars().find(|character| {
        !character.is_ascii_alphanumeric() && !".!#$%&'*+/=?^_`{|}~-".contains(*character)
    }) {
        return Some(format!(
            "A part followed by '@' should not contain the symbol '{symbol}'."
        ));
    }
    if let Some(symbol) = domain
        .chars()
        .find(|character| !character.is_ascii_alphanumeric() && !".-".contains(*character))
    {
        return Some(format!(
            "A part following '@' should not contain the symbol '{symbol}'."
        ));
    }
    let labels_valid = domain.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
    });
    if !labels_valid {
        return Some(format!("'.' is used at a wrong position in '{domain}'."));
    }
    None
}

/// Whether a URL input's value is an absolute URL: a scheme, then for the
/// schemes with hosts a host without spaces or forbidden characters and a
/// numeric port.
fn valid_url(value: &str) -> bool {
    let value = value.trim_matches(|character: char| character <= ' ');
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    let mut characters = scheme.chars();
    let scheme_valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters
            .all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character));
    if !scheme_valid {
        return false;
    }
    let special = ["http", "https", "ws", "wss", "ftp"]
        .iter()
        .any(|special| scheme.eq_ignore_ascii_case(special));
    if !special {
        return true;
    }
    let authority = rest
        .trim_start_matches(['/', '\\'])
        .split(['/', '\\', '?', '#'])
        .next()
        .unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if !host.ends_with(']') || host.starts_with('[') => (host, Some(port)),
        _ => (host_port, None),
    };
    !host.is_empty()
        && !host
            .chars()
            .any(|character| character <= ' ' || "<>^|%\"`{}".contains(character))
        && port.is_none_or(|port| port.bytes().all(|byte| byte.is_ascii_digit()))
}

/// A date or time value with its optional parts filled in, so that values
/// of one type compare as text; `None` when the value is not one of the
/// type.
fn date_key(kind: &str, value: &str) -> Option<String> {
    let value = value.trim();
    let digits = |text: &str, count: usize| {
        text.len() == count && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    let year = |text: &str| text.len() >= 4 && digits(text, text.len()) && text != "0000";
    let in_range = |text: &str, low: u32, high: u32| {
        text.parse::<u32>()
            .is_ok_and(|number| (low..=high).contains(&number))
    };
    let date = |text: &str| -> Option<String> {
        let mut parts = text.splitn(3, '-');
        let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
        let days = match m.parse::<u32>().ok()? {
            2 => {
                let y: u32 = y.parse().ok()?;
                if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        (year(y) && digits(m, 2) && in_range(m, 1, 12) && digits(d, 2) && in_range(d, 1, days))
            .then(|| format!("{y:0>6}-{m}-{d}"))
    };
    let time = |text: &str| -> Option<String> {
        let mut parts = text.splitn(3, ':');
        let (h, m) = (parts.next()?, parts.next()?);
        let (s, fraction) = match parts.next() {
            Some(rest) => rest.split_once('.').unwrap_or((rest, "")),
            None => ("00", ""),
        };
        (digits(h, 2)
            && in_range(h, 0, 23)
            && digits(m, 2)
            && in_range(m, 0, 59)
            && digits(s, 2)
            && in_range(s, 0, 59)
            && fraction.len() <= 3
            && fraction.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("{h}:{m}:{s}.{fraction:0<3}"))
    };
    match kind {
        "date" => date(value),
        "month" => {
            let (y, m) = value.split_once('-')?;
            (year(y) && digits(m, 2) && in_range(m, 1, 12)).then(|| format!("{y:0>6}-{m}"))
        }
        "week" => {
            let (y, w) = value.split_once("-W")?;
            (year(y) && digits(w, 2) && in_range(w, 1, 53)).then(|| format!("{y:0>6}-W{w}"))
        }
        "time" => time(value),
        _ => {
            let (day, clock) = value.split_once(['T', ' '])?;
            Some(format!("{}T{}", date(day)?, time(clock)?))
        }
    }
}

#[cfg(test)]
mod tests;
