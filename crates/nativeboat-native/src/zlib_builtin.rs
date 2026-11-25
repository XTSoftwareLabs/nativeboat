//! The native part of Node's `zlib` module: compression and decompression of
//! raw DEFLATE, zlib and gzip data with the pinned no-std DEFLATE
//! implementation, and CRC-32 checksums. Errors carry zlib's messages.

use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Value, ops};
use miniz_oxide::DataFormat;

#[derive(Clone, Copy)]
enum Format {
    Raw,
    Zlib,
    Gzip,
    Unzip,
}

/// zlib's message for input that ends before its stream does.
const TRUNCATED: &str = "unexpected end of file";

pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    // (mode, data, level, partial): `partial` accepts input that ends early,
    // as zlib does when a stream finishes with a flush other than Z_FINISH.
    let process = realm.function("zlibProcess", 2, false, |realm, _, args| {
        let mode = args
            .first()
            .map(|value| ops::to_string(realm, value))
            .transpose()?
            .map_or_else(String::new, |value| value.to_rust_lossy());
        let compression = match mode.as_str() {
            "deflateRaw" => Some(Format::Raw),
            "deflate" => Some(Format::Zlib),
            "gzip" => Some(Format::Gzip),
            _ => None,
        };
        let format = match mode.as_str() {
            "inflateRaw" | "deflateRaw" => Format::Raw,
            "inflate" | "deflate" => Format::Zlib,
            "gunzip" | "gzip" => Format::Gzip,
            "unzip" => Format::Unzip,
            _ => {
                return Err(realm.throw(ErrorKind::Type, "unsupported zlib operation"));
            }
        };
        let Some(input) = args.get(1).and_then(crate::typed_array_builtin::view_bytes) else {
            return Err(realm.throw(
                ErrorKind::Type,
                "The data argument must be a Buffer or ArrayBufferView",
            ));
        };
        let output = match compression {
            Some(format) => {
                // Node's default level is zlib's, 6; -1 asks for it too.
                let level = match args.get(2) {
                    Some(Value::Number(level)) if (0.0..=9.0).contains(level) => *level as u8,
                    _ => 6,
                };
                compress(format, &input, level)
            }
            None => {
                let partial = args.get(3).is_some_and(ops::to_boolean);
                decompress(format, &input, partial)
                    .map_err(|message| realm.throw(ErrorKind::Error, &message))?
            }
        };
        Ok(Value::Object(crate::typed_array_builtin::new_bytes(
            realm, output, None,
        )))
    });
    exports.define(
        Key::name("zlibProcess"),
        Property::data(Value::Object(process)),
    );
    // (data, value): the CRC-32 of the bytes, continuing from `value`.
    let checksum = realm.function("crc32", 2, false, |realm, _, args| {
        let Some(input) = args
            .first()
            .and_then(crate::typed_array_builtin::view_bytes)
        else {
            return Err(realm.throw(
                ErrorKind::Type,
                "The data argument must be a Buffer or ArrayBufferView",
            ));
        };
        let initial = match args.get(1) {
            Some(Value::Number(value)) => *value as u32,
            _ => 0,
        };
        Ok(Value::Number(f64::from(crc32_from(initial, &input))))
    });
    exports.define(Key::name("crc32"), Property::data(Value::Object(checksum)));
    exports
}

fn compress(format: Format, input: &[u8], level: u8) -> Vec<u8> {
    match format {
        Format::Raw => miniz_oxide::deflate::compress_to_vec(input, level),
        Format::Zlib | Format::Unzip => miniz_oxide::deflate::compress_to_vec_zlib(input, level),
        Format::Gzip => {
            // zlib's header on Windows: no name or time, and the OS byte 10.
            let mut output = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 10];
            output.extend(miniz_oxide::deflate::compress_to_vec(input, level));
            output.extend(crc32(input).to_le_bytes());
            output.extend((input.len() as u32).to_le_bytes());
            output
        }
    }
}

