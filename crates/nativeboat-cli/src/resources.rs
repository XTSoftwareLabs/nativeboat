//! Windows resources of built executables, written by nativeboat itself
//! instead of the Windows SDK's `rc.exe` and `mt.exe`: the application
//! manifest, version information and icon, as a `.res` file that `rust-lld`
//! links into the executable.

use nativeboat_project::Metadata;
use std::path::Path;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;
/// English (United States), with the Unicode code page in version strings.
const LANGUAGE: u16 = 0x0409;

/// The application manifest: Windows 10/11 behavior, themed Common Controls,
/// per-monitor DPI awareness, long paths and the UTF-8 code page.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

/// A `.res` file under construction.
struct Res(Vec<u8>);

impl Res {
    /// Starts with the empty entry that marks a 32-bit resource file.
    fn new() -> Res {
        let mut res = Res(Vec::new());
        res.entry(0, 0, 0, &[]);
        res
    }

    /// Adds a resource with a numeric type and name.
    fn entry(&mut self, kind: u16, name: u16, language: u16, data: &[u8]) {
        let bytes = &mut self.0;
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&32u32.to_le_bytes());
        for ordinal in [kind, name] {
            bytes.extend_from_slice(&0xffffu16.to_le_bytes());
            bytes.extend_from_slice(&ordinal.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes()); // data version
        bytes.extend_from_slice(&0x1030u16.to_le_bytes()); // moveable, pure, discardable
        bytes.extend_from_slice(&language.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes()); // version, characteristics
        bytes.extend_from_slice(data);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
    }
}

/// The numeric components of a version's core (`1.2.3` of `1.2.3-beta.1`):
/// up to four, each the leading digits of its component, missing ones zero.
/// Prerelease and build suffixes do not change the file version.
fn version_numbers(version: &str) -> [u16; 4] {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let mut numbers = [0u16; 4];
    for (number, part) in numbers.iter_mut().zip(core.split('.')) {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        *number = digits
            .parse::<u64>()
            .map_or(0, |value| value.min(0xffff) as u16);
    }
    numbers
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A version-information block: header, key, value and children, each
/// aligned to 32 bits.
fn block(
    key: &str,
    value: &[u8],
    value_length: usize,
    text: bool,
    children: &[Vec<u8>],
) -> Vec<u8> {
    let mut bytes = header(key, value_length, text);
    bytes.extend_from_slice(value);
    for child in children {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        bytes.extend_from_slice(child);
    }
    let length = bytes.len() as u16;
    bytes[..2].copy_from_slice(&length.to_le_bytes());
    bytes
}

fn header(key: &str, value_length: usize, text: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0u16.to_le_bytes()); // length, filled in later
    bytes.extend_from_slice(&(value_length as u16).to_le_bytes());
    bytes.extend_from_slice(&u16::from(text).to_le_bytes());
    bytes.extend_from_slice(&utf16(key));
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    bytes
}

/// `VS_VERSIONINFO` for the application.
fn version_info(metadata: &Metadata) -> Vec<u8> {
    let [a, b, c, d] = version_numbers(&metadata.version);
    let most = (u32::from(a) << 16) | u32::from(b);
    let least = (u32::from(c) << 16) | u32::from(d);
    let mut fixed = Vec::new();
    for word in [
        0xfeef_04bd, // signature
        0x0001_0000, // structure version
        most,
        least,
        most,
        least,
        0x3f,        // flags mask
        0,           // flags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&u32::to_le_bytes(word));
    }
    let description = if metadata.description.is_empty() {
        &metadata.product_name
    } else {
        &metadata.description
    };
    let original = format!("{}.exe", metadata.product_name);
    let strings: Vec<Vec<u8>> = [
        ("FileDescription", description.as_str()),
        ("FileVersion", metadata.version.as_str()),
        ("InternalName", metadata.product_name.as_str()),
        ("OriginalFilename", original.as_str()),
        ("ProductName", metadata.product_name.as_str()),
        ("ProductVersion", metadata.version.as_str()),
    ]
    .into_iter()
    .map(|(key, value)| {
        let value = utf16(value);
        block(key, &value, value.len() / 2, true, &[])
    })
    .collect();
    let table = block("040904b0", &[], 0, true, &strings);
    let string_info = block("StringFileInfo", &[], 0, true, &[table]);
    let translation = block("Translation", &[0x09, 0x04, 0xb0, 0x04], 4, false, &[]);
    let var_info = block("VarFileInfo", &[], 0, true, &[translation]);
    block(
        "VS_VERSION_INFO",
        &fixed,
        fixed.len(),
        false,
        &[string_info, var_info],
    )
}

