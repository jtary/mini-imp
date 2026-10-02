//! Print an imposed PDF through the Windows print dialog.
//!
//! The dialog returns a printer DC. Each sheet is rasterized with Pdfium at up
//! to 300 DPI and drawn into the printable area so the printer's hardware
//! margins do not clip the sheet.

use std::fs;
use std::path::PathBuf;

use lopdf::{Document, Object};
use pdfium_render::prelude::*;
use windows::Win32::Foundation::{GlobalFree, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteDC, GetDeviceCaps, HDC, HORZRES,
    LOGPIXELSX, LOGPIXELSY, SRCCOPY, StretchDIBits, VERTRES,
};
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC,
    PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
};
use windows::core::w;

use crate::pdfium_lib;

const MAX_RENDER_DPI: i32 = 300;
const MAX_PIXELS: u64 = 40_000_000;

pub enum PrintOutcome {
    Printed,
    Cancelled,
}

#[repr(C)]
struct DocInfoW {
    cb_size: i32,
    lpsz_doc_name: *const u16,
    lpsz_output: *const u16,
    lpsz_datatype: *const u16,
    fw_type: u32,
}

unsafe extern "system" {
    fn StartDocW(hdc: HDC, lpdi: *const DocInfoW) -> i32;
    fn EndDoc(hdc: HDC) -> i32;
    fn StartPage(hdc: HDC) -> i32;
    fn EndPage(hdc: HDC) -> i32;
}

pub fn print_pdf(pdf_bytes: &[u8]) -> Result<PrintOutcome, String> {
    let (sheet_w, sheet_h) = sheet_points(pdf_bytes)?;
    let pdfium = pdfium_lib::open()?;

    let mut dialog = PRINTDLGW {
        lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
        Flags: PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE,
        nCopies: 1,
        nFromPage: 1,
        nToPage: 1,
        nMinPage: 1,
        nMaxPage: 1,
        hwndOwner: HWND::default(),
        ..PRINTDLGW::default()
    };

    let accepted = unsafe { PrintDlgW(&mut dialog) };
    if !accepted.as_bool() {
        free_dialog(&dialog);
        let error = unsafe { CommDlgExtendedError() };
        if error.0 == 0 {
            return Ok(PrintOutcome::Cancelled);
        }
        return Err(format!("The print dialog failed (code {}).", error.0));
    }

    let copies = if dialog.Flags.contains(PD_USEDEVMODECOPIESANDCOLLATE) {
        1
    } else {
        u32::from(dialog.nCopies.max(1))
    };
    let hdc = dialog.hDC;
    let result = print_on_dc(hdc, &pdfium, pdf_bytes, sheet_w, sheet_h, copies);
    free_dialog(&dialog);
    if !hdc.is_invalid() {
        unsafe {
            let _ = DeleteDC(hdc);
        }
    }
    result
}

fn print_on_dc(
    hdc: HDC,
    pdfium: &Pdfium,
    pdf_bytes: &[u8],
    sheet_w_pt: f64,
    sheet_h_pt: f64,
    copies: u32,
) -> Result<PrintOutcome, String> {
    if hdc.is_invalid() {
        return Err("Windows did not return a printer.".into());
    }

    let temp_path = temp_pdf_path();
    fs::write(&temp_path, pdf_bytes)
        .map_err(|error| format!("Could not prepare the print file: {error}"))?;
    let printed = (|| {
        let document = pdfium
            .load_pdf_from_file(&temp_path, None)
            .map_err(|error| format!("Could not open the imposed PDF for printing: {error}"))?;
        let page_count = document.pages().len();
        if page_count <= 0 {
            return Err("The imposed PDF has no pages.".into());
        }

        let dpi_x = unsafe { GetDeviceCaps(Some(hdc), LOGPIXELSX) }.max(72);
        let dpi_y = unsafe { GetDeviceCaps(Some(hdc), LOGPIXELSY) }.max(72);
        let avail_w = unsafe { GetDeviceCaps(Some(hdc), HORZRES) }.max(1);
        let avail_h = unsafe { GetDeviceCaps(Some(hdc), VERTRES) }.max(1);
        let render_dpi = render_dpi(sheet_w_pt, sheet_h_pt, dpi_x.min(dpi_y));
        let pixel_w = ((sheet_w_pt / 72.0) * render_dpi as f64).round().max(1.0) as i32;
        let pixel_h = ((sheet_h_pt / 72.0) * render_dpi as f64).round().max(1.0) as i32;

        let natural_w = (sheet_w_pt / 72.0) * dpi_x as f64;
        let natural_h = (sheet_h_pt / 72.0) * dpi_y as f64;
        let fit = (avail_w as f64 / natural_w).min(avail_h as f64 / natural_h);
        let dest_w = (natural_w * fit).round().max(1.0) as i32;
        let dest_h = (natural_h * fit).round().max(1.0) as i32;
        let dest_x = (avail_w - dest_w) / 2;
        let dest_y = (avail_h - dest_h) / 2;

        let doc_name = w!("OpenImp");
        let info = DocInfoW {
            cb_size: std::mem::size_of::<DocInfoW>() as i32,
            lpsz_doc_name: doc_name.0,
            lpsz_output: std::ptr::null(),
            lpsz_datatype: std::ptr::null(),
            fw_type: 0,
        };
        if unsafe { StartDocW(hdc, &info) } <= 0 {
            return Err("The printer rejected the document.".into());
        }
        let mut job = PrintJob { hdc, open: true };

        for _ in 0..copies {
            for page_index in 0..page_count {
                let (width, height, pixels) =
                    pdfium_lib::render_page_bgra(&document, page_index as usize, pixel_w, pixel_h)?;
                if unsafe { StartPage(hdc) } <= 0 {
                    return Err("The printer rejected a sheet.".into());
                }
                let drawn = blit_bgra(hdc, dest_x, dest_y, dest_w, dest_h, width, height, &pixels);
                if unsafe { EndPage(hdc) } <= 0 || !drawn {
                    return Err("Windows could not draw a sheet.".into());
                }
            }
        }

        if unsafe { EndDoc(hdc) } <= 0 {
            return Err("Windows could not finish the print job.".into());
        }
        job.open = false;
        Ok(PrintOutcome::Printed)
    })();
    let _ = fs::remove_file(&temp_path);
    printed
}