fn decompress(format: Format, input: &[u8], partial: bool) -> Result<Vec<u8>, String> {
    let inflated = |format| {
        let mut output = Vec::new();
        let (_, ended) = inflate(input, format, &mut output)?;
        if ended || partial {
            Ok(output)
        } else {
            Err(TRUNCATED.to_owned())
        }
    };
    match format {
        Format::Raw => inflated(DataFormat::Raw),
        Format::Zlib => inflated(DataFormat::Zlib),
        Format::Gzip => gunzip(input, partial),
        Format::Unzip if input.starts_with(&[0x1f, 0x8b]) => gunzip(input, partial),
        Format::Unzip => inflated(DataFormat::Zlib),
    }
}

/// Decompresses gzip data, member after member as zlib does; bytes after a
/// member that do not start another are ignored.
fn gunzip(input: &[u8], partial: bool) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut rest = input;
    loop {
        let offset = match gzip_header(rest) {
            Ok(offset) => offset,
            Err(message) if message == TRUNCATED && partial => return Ok(output),
            Err(message) => return Err(message),
        };
        let start = output.len();
        let (used, ended) = inflate(&rest[offset..], DataFormat::Raw, &mut output)?;
        let trailer = &rest[offset + used..];
        if !ended || trailer.len() < 8 {
            return if partial {
                Ok(output)
            } else {
                Err(TRUNCATED.to_owned())
            };
        }
        let expected_crc = u32::from_le_bytes(trailer[..4].try_into().expect("four-byte slice"));
        let expected_size = u32::from_le_bytes(trailer[4..8].try_into().expect("four-byte slice"));
        let member = &output[start..];
        if crc32(member) != expected_crc {
            return Err("incorrect data check".to_owned());
        }
        if member.len() as u32 != expected_size {
            return Err("incorrect length check".to_owned());
        }
        rest = &trailer[8..];
        if !rest.starts_with(&[0x1f, 0x8b]) {
            return Ok(output);
        }
    }
}

/// The length of the gzip header at the start of `input`.
fn gzip_header(input: &[u8]) -> Result<usize, String> {
    let truncated = || TRUNCATED.to_owned();
    if input.len() < 10 {
        return if [0x1f, 0x8b][..input.len().min(2)] == input[..input.len().min(2)] {
            Err(truncated())
        } else {
            Err("incorrect header check".to_owned())
        };
    }
    if input[..2] != [0x1f, 0x8b] {
        return Err("incorrect header check".to_owned());
    }
    if input[2] != 8 {
        return Err("unknown compression method".to_owned());
    }
    let flags = input[3];
    if flags & 0xe0 != 0 {
        return Err("unknown header flags set".to_owned());
    }
    let mut offset = 10;
    if flags & 0x04 != 0 {
        if input.len() < offset + 2 {
            return Err(truncated());
        }
        let size = usize::from(u16::from_le_bytes([input[offset], input[offset + 1]]));
        offset += size + 2;
        if offset > input.len() {
            return Err(truncated());
        }
    }
    for flag in [0x08, 0x10] {
        if flags & flag != 0 {
            let Some(end) = input[offset..].iter().position(|byte| *byte == 0) else {
                return Err(truncated());
            };
            offset += end + 1;
        }
    }
    if flags & 0x02 != 0 {
        if input.len() < offset + 2 {
            return Err(truncated());
        }
        let expected_header_crc = u16::from_le_bytes([input[offset], input[offset + 1]]);
        if crc32(&input[..offset]) as u16 != expected_header_crc {
            return Err("header crc mismatch".to_owned());
        }
        offset += 2;
    }
    Ok(offset)
}

