//! Printing painted pages on a printer, or into a file through a printer
//! such as Windows' "Microsoft Print to PDF". Pages are painted onto the
//! printer's device context as they are onto a window's, so text and
//! shapes stay vector graphics.

use crate::Error;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::wide;
use core::ptr::{null, null_mut};
use nativeboat_ir::NativeTree;
use windows_sys::Win32::Foundation::{GlobalFree, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDCW, DEVMODEW, DM_ORIENTATION, DM_PAPERLENGTH, DM_PAPERSIZE, DM_PAPERWIDTH,
    DMORIENT_LANDSCAPE, DMORIENT_PORTRAIT, DeleteDC, GetDeviceCaps, HDC, LOGPIXELSX, LOGPIXELSY,
    PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH,
};
use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, GetDefaultPrinterW, OpenPrinterW, PRINTER_HANDLE,
};
use windows_sys::Win32::Storage::Xps::{DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::UI::Controls::Dialogs::{
    PD_NOCURRENTPAGE, PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE,
    PRINTDLGEXW, PrintDlgExW, START_PAGE_GENERAL,
};

/// The printer a document is printed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Printer<'a> {
    /// A printer by name, such as "Microsoft Print to PDF".
    Named(&'a str),
    /// The user's default printer.
    Default,
    /// The one the user picks in the print dialog, owned by the window
    /// with this handle.
    Dialog(usize),
}

/// The paper to ask a printer for: its portrait size in millimetres, or
/// the printer's own, and its orientation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Paper {
    /// Width and height in millimetres, portrait.
    pub size: Option<(f32, f32)>,
    /// Whether pages are turned sideways.
    pub landscape: bool,
}

/// A printer's device context, ready for a document.
#[derive(Debug)]
pub struct PrintJob {
    dc: HDC,
    /// The paper as printed, in CSS pixels.
    pub paper: (f32, f32),
    /// Device pixels per CSS pixel.
    scale: f32,
    /// Where the printable area begins on the paper, in device pixels.
    offset: (i32, i32),
}

const DM_OUT_BUFFER: u32 = 2;
const DM_IN_BUFFER: u32 = 8;
/// What `PrintDlgExW` reports when the user chose to print.
const PD_RESULT_PRINT: u32 = 1;

/// The Windows paper size code of a portrait size in millimetres, if one
/// fits it within a millimetre.
fn paper_code(size: (f32, f32)) -> Option<i16> {
    const SIZES: [(f32, f32, i16); 9] = [
        (215.9, 279.4, 1),  // Letter
        (279.4, 431.8, 3),  // Tabloid
        (431.8, 279.4, 4),  // Ledger
        (215.9, 355.6, 5),  // Legal
        (297.0, 420.0, 8),  // A3
        (210.0, 297.0, 9),  // A4
        (148.0, 210.0, 11), // A5
        (420.0, 594.0, 66), // A2
        (105.0, 148.0, 70), // A6
    ];
    SIZES
        .iter()
        .find(|(width, height, _)| (width - size.0).abs() < 1.0 && (height - size.1).abs() < 1.0)
        .map(|(_, _, code)| *code)
}

/// The printer's settings for `paper`, as a DEVMODE buffer.
fn settings(name: &[u16], paper: Paper) -> Option<Vec<u64>> {
    let mut printer = PRINTER_HANDLE::default();
    // SAFETY: the name is null-terminated; the handle is written.
    if unsafe { OpenPrinterW(name.as_ptr(), &mut printer, null()) } == 0 {
        return None;
    }
    // SAFETY: a size query writes nothing.
    let size =
        unsafe { DocumentPropertiesW(null_mut(), printer, name.as_ptr(), null_mut(), null(), 0) };
    let mut buffer = alloc::vec![0u64; (size.max(0) as usize).div_ceil(8)];
    let mode = buffer.as_mut_ptr().cast::<DEVMODEW>();
    // SAFETY: the buffer holds `size` bytes, aligned for DEVMODEW.
    let read = size > 0
        && unsafe {
            DocumentPropertiesW(
                null_mut(),
                printer,
                name.as_ptr(),
                mode,
                null(),
                DM_OUT_BUFFER,
            )
        } >= 0;
    if read {
        // SAFETY: the printer filled the buffer with a DEVMODEW.
        unsafe {
            let fields = &mut (*mode).Anonymous1.Anonymous1;
            fields.dmOrientation = if paper.landscape {
                DMORIENT_LANDSCAPE
            } else {
                DMORIENT_PORTRAIT
            } as i16;
            (*mode).dmFields |= DM_ORIENTATION;
            if let Some(size) = paper.size {
                match paper_code(size) {
                    Some(code) => {
                        fields.dmPaperSize = code;
                        (*mode).dmFields |= DM_PAPERSIZE;
                    }
                    None => {
                        fields.dmPaperWidth = (size.0 * 10.0) as i16;
                        fields.dmPaperLength = (size.1 * 10.0) as i16;
                        (*mode).dmFields |= DM_PAPERWIDTH | DM_PAPERLENGTH;
                    }
                }
            }
            DocumentPropertiesW(
                null_mut(),
                printer,
                name.as_ptr(),
                mode,
                mode,
                DM_IN_BUFFER | DM_OUT_BUFFER,
            );
        }
    }
    // SAFETY: opened above.
    unsafe { ClosePrinter(printer) };
    read.then_some(buffer)
}

