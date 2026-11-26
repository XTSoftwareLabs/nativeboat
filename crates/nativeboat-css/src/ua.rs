//! The built-in user agent stylesheet.

use crate::prelude::*;

use crate::parser::{Stylesheet, parse_stylesheet};
use core::sync::atomic::{AtomicPtr, Ordering};

/// Default HTML styles, following the browser defaults applications rely on.
pub const USER_AGENT_CSS: &str = r#"
html, address, blockquote, body, center, dd, details, dialog, div, dl, dt, fieldset,
figcaption, figure, footer, form, h1, h2, h3, h4, h5, h6, header, hgroup, hr, legend,
main, menu, nav, ol, p, pre, search, section, summary, ul, article, aside {
  display: block;
}
head, script, style, link, meta, title, template, base, datalist, noscript, param, area,
[hidden], dialog:not([open]), input[type=hidden] {
  display: none;
}
html { color: CanvasText; }
body { margin: 8px; }
h1 { font-size: 2em; margin: 0.67em 0; font-weight: bold; }
h2 { font-size: 1.5em; margin: 0.83em 0; font-weight: bold; }
h3 { font-size: 1.17em; margin: 1em 0; font-weight: bold; }
h4 { margin: 1.33em 0; font-weight: bold; }
h5 { font-size: 0.83em; margin: 1.67em 0; font-weight: bold; }
h6 { font-size: 0.67em; margin: 2.33em 0; font-weight: bold; }
p, dl, menu, ol, ul, pre { margin: 1em 0; }
blockquote, figure { margin: 1em 40px; }
ol, ul, menu { padding-left: 40px; }
ol ol, ol ul, ul ol, ul ul, menu ol, menu ul { margin-top: 0; margin-bottom: 0; }
ul, menu { list-style-type: disc; }
ol { list-style-type: decimal; }
ul ul, ol ul { list-style-type: circle; }
ul ul ul, ul ol ul, ol ul ul, ol ol ul { list-style-type: square; }
li { display: list-item; }
dd { margin-left: 40px; }
b, strong, th { font-weight: bolder; }
i, em, cite, var, dfn, address { font-style: italic; }
small { font-size: smaller; }
big { font-size: larger; }
sub { vertical-align: sub; font-size: smaller; }
sup { vertical-align: super; font-size: smaller; }
code, kbd, samp, pre, tt { font-family: monospace; }
pre, xmp, plaintext, listing { white-space: pre; }
textarea { white-space: pre-wrap; }
a:any-link { color: LinkText; text-decoration: underline; cursor: pointer; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
center { text-align: center; }
bdi, output { unicode-bidi: isolate; }
bdo { unicode-bidi: isolate-override; }
mark { background-color: Mark; color: MarkText; }
hr { color: gray; border-style: inset; border-width: 1px; margin: 0.5em auto; overflow: hidden; }
table { display: table; border-color: gray; border-spacing: 2px; box-sizing: border-box; }
caption { display: table-caption; text-align: center; }
thead { display: table-header-group; vertical-align: middle; }
tbody { display: table-row-group; vertical-align: middle; }
tfoot { display: table-footer-group; vertical-align: middle; }
tr { display: table-row; vertical-align: inherit; }
td, th { display: table-cell; padding: 1px; vertical-align: inherit; }
th { text-align: center; }
col { display: table-column; }
colgroup { display: table-column-group; }
fieldset { margin: 0 2px; padding: 0.35em 0.75em 0.625em; border: 2px groove ThreeDFace; }
legend { padding: 0 2px; }
img, video, canvas, svg, iframe, embed, object { display: inline-block; }
video { object-fit: contain; }
iframe { border: 2px inset; }
audio:not([controls]) { display: none; }
input, textarea, select, button {
  display: inline-block;
  font-family: Arial;
  font-size: 13.333px;
  font-weight: normal;
  font-style: normal;
  line-height: normal;
  color: FieldText;
  letter-spacing: normal;
  text-transform: none;
  text-indent: 0;
  text-align: start;
  cursor: default;
}
input, textarea { padding: 1px 2px; border: 2px inset ButtonBorder; background-color: Field; cursor: text; }
input[type=checkbox], input[type=radio] { padding: 0; border: none; margin: 3px 3px 3px 4px; background-color: transparent; cursor: default; }
select, button, input[type=button], input[type=submit], input[type=reset], input[type=checkbox], input[type=radio],
input[type=search], input[type=color], input[type=image], input[type=file] { box-sizing: border-box; }
input[type=button], input[type=submit], input[type=reset], button {
  padding: 1px 6px;
  border: 2px outset ButtonBorder;
  background-color: ButtonFace;
  color: ButtonText;
  text-align: center;
}
select { border: 1px solid ButtonBorder; background-color: Field; }
textarea { overflow: auto; font-family: monospace; resize: both; }
option { display: block; }
slot { display: contents; }
label { cursor: default; }
details > summary:first-of-type { display: list-item; list-style: disclosure-closed inside; }
details[open] > summary:first-of-type { list-style-type: disclosure-open; }
dialog { position: absolute; margin: auto; border: solid; padding: 1em; background-color: Canvas; color: CanvasText; }
::placeholder { color: GrayText; }
"#;

/// The parsed user agent stylesheet.
pub fn user_agent_stylesheet() -> &'static Stylesheet {
    static SHEET: AtomicPtr<Stylesheet> = AtomicPtr::new(core::ptr::null_mut());
    let mut sheet = SHEET.load(Ordering::Acquire);
    if sheet.is_null() {
        let candidate = Box::into_raw(Box::new(parse_stylesheet(USER_AGENT_CSS).0));
        match SHEET.compare_exchange(
            core::ptr::null_mut(),
            candidate,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => sheet = candidate,
            Err(existing) => {
                // SAFETY: this thread exclusively owns the candidate because
                // another thread installed a different immutable stylesheet.
                unsafe { drop(Box::from_raw(candidate)) };
                sheet = existing;
            }
        }
    }
    // SAFETY: the installed stylesheet is immutable and intentionally lives
    // for the process lifetime. Atomic publication happens before this read.
    unsafe { &*sheet }
}

#[cfg(test)]
mod tests;