/// Inflates raw or zlib-wrapped DEFLATE data into `output`, returning how
/// many input bytes the stream used and whether it ended within them.
fn inflate(
    input: &[u8],
    format: DataFormat,
    output: &mut Vec<u8>,
) -> Result<(usize, bool), String> {
    use miniz_oxide::inflate::stream::InflateState;
    use miniz_oxide::{MZError, MZFlush, MZStatus};
    let mut state = InflateState::new_boxed(format);
    let mut buffer = vec![0; 32 * 1024];
    let mut used = 0;
    loop {
        let result = miniz_oxide::inflate::stream::inflate(
            &mut state,
            &input[used..],
            &mut buffer,
            MZFlush::None,
        );
        used += result.bytes_consumed;
        output.extend_from_slice(&buffer[..result.bytes_written]);
        match result.status {
            Ok(MZStatus::StreamEnd) => return Ok((used, true)),
            Ok(_) if result.bytes_consumed > 0 || result.bytes_written > 0 => {}
            // No progress: the input ran out.
            Ok(_) | Err(MZError::Buf) => return Ok((used, false)),
            Err(_) => return Err("invalid compressed data".to_owned()),
        }
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    crc32_from(0, bytes)
}

/// The CRC-32 of `bytes` continuing a checksum whose value so far is
/// `initial`.
fn crc32_from(initial: u32, bytes: &[u8]) -> u32 {
    let mut crc = !initial;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;
    use flate2::Compression;
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use std::io::Write;

    fn deflate(input: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    }

    fn zlib(input: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    }

    fn gzip(input: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn inflates_raw_zlib_and_gzip_payloads() {
        let message = b"MarkText native updater payload";
        assert_eq!(
            decompress(Format::Raw, &deflate(message), false).unwrap(),
            message
        );
        assert_eq!(
            decompress(Format::Zlib, &zlib(message), false).unwrap(),
            message
        );
        assert_eq!(
            decompress(Format::Gzip, &gzip(message), false).unwrap(),
            message
        );
        assert_eq!(
            decompress(Format::Unzip, &gzip(message), false).unwrap(),
            message
        );
        assert_eq!(
            decompress(Format::Unzip, &zlib(message), false).unwrap(),
            message
        );
    }

    #[test]
    fn compresses_raw_zlib_and_gzip_payloads_other_inflaters_read() {
        use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
        use std::io::Read;
        let message = b"MarkText native export payload, payload, payload";
        let mut text = Vec::new();
        DeflateDecoder::new(&compress(Format::Raw, message, 6)[..])
            .read_to_end(&mut text)
            .unwrap();
        assert_eq!(text, message);
        text.clear();
        ZlibDecoder::new(&compress(Format::Zlib, message, 9)[..])
            .read_to_end(&mut text)
            .unwrap();
        assert_eq!(text, message);
        text.clear();
        let gzipped = compress(Format::Gzip, message, 1);
        assert_eq!(gzipped[..4], [0x1f, 0x8b, 8, 0]);
        GzDecoder::new(&gzipped[..]).read_to_end(&mut text).unwrap();
        assert_eq!(text, message);
        assert_eq!(decompress(Format::Gzip, &gzipped, false).unwrap(), message);
        // The binding takes the mode and an optional level.
        let mut realm = Realm::new();
        let binding = binding(&mut realm);
        let process = ops::get_named(&mut realm, &Value::Object(binding), "zlibProcess").unwrap();
        let input = crate::typed_array_builtin::new_bytes(&realm, message.to_vec(), None);
        let compressed = ops::call(
            &mut realm,
            &process,
            Value::Undefined,
            &[Value::str("gzip"), Value::Object(input), Value::Number(9.0)],
        )
        .unwrap();
        let output = ops::call(
            &mut realm,
            &process,
            Value::Undefined,
            &[Value::str("gunzip"), compressed],
        )
        .unwrap();
        assert_eq!(
            crate::typed_array_builtin::view_bytes(&output).unwrap(),
            message
        );
    }

    #[test]
    fn rejects_bad_headers_truncated_streams_and_checksums() {
        assert!(decompress(Format::Gzip, b"bad", false).is_err());
        assert!(decompress(Format::Raw, b"bad", false).is_err());
        let mut bad_flags = gzip(b"payload");
        bad_flags[3] = 0x20;
        assert!(decompress(Format::Gzip, &bad_flags, false).is_err());
        let mut data = gzip(b"payload");
        data[10] ^= 1;
        assert!(decompress(Format::Gzip, &data, false).is_err());
        let mut bad_crc = gzip(b"payload");
        let crc = bad_crc.len() - 8;
        bad_crc[crc] ^= 1;
        assert!(decompress(Format::Gzip, &bad_crc, false).is_err());
        let mut bad_size = gzip(b"payload");
        let size = bad_size.len() - 4;
        bad_size[size] ^= 1;
        assert!(decompress(Format::Gzip, &bad_size, false).is_err());
    }

    #[test]
    fn gzip_headers_accept_optional_fields_and_reject_truncation() {
        let mut optional = gzip(b"optional fields");
        optional[3] = 0x1e; // FEXTRA, FNAME, FCOMMENT and FHCRC.
        optional.splice(10..10, [2, 0, 0xaa, 0xbb, b'n', 0, b'c', 0, 0, 0]);
        let crc_offset = 10 + 2 + 2 + 2 + 2;
        let header_crc = crc32(&optional[..crc_offset]) as u16;
        optional[crc_offset..crc_offset + 2].copy_from_slice(&header_crc.to_le_bytes());
        assert_eq!(
            decompress(Format::Gzip, &optional, false).unwrap(),
            b"optional fields"
        );
        optional[crc_offset] ^= 1;
        assert!(decompress(Format::Gzip, &optional, false).is_err());

        let mut oversized_extra = gzip(b"payload");
        oversized_extra[3] = 0x04;
        oversized_extra[10] = 0xff;
        oversized_extra[11] = 0xff;
        assert!(decompress(Format::Gzip, &oversized_extra, false).is_err());

        let mut unterminated_name = vec![0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 0];
        unterminated_name.extend_from_slice(&[b'x'; 10]);
        assert!(decompress(Format::Gzip, &unterminated_name, false).is_err());

        let mut short_data = vec![0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 0, 1, 0];
        short_data.extend_from_slice(&[0; 6]);
        assert!(decompress(Format::Gzip, &short_data, false).is_err());
    }

    #[test]
    fn binding_inflates_raw_and_wrapped_streams_and_rejects_invalid_arguments() {
        let realm = &mut Realm::new();
        let exports = Value::Object(binding(realm));
        let function = ops::get_named(realm, &exports, "zlibProcess").unwrap();
        let message = b"MarkText updater differential block";
        let input = crate::typed_array_builtin::new_bytes(realm, deflate(message), None);
        let output = ops::call(
            realm,
            &function,
            Value::Undefined,
            &[Value::str("inflateRaw"), Value::Object(input)],
        )
        .unwrap();
        assert_eq!(
            crate::typed_array_builtin::view_bytes(&output).unwrap(),
            message
        );
        let compressed = crate::typed_array_builtin::new_bytes(realm, zlib(b"wrapped"), None);
        let output = ops::call(
            realm,
            &function,
            Value::Undefined,
            &[Value::str("inflate"), Value::Object(compressed)],
        )
        .unwrap();
        assert_eq!(
            crate::typed_array_builtin::view_bytes(&output).unwrap(),
            b"wrapped"
        );
        let compressed = crate::typed_array_builtin::new_bytes(realm, gzip(b"gzip"), None);
        let output = ops::call(
            realm,
            &function,
            Value::Undefined,
            &[Value::str("gunzip"), Value::Object(compressed)],
        )
        .unwrap();
        assert_eq!(
            crate::typed_array_builtin::view_bytes(&output).unwrap(),
            b"gzip"
        );
        assert!(ops::call(realm, &function, Value::Undefined, &[]).is_err());
        assert!(
            ops::call(
                realm,
                &function,
                Value::Undefined,
                &[Value::str("unknown"), Value::Null],
            )
            .is_err()
        );
        assert!(
            ops::call(
                realm,
                &function,
                Value::Undefined,
                &[Value::str("unzip"), Value::Null],
            )
            .is_err()
        );
    }

    #[test]
    fn truncated_input_yields_what_it_holds_only_when_partial() {
        let message: Vec<u8> = (0..20_000).map(|index| (index * 7 % 251) as u8).collect();
        for (format, data) in [
            (Format::Raw, deflate(&message)),
            (Format::Zlib, zlib(&message)),
            (Format::Gzip, gzip(&message)),
            (Format::Unzip, gzip(&message)),
        ] {
            let cut = &data[..data.len() / 2];
            assert_eq!(decompress(format, cut, false).unwrap_err(), TRUNCATED);
            let output = decompress(format, cut, true).unwrap();
            assert!(!output.is_empty() && message.starts_with(&output));
            // Complete input is the same either way.
            assert_eq!(decompress(format, &data, true).unwrap(), message);
        }
        // A gzip member missing its trailer, or a header cut short.
        let data = gzip(b"trailer");
        assert_eq!(
            decompress(Format::Gzip, &data[..data.len() - 3], true).unwrap(),
            b"trailer"
        );
        assert_eq!(
            decompress(Format::Gzip, &data[..data.len() - 3], false).unwrap_err(),
            TRUNCATED
        );
        assert!(
            decompress(Format::Gzip, &data[..4], true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            decompress(Format::Gzip, &data[..4], false).unwrap_err(),
            TRUNCATED
        );
        // Corrupt data is an error even when partial.
        let mut bad_crc = gzip(b"payload");
        let crc = bad_crc.len() - 8;
        bad_crc[crc] ^= 1;
        assert_eq!(
            decompress(Format::Gzip, &bad_crc, true).unwrap_err(),
            "incorrect data check"
        );
        assert_eq!(
            decompress(Format::Gzip, b"not gzip", true).unwrap_err(),
            "incorrect header check"
        );
    }

    #[test]
    fn gzip_members_concatenate_and_trailing_bytes_are_ignored() {
        let mut data = gzip(b"first ");
        data.extend(gzip(b"second"));
        assert_eq!(
            decompress(Format::Gzip, &data, false).unwrap(),
            b"first second"
        );
        data.extend_from_slice(b"\0\0garbage");
        assert_eq!(
            decompress(Format::Unzip, &data, false).unwrap(),
            b"first second"
        );
    }

    #[test]
    fn crc32_matches_zlib_and_continues_from_a_value() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"hello"), 0x3610_a686);
        assert_eq!(crc32_from(crc32(b"hel"), b"lo"), crc32(b"hello"));
        let realm = &mut Realm::new();
        let exports = Value::Object(binding(realm));
        let function = ops::get_named(realm, &exports, "crc32").unwrap();
        let input = crate::typed_array_builtin::new_bytes(realm, b"lo".to_vec(), None);
        let value = ops::call(
            realm,
            &function,
            Value::Undefined,
            &[
                Value::Object(input),
                Value::Number(f64::from(crc32(b"hel"))),
            ],
        )
        .unwrap();
        assert!(matches!(value, Value::Number(number) if number == f64::from(crc32(b"hello"))));
        assert!(ops::call(realm, &function, Value::Undefined, &[Value::Null]).is_err());
        // The binding's fourth argument asks for partial output.
        let process = ops::get_named(realm, &exports, "zlibProcess").unwrap();
        let data = zlib(b"partial output");
        let cut =
            crate::typed_array_builtin::new_bytes(realm, data[..data.len() - 4].to_vec(), None);
        let output = ops::call(
            realm,
            &process,
            Value::Undefined,
            &[
                Value::str("inflate"),
                Value::Object(cut.clone()),
                Value::Undefined,
                Value::Boolean(true),
            ],
        )
        .unwrap();
        assert_eq!(
            crate::typed_array_builtin::view_bytes(&output).unwrap(),
            b"partial output"
        );
        assert!(
            ops::call(
                realm,
                &process,
                Value::Undefined,
                &[Value::str("inflate"), Value::Object(cut)]
            )
            .is_err()
        );
    }
}