/// The user's default printer.
fn default_printer() -> Option<Vec<u16>> {
    let mut length = 0u32;
    // SAFETY: a size query with no buffer.
    unsafe { GetDefaultPrinterW(null_mut(), &mut length) };
    let mut name = alloc::vec![0u16; length as usize];
    // SAFETY: the buffer holds `length` units.
    (length > 0 && unsafe { GetDefaultPrinterW(name.as_mut_ptr(), &mut length) } != 0)
        .then_some(name)
}

/// Opens a printer for a document on `paper`; `None` when the user cancels
/// the print dialog.
pub fn open_printer(printer: Printer<'_>, paper: Paper) -> Result<Option<PrintJob>, Error> {
    let failed = |what: &str| Error {
        message: format!("cannot {what}"),
    };
    // Pages paint with GDI+.
    crate::init()?;
    let dc = match printer {
        Printer::Dialog(owner) => {
            let owner = owner as windows_sys::Win32::Foundation::HWND;
            let mut dialog = PRINTDLGEXW {
                lStructSize: size_of::<PRINTDLGEXW>() as u32,
                hwndOwner: owner,
                Flags: PD_RETURNDC
                    | PD_NOPAGENUMS
                    | PD_NOSELECTION
                    | PD_NOCURRENTPAGE
                    | PD_USEDEVMODECOPIESANDCOLLATE,
                nCopies: 1,
                nStartPage: START_PAGE_GENERAL,
                ..PRINTDLGEXW::default()
            };
            // SAFETY: the structure is initialized and the owner is live.
            let result = unsafe { PrintDlgExW(&mut dialog) };
            // SAFETY: the dialog allocated these, if anything.
            unsafe {
                if !dialog.hDevMode.is_null() {
                    GlobalFree(dialog.hDevMode);
                }
                if !dialog.hDevNames.is_null() {
                    GlobalFree(dialog.hDevNames);
                }
            }
            if result < 0 {
                return Err(failed("show the print dialog"));
            }
            if dialog.dwResultAction != PD_RESULT_PRINT || dialog.hDC.is_null() {
                if !dialog.hDC.is_null() {
                    // SAFETY: the dialog created it for us.
                    unsafe { DeleteDC(dialog.hDC) };
                }
                return Ok(None);
            }
            dialog.hDC
        }
        Printer::Named(_) | Printer::Default => {
            let name = match printer {
                Printer::Named(name) => wide(name),
                _ => default_printer().ok_or_else(|| failed("find a printer"))?,
            };
            let mode = settings(&name, paper);
            let spool = wide("WINSPOOL");
            // SAFETY: the strings are null-terminated; the settings are a
            // DEVMODEW the printer filled, when present.
            let dc = unsafe {
                CreateDCW(
                    spool.as_ptr(),
                    name.as_ptr(),
                    null(),
                    mode.as_ref()
                        .map_or(null(), |mode| mode.as_ptr().cast::<DEVMODEW>()),
                )
            };
            if dc.is_null() {
                return Err(failed("open the printer"));
            }
            dc
        }
    };
    // SAFETY: the DC is live.
    let caps = |index: u32| unsafe { GetDeviceCaps(dc, index as i32) };
    let dpi = (
        caps(LOGPIXELSX).max(1) as f32,
        caps(LOGPIXELSY).max(1) as f32,
    );
    Ok(Some(PrintJob {
        dc,
        paper: (
            caps(PHYSICALWIDTH) as f32 / dpi.0 * 96.0,
            caps(PHYSICALHEIGHT) as f32 / dpi.1 * 96.0,
        ),
        scale: dpi.0 / 96.0,
        offset: (caps(PHYSICALOFFSETX), caps(PHYSICALOFFSETY)),
    }))
}

