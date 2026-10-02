//! File types, image decoding, and background thumbnails.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, ImageReader, RgbaImage};
use pdfium_render::prelude::Pdfium;

use crate::pdfium_lib;

pub const THUMB_WIDTH: u32 = 200;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "gif"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    Image,
}

pub fn file_kind(path: &Path) -> Option<FileKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if extension == "pdf" {
        Some(FileKind::Pdf)
    } else if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        Some(FileKind::Image)
    } else {
        None
    }
}

pub fn image_pixel_size(path: &Path) -> Result<(u32, u32), String> {
    ImageReader::open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?
        .with_guessed_format()
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?
        .into_dimensions()
        .map_err(|error| format!("Could not read {}: {error}", path.display()))
}

pub fn encode_image_jpeg(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let image =
        image::open(path).map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let rgba = flatten_on_white(image);
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err(format!("{} is an empty image.", path.display()));
    }
    let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
        .map_err(|error| format!("Could not encode {}: {error}", path.display()))?;
    Ok((jpeg, width, height))
}

pub fn image_thumbnail(path: &Path) -> Result<ThumbPixels, String> {
    let image =
        image::open(path).map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let rgba = flatten_on_white(image);
    let (width, height) = rgba.dimensions();
    let image = if width > THUMB_WIDTH {
        let thumb_h = ((height as f64) * (THUMB_WIDTH as f64) / (width as f64))
            .round()
            .max(1.0) as u32;
        image::imageops::resize(
            &rgba,
            THUMB_WIDTH,
            thumb_h,
            image::imageops::FilterType::Triangle,
        )
    } else {
        rgba
    };
    let (width, height) = image.dimensions();
    Ok(ThumbPixels {
        width,
        height,
        rgba: image.into_raw(),
    })
}

pub struct ThumbPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub enum ThumbJob {
    Image {
        id: u64,
        path: PathBuf,
    },
    Pdf {
        id: u64,
        path: PathBuf,
        page_index: usize,
    },
}

pub enum ThumbUpdate {
    Ready { id: u64, image: ThumbPixels },
    Failed { id: u64 },
    PdfiumReady,
    PdfiumMissing,
}

pub fn spawn_thumbnail_worker() -> (Sender<ThumbJob>, Receiver<ThumbUpdate>) {
    let (job_tx, job_rx) = mpsc::channel();
    let (update_tx, update_rx) = mpsc::channel();
    thread::spawn(move || thumbnail_loop(job_rx, update_tx));
    (job_tx, update_rx)
}

fn thumbnail_loop(jobs: Receiver<ThumbJob>, updates: Sender<ThumbUpdate>) {
    match pdfium_lib::open() {
        Ok(pdfium) => {
            let _ = updates.send(ThumbUpdate::PdfiumReady);
            thumbnail_loop_with_pdfium(&pdfium, jobs, updates);
        }
        Err(_) => {
            let _ = updates.send(ThumbUpdate::PdfiumMissing);
            while let Ok(job) = jobs.recv() {
                let update = match job {
                    ThumbJob::Image { id, path } => match image_thumbnail(&path) {
                        Ok(image) => ThumbUpdate::Ready { id, image },
                        Err(_) => ThumbUpdate::Failed { id },
                    },
                    ThumbJob::Pdf { id, .. } => ThumbUpdate::Failed { id },
                };
                if updates.send(update).is_err() {
                    break;
                }
            }
        }
    }
}

fn thumbnail_loop_with_pdfium(
    pdfium: &Pdfium,
    jobs: Receiver<ThumbJob>,
    updates: Sender<ThumbUpdate>,
) {
    let mut open_path: Option<PathBuf> = None;
    let mut open_doc = None;
    while let Ok(job) = jobs.recv() {
        let update = match job {
            ThumbJob::Image { id, path } => match image_thumbnail(&path) {
                Ok(image) => ThumbUpdate::Ready { id, image },
                Err(_) => ThumbUpdate::Failed { id },
            },
            ThumbJob::Pdf {
                id,
                path,
                page_index,
            } => match pdf_thumbnail(pdfium, &mut open_path, &mut open_doc, &path, page_index) {
                Ok(image) => ThumbUpdate::Ready { id, image },
                Err(_) => ThumbUpdate::Failed { id },
            },
        };
        if updates.send(update).is_err() {
            break;
        }
    }
}

fn pdf_thumbnail<'a>(
    pdfium: &'a Pdfium,
    open_path: &mut Option<PathBuf>,
    open_doc: &mut Option<pdfium_render::prelude::PdfDocument<'a>>,
    path: &Path,
    page_index: usize,
) -> Result<ThumbPixels, String> {
    if open_path.as_deref() != Some(path) {
        *open_doc = None;
        *open_path = None;
        let document = pdfium
            .load_pdf_from_file(path, None)
            .map_err(|error| error.to_string())?;
        *open_path = Some(path.to_path_buf());
        *open_doc = Some(document);
    }
    let document = open_doc
        .as_ref()
        .ok_or_else(|| "No PDF is open.".to_string())?;
    let (width, height, rgba) =
        pdfium_lib::render_document_page(document, page_index, THUMB_WIDTH as i32, None)?;
    Ok(ThumbPixels {
        width,
        height,
        rgba,
    })
}

fn flatten_on_white(image: DynamicImage) -> RgbaImage {
    let source = image.to_rgba8();
    let mut flat = RgbaImage::new(source.width(), source.height());
    for (x, y, pixel) in source.enumerate_pixels() {
        let alpha = pixel[3] as u16;
        let blend =
            |channel: u8| -> u8 { ((channel as u16 * alpha + 255 * (255 - alpha)) / 255) as u8 };
        flat.put_pixel(
            x,
            y,
            image::Rgba([blend(pixel[0]), blend(pixel[1]), blend(pixel[2]), 255]),
        );
    }
    flat
}
