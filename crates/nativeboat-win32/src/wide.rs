//! UTF-16 strings for Win32.

#[allow(unused_imports)]
use crate::prelude::*;
/// A NUL-terminated UTF-16 copy of a string.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// Converts UTF-16 text up to the first NUL.
pub(crate) fn narrow(text: &[u16]) -> String {
    let end = text
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        assert_eq!(wide("hé"), vec![104, 233, 0]);
        assert_eq!(narrow(&wide("hé")), "hé");
        assert_eq!(narrow(&[97, 98]), "ab");
    }
}