impl PrintJob {
    /// Prints a document laid out for this paper: `tree` from each of
    /// `breaks` (in CSS pixels down the document), a page of `content` size
    /// inside `margins` (top, right, bottom, left) each. With `output`, the
    /// printer writes the document to that file instead of printing it.
    pub fn print(
        self,
        name: &str,
        output: Option<&str>,
        tree: &NativeTree,
        breaks: &[f32],
        content: (f32, f32),
        margins: [f32; 4],
    ) -> Result<(), Error> {
        let failed = |what: &str| Error {
            message: format!("cannot {what}"),
        };
        let title = wide(name);
        let path = output.map(wide);
        let document = DOCINFOW {
            cbSize: size_of::<DOCINFOW>() as i32,
            lpszDocName: title.as_ptr(),
            lpszOutput: path.as_ref().map_or(null(), |path| path.as_ptr()),
            ..DOCINFOW::default()
        };
        // SAFETY: the DC is live and the strings outlive the call.
        if unsafe { StartDocW(self.dc, &document) } <= 0 {
            return Err(failed("start the document"));
        }
        let scale = self.scale;
        let origin = (
            margins[3] - self.offset.0 as f32 / scale,
            margins[0] - self.offset.1 as f32 / scale,
        );
        let mut page = tree.root.clone();
        page.scroll_x = 0.0;
        for (index, start) in breaks.iter().enumerate() {
            let height = breaks
                .get(index + 1)
                .map_or(content.1, |next| (next - start).min(content.1));
            page.scroll_y = *start;
            let clip = RECT {
                left: (origin.0 * scale).floor() as i32,
                top: (origin.1 * scale).floor() as i32,
                right: ((origin.0 + content.0) * scale).ceil() as i32,
                bottom: ((origin.1 + height) * scale).ceil() as i32,
            };
            // SAFETY: the DC is live within the document.
            if unsafe { StartPage(self.dc) } <= 0 {
                // SAFETY: as above.
                unsafe { EndDoc(self.dc) };
                return Err(failed("start a page"));
            }
            crate::render::paint_surface(self.dc, &page, scale, origin, clip);
            // SAFETY: as above.
            unsafe { EndPage(self.dc) };
        }
        // SAFETY: as above.
        if unsafe { EndDoc(self.dc) } <= 0 {
            return Err(failed("finish the document"));
        }
        Ok(())
    }
}

/// A new path in the user's temporary directory for a printer to write a
/// document to.
pub fn temporary_output(extension: &str) -> String {
    use core::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::Storage::FileSystem::GetTempPathW;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let mut directory = [0u16; 261];
    // SAFETY: the buffer holds 261 units.
    let length = unsafe { GetTempPathW(directory.len() as u32, directory.as_mut_ptr()) } as usize;
    let directory = String::from_utf16_lossy(&directory[..length.min(directory.len())]);
    format!(
        "{directory}nativeboat-print-{}-{}.{extension}",
        // SAFETY: no preconditions.
        unsafe { GetCurrentProcessId() },
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The contents of a file a printer wrote, which is then deleted.
pub fn take_output(path: &str) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, DeleteFileW, FILE_SHARE_READ, GetFileSizeEx, OPEN_EXISTING, ReadFile,
    };
    let name = wide(path);
    // SAFETY: a null-terminated name; the handle is closed below.
    let file = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if file == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut size = 0i64;
    let mut bytes = Vec::new();
    // SAFETY: the handle is open; reads fill the buffer they are given.
    let read = unsafe {
        GetFileSizeEx(file, &mut size) != 0 && {
            bytes.resize(size.max(0) as usize, 0);
            let mut done = 0usize;
            loop {
                let mut count = 0u32;
                let chunk = (bytes.len() - done).min(u32::MAX as usize) as u32;
                if chunk == 0 {
                    break true;
                }
                if ReadFile(
                    file,
                    bytes[done..].as_mut_ptr(),
                    chunk,
                    &mut count,
                    null_mut(),
                ) == 0
                    || count == 0
                {
                    break false;
                }
                done += count as usize;
            }
        }
    };
    // SAFETY: opened above; the name is null-terminated.
    unsafe {
        CloseHandle(file);
        DeleteFileW(name.as_ptr());
    }
    read.then_some(bytes)
}

impl Drop for PrintJob {
    fn drop(&mut self) {
        // SAFETY: the job owns its device context.
        unsafe { DeleteDC(self.dc) };
    }
}

#[cfg(test)]
mod tests;