/// One image of an icon: its directory fields and image data.
struct Image {
    width: u8,
    height: u8,
    colors: u8,
    planes: u16,
    bits: u16,
    data: Vec<u8>,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The images of an `.ico` file.
fn ico_images(data: &[u8]) -> Option<Vec<Image>> {
    if u16_at(data, 0)? != 0 || u16_at(data, 2)? != 1 {
        return None;
    }
    let count = usize::from(u16_at(data, 4)?);
    if count == 0 {
        return None;
    }
    (0..count)
        .map(|index| {
            let entry = 6 + index * 16;
            let size = u32_at(data, entry + 8)? as usize;
            let offset = u32_at(data, entry + 12)? as usize;
            Some(Image {
                width: *data.get(entry)?,
                height: *data.get(entry + 1)?,
                colors: *data.get(entry + 2)?,
                planes: u16_at(data, entry + 4)?,
                bits: u16_at(data, entry + 6)?,
                data: data.get(offset..offset.checked_add(size)?)?.to_vec(),
            })
        })
        .collect()
}

/// A PNG file as a single icon image (Windows reads PNG icon images).
fn png_image(data: &[u8]) -> Option<Image> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") || data.get(12..16)? != b"IHDR" {
        return None;
    }
    let side = |at: usize| {
        let pixels = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?);
        // Directory entries store 256 as 0 and cannot describe larger images.
        match pixels {
            1..=255 => Some(pixels as u8),
            256 => Some(0),
            _ => None,
        }
    };
    Some(Image {
        width: side(16)?,
        height: side(20)?,
        colors: 0,
        planes: 1,
        bits: 32,
        data: data.to_vec(),
    })
}

/// The `.res` data of an executable for the application.
pub(crate) fn resources(metadata: &Metadata) -> Result<Vec<u8>, String> {
    let mut res = Res::new();
    res.entry(RT_MANIFEST, 1, LANGUAGE, MANIFEST.as_bytes());
    res.entry(RT_VERSION, 1, LANGUAGE, &version_info(metadata));
    if let Some(icon) = &metadata.icon {
        let images = icon_images(icon)?;
        let mut group = Vec::new();
        for value in [0u16, 1, images.len() as u16] {
            group.extend_from_slice(&value.to_le_bytes());
        }
        for (index, image) in images.iter().enumerate() {
            let id = index as u16 + 1;
            res.entry(RT_ICON, id, LANGUAGE, &image.data);
            group.extend_from_slice(&[image.width, image.height, image.colors, 0]);
            group.extend_from_slice(&image.planes.to_le_bytes());
            group.extend_from_slice(&image.bits.to_le_bytes());
            group.extend_from_slice(&(image.data.len() as u32).to_le_bytes());
            group.extend_from_slice(&id.to_le_bytes());
        }
        res.entry(RT_GROUP_ICON, 1, LANGUAGE, &group);
    }
    Ok(res.0)
}

fn icon_images(icon: &Path) -> Result<Vec<Image>, String> {
    let data = std::fs::read(icon)
        .map_err(|error| format!("cannot read the icon {}: {error}", icon.display()))?;
    ico_images(&data)
        .or_else(|| png_image(&data).map(|image| vec![image]))
        .ok_or_else(|| {
            format!(
                "{}: the icon must be an .ico file or a PNG image of at most 256x256 pixels",
                icon.display()
            )
        })
}

#[cfg(test)]
mod tests;