struct PrintJob {
    hdc: HDC,
    open: bool,
}

impl Drop for PrintJob {
    fn drop(&mut self) {
        if self.open {
            unsafe {
                let _ = EndDoc(self.hdc);
            }
        }
    }
}

fn blit_bgra(
    hdc: HDC,
    dest_x: i32,
    dest_y: i32,
    dest_w: i32,
    dest_h: i32,
    src_w: i32,
    src_h: i32,
    pixels: &[u8],
) -> bool {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: src_w,
            biHeight: -src_h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        ..BITMAPINFO::default()
    };
    let drawn = unsafe {
        StretchDIBits(
            hdc,
            dest_x,
            dest_y,
            dest_w,
            dest_h,
            0,
            0,
            src_w,
            src_h,
            Some(pixels.as_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    drawn > 0
}

fn render_dpi(sheet_w_pt: f64, sheet_h_pt: f64, printer_dpi: i32) -> i32 {
    let mut dpi = printer_dpi.clamp(72, MAX_RENDER_DPI);
    loop {
        let width = (sheet_w_pt / 72.0) * dpi as f64;
        let height = (sheet_h_pt / 72.0) * dpi as f64;
        if width > 0.0 && height > 0.0 && (width * height) <= MAX_PIXELS as f64 {
            return dpi;
        }
        if dpi <= 72 {
            return 72;
        }
        dpi = (dpi * 3 / 4).max(72);
    }
}

fn sheet_points(pdf_bytes: &[u8]) -> Result<(f64, f64), String> {
    let doc = Document::load_mem(pdf_bytes)
        .map_err(|error| format!("Could not read the imposed PDF: {error}"))?;
    let pages = doc.get_pages();
    let page_id = pages
        .get(&1)
        .copied()
        .ok_or_else(|| "The imposed PDF has no pages.".to_string())?;
    let page = doc
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| format!("Could not read the sheet size: {error}"))?;
    let media = page
        .get(b"MediaBox")
        .and_then(Object::as_array)
        .map_err(|_| "The imposed sheet has no media box.".to_string())?;
    if media.len() != 4 {
        return Err("The imposed sheet has a broken media box.".into());
    }
    let mut values = [0.0; 4];
    for (index, item) in media.iter().enumerate() {
        values[index] = match item {
            Object::Integer(value) => *value as f64,
            Object::Real(value) => *value as f64,
            _ => return Err("The imposed sheet size is not numeric.".into()),
        };
    }
    let width = (values[2] - values[0]).abs();
    let height = (values[3] - values[1]).abs();
    if width < 1.0 || height < 1.0 {
        return Err("The imposed sheet is empty.".into());
    }
    Ok((width, height))
}

fn temp_pdf_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "openimp-print-{}-{}.pdf",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0)
    ))
}

fn free_dialog(dialog: &PRINTDLGW) {
    free_global(dialog.hDevMode);
    free_global(dialog.hDevNames);
}

fn free_global(handle: HGLOBAL) {
    if !handle.is_invalid() {
        unsafe {
            let _ = GlobalFree(Some(handle));
        }
    }
}
