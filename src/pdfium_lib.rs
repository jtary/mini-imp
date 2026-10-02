//! Load Pdfium at runtime. The DLL is not linked into the executable.

use std::path::PathBuf;
use std::sync::Mutex;

use pdfium_render::prelude::*;

pub fn open() -> Result<Pdfium, String> {
    static BIND_LOCK: Mutex<()> = Mutex::new(());
    let _guard = BIND_LOCK
        .lock()
        .map_err(|_| "Could not load Pdfium.".to_string())?;
    if let Some(path) = bundled_library() {
        match Pdfium::bind_to_library(&path) {
            Ok(bindings) => return Ok(Pdfium::new(bindings)),
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => {
                return Ok(Pdfium::default());
            }
            Err(PdfiumError::LoadLibraryError(_)) => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    match Pdfium::bind_to_system_library() {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
        Err(error) => Err(format!(
            "{error}. Place a 64-bit pdfium.dll next to OpenImp.exe. PDF export still works without it; page previews and printing need it."
        )),
    }
}

fn bundled_library() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("pdfium.dll"));
        }
    }
    candidates.push(PathBuf::from("pdfium.dll"));
    candidates.into_iter().find(|path| path.is_file())
}

pub fn render_document_page(
    document: &PdfDocument<'_>,
    page_index: usize,
    target_width: i32,
    target_height: Option<i32>,
) -> Result<(u32, u32, Vec<u8>), String> {
    let page = document
        .pages()
        .get(page_index as i32)
        .map_err(|error| error.to_string())?;
    let mut config = PdfRenderConfig::new().set_target_width(target_width.max(1));
    if let Some(height) = target_height {
        config = config.set_target_height(height.max(1));
    }
    let bitmap = page
        .render_with_config(&config)
        .map_err(|error| error.to_string())?;
    let width = bitmap.width().max(0) as u32;
    let height = bitmap.height().max(0) as u32;
    let rgba = bitmap.as_rgba_bytes();
    if width == 0 || height == 0 || rgba.len() != (width as usize) * (height as usize) * 4 {
        return Err("Pdfium returned a bitmap OpenImp could not use.".into());
    }
    Ok((width, height, rgba))
}

/// BGRA pixels, tightly packed, top row first. Suitable for a 32-bit GDI DIB.
///
/// Pdfium's default render config swaps its native BGRA buffer to RGBA. A
/// 32-bit `BI_RGB` bitmap is BGRA, so this render keeps the native order.
pub fn render_page_bgra(
    document: &PdfDocument<'_>,
    page_index: usize,
    target_width: i32,
    target_height: i32,
) -> Result<(i32, i32, Vec<u8>), String> {
    let page = document
        .pages()
        .get(page_index as i32)
        .map_err(|error| error.to_string())?;
    let config = PdfRenderConfig::new()
        .set_target_size(target_width.max(1), target_height.max(1))
        .set_reverse_byte_order(false);
    let bitmap = page
        .render_with_config(&config)
        .map_err(|error| error.to_string())?;
    let width = bitmap.width();
    let height = bitmap.height();
    if width <= 0 || height <= 0 {
        return Err("Pdfium returned an empty page.".into());
    }
    let raw = bitmap.as_raw_bytes();
    let stride = raw.len() / height as usize;
    let row_bytes = width as usize * 4;
    if stride < row_bytes {
        return Err("Pdfium returned a bitmap with a short row.".into());
    }
    let mut tight = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        tight.extend_from_slice(&raw[start..start + row_bytes]);
    }
    Ok((width, height, tight))
}
