//! Build an imposed PDF. Source pages stay vectors: each one becomes a Form
//! XObject and is placed with a transform. Images are embedded as JPEG XObjects.

use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::layout::{self, Placement, SheetSpec, SourceSize};
use crate::sources;

#[derive(Debug, Clone)]
pub struct PageGeometry {
    pub width_pt: f64,
    pub height_pt: f64,
}

#[derive(Debug, Clone)]
pub enum SourceFile {
    Pdf { path: PathBuf, page_index: usize },
    Image { path: PathBuf },
}

pub fn read_pdf_pages(path: &Path) -> Result<Vec<PageGeometry>, String> {
    let doc = load_source(path)?;
    let pages = doc.get_pages();
    let mut geometries = Vec::with_capacity(pages.len());
    for page_number in 1..=pages.len() as u32 {
        let page_id = pages[&page_number];
        geometries.push(page_geometry(&doc, page_id)?);
    }
    Ok(geometries)
}

pub fn export_pdf(
    sources: &[SourceFile],
    targets: &[Option<(f64, f64)>],
    spec: &SheetSpec,
) -> Result<Vec<u8>, String> {
    if sources.is_empty() {
        return Err("Add a PDF or an image before exporting.".into());
    }

    let mut pdf_pages: HashMap<PathBuf, Vec<PageGeometry>> = HashMap::new();
    let mut encoded_images: HashMap<PathBuf, (Vec<u8>, u32, u32)> = HashMap::new();
    let mut sizes = Vec::with_capacity(sources.len());
    let mut prepared = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let (target_width_pt, target_height_pt) = match targets.get(index).copied().flatten() {
            Some((width, height)) => (Some(width), Some(height)),
            None => (None, None),
        };
        match source {
            SourceFile::Pdf { path, page_index } => {
                if !pdf_pages.contains_key(path) {
                    pdf_pages.insert(path.clone(), read_pdf_pages(path)?);
                }
                let page = pdf_pages
                    .get(path)
                    .and_then(|pages| pages.get(*page_index))
                    .ok_or_else(|| format!("{} has no page {}.", path.display(), page_index + 1))?;
                sizes.push(SourceSize {
                    width_pt: page.width_pt,
                    height_pt: page.height_pt,
                    target_width_pt,
                    target_height_pt,
                });
                prepared.push(Prepared::Pdf {
                    path: path.clone(),
                    page_index: *page_index,
                });
            }
            SourceFile::Image { path } => {
                if !encoded_images.contains_key(path) {
                    encoded_images.insert(path.clone(), sources::encode_image_jpeg(path)?);
                }
                let (_, width, height) = encoded_images[path];
                sizes.push(SourceSize {
                    width_pt: width as f64,
                    height_pt: height as f64,
                    target_width_pt,
                    target_height_pt,
                });
                prepared.push(Prepared::Image { path: path.clone() });
            }
        }
    }

    let sheets = layout::impose(&sizes, spec)?;
    let mut out = Document::with_version("1.6");
    let mut pdfs: HashMap<PathBuf, Vec<ImportedPage>> = HashMap::new();
    let mut image_objects: HashMap<PathBuf, ObjectId> = HashMap::new();
    let mut image_ids: Vec<Option<ObjectId>> = Vec::with_capacity(prepared.len());

    for item in &prepared {
        match item {
            Prepared::Pdf { path, .. } => {
                image_ids.push(None);
                if !pdfs.contains_key(path) {
                    pdfs.insert(path.clone(), import_pdf(path, &mut out)?);
                }
            }
            Prepared::Image { path } => {
                if !image_objects.contains_key(path) {
                    let (jpeg, width, height) = encoded_images
                        .get(path)
                        .ok_or_else(|| format!("Missing image data for {}.", path.display()))?;
                    let id = embed_jpeg(&mut out, jpeg, *width, *height);
                    image_objects.insert(path.clone(), id);
                }
                image_ids.push(Some(image_objects[path]));
            }
        }
    }

    let pages_id = out.new_object_id();
    let mut page_ids = Vec::with_capacity(sheets.len());
    for sheet in &sheets {
        let page_id = build_sheet_page(
            &mut out, pages_id, spec, sheet, &prepared, &pdfs, &image_ids,
        )?;
        page_ids.push(page_id);
    }

    let kids: Vec<Object> = page_ids.into_iter().map(Object::Reference).collect();
    let count = kids.len() as i64;
    out.set_object(
        pages_id,
        dictionary_of(&[
            ("Type", Object::from("Pages")),
            ("Count", Object::Integer(count)),
            ("Kids", Object::Array(kids)),
        ]),
    );
    let catalog_id = out.add_object(dictionary_of(&[
        ("Type", Object::from("Catalog")),
        ("Pages", Object::Reference(pages_id)),
    ]));
    out.trailer.set("Root", catalog_id);

    let mut bytes = Vec::new();
    out.save_to(&mut bytes)
        .map_err(|error| format!("Could not write the imposed PDF: {error}"))?;
    Ok(bytes)
}

enum Prepared {
    Pdf { path: PathBuf, page_index: usize },
    Image { path: PathBuf },
}

struct ImportedPage {
    form_id: ObjectId,
    width_pt: f64,
    height_pt: f64,
}

enum OwnedResources {
    Ref(ObjectId),
    Inline(Dictionary),
}

struct FormParts {
    content: Vec<u8>,
    resources: Option<OwnedResources>,
    group: Option<Object>,
    llx: f64,
    lly: f64,
    urx: f64,
    ury: f64,
    rotate: i64,
}

fn load_source(path: &Path) -> Result<Document, String> {
    let mut doc = Document::load(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    if doc.is_encrypted() && doc.decrypt("").is_err() {
        return Err(format!(
            "{} is encrypted. Remove the password and try again.",
            path.display()
        ));
    }
    Ok(doc)
}

fn page_geometry(doc: &Document, page_id: ObjectId) -> Result<PageGeometry, String> {
    let (llx, lly, urx, ury) = page_box(doc, page_id)?;
    let rotate = inherited_number(doc, page_id, b"Rotate").unwrap_or(0.0) as i64;
    let (width, height) = layout::visual_page_size((urx - llx).abs(), (ury - lly).abs(), rotate);
    if width <= 0.0 || height <= 0.0 {
        return Err("A PDF page has no printable size.".into());
    }
    Ok(PageGeometry {
        width_pt: width,
        height_pt: height,
    })
}

fn import_pdf(path: &Path, out: &mut Document) -> Result<Vec<ImportedPage>, String> {
    let mut src = load_source(path)?;
    src.renumber_objects_with(out.max_id + 1);
    out.max_id = src.max_id;

    let pages = src.get_pages();
    let mut page_ids = Vec::with_capacity(pages.len());
    for page_number in 1..=pages.len() as u32 {
        page_ids.push(pages[&page_number]);
    }
    if page_ids.is_empty() {
        return Err(format!("{} has no pages.", path.display()));
    }

    let mut parts = Vec::with_capacity(page_ids.len());
    for page_id in &page_ids {
        let (llx, lly, urx, ury) = page_box(&src, *page_id)?;
        let rotate = inherited_number(&src, *page_id, b"Rotate").unwrap_or(0.0) as i64;
        let content = page_content_bytes(&src, *page_id)?;
        let resources = inherited_resources(&src, *page_id);
        let group = src
            .get_object(*page_id)
            .ok()
            .and_then(|object| object.as_dict().ok())
            .and_then(|dict| dict.get(b"Group").ok().cloned());
        parts.push(FormParts {
            content,
            resources,
            group,
            llx,
            lly,
            urx,
            ury,
            rotate,
        });
    }

    let mut skip = HashSet::new();
    if let Some(catalog_id) = src
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|object| object.as_reference().ok())
    {
        skip.insert(catalog_id);
        collect_page_tree(&src, catalog_id, &mut skip);
    }
    for page_id in &page_ids {
        skip.insert(*page_id);
        for content_id in src.get_page_contents(*page_id) {
            skip.insert(content_id);
        }
    }

    for (id, object) in std::mem::take(&mut src.objects) {
        if !skip.contains(&id) {
            out.objects.insert(id, object);
        }
    }

    let mut imported = Vec::with_capacity(parts.len());
    for part in parts {
        let (matrix, width, height) =
            layout::upright_matrix(part.llx, part.lly, part.urx, part.ury, part.rotate);
        let mut form = Dictionary::new();
        form.set("Type", "XObject");
        form.set("Subtype", "Form");
        form.set("FormType", 1_i64);
        form.set(
            "BBox",
            vec![
                number(part.llx),
                number(part.lly),
                number(part.urx),
                number(part.ury),
            ],
        );
        form.set("Matrix", matrix.into_iter().map(number).collect::<Vec<_>>());
        if let Some(resources) = part.resources {
            let resource_id = match resources {
                OwnedResources::Ref(id) => id,
                OwnedResources::Inline(dict) => out.add_object(dict),
            };
            form.set("Resources", resource_id);
        }
        if let Some(group) = part.group {
            form.set("Group", group);
        }
        let mut stream = Stream::new(form, part.content);
        let _ = stream.compress();
        let form_id = out.add_object(stream);
        imported.push(ImportedPage {
            form_id,
            width_pt: width,
            height_pt: height,
        });
    }
    Ok(imported)
}

fn collect_page_tree(doc: &Document, node_id: ObjectId, skip: &mut HashSet<ObjectId>) {
    let Ok(dict) = doc.get_object(node_id).and_then(Object::as_dict) else {
        return;
    };
    let type_name = dict
        .get(b"Type")
        .ok()
        .and_then(|object| object.as_name().ok());
    if type_name == Some(b"Page") {
        return;
    }
    if type_name == Some(b"Catalog") {
        skip.insert(node_id);
        if let Some(pages_id) = dict
            .get(b"Pages")
            .ok()
            .and_then(|object| object.as_reference().ok())
        {
            collect_page_tree(doc, pages_id, skip);
        }
        return;
    }
    let has_kids = dict.get(b"Kids").is_ok();
    if type_name != Some(b"Pages") && !has_kids {
        return;
    }
    skip.insert(node_id);
    let Some(kids) = dict
        .get(b"Kids")
        .ok()
        .and_then(|object| resolve(doc, object).as_array().ok())
    else {
        return;
    };
    for kid in kids {
        if let Ok(kid_id) = kid.as_reference() {
            collect_page_tree(doc, kid_id, skip);
        }
    }
}

fn build_sheet_page(
    out: &mut Document,
    pages_id: ObjectId,
    spec: &SheetSpec,
    sheet: &layout::SheetLayout,
    prepared: &[Prepared],
    pdfs: &HashMap<PathBuf, Vec<ImportedPage>>,
    image_ids: &[Option<ObjectId>],
) -> Result<ObjectId, String> {
    let mut xobjects = Dictionary::new();
    let mut operations = Vec::new();
    for (index, placement) in sheet.placements.iter().enumerate() {
        let name = format!("P{index}");
        match &prepared[placement.source_index] {
            Prepared::Pdf { path, page_index } => {
                let page = pdfs
                    .get(path)
                    .and_then(|pages| pages.get(*page_index))
                    .ok_or_else(|| format!("Missing imported page from {}.", path.display()))?;
                xobjects.set(name.clone(), page.form_id);
                push_placement(
                    &mut operations,
                    &name,
                    placement,
                    page.width_pt,
                    page.height_pt,
                    true,
                );
            }
            Prepared::Image { .. } => {
                let image_id = image_ids
                    .get(placement.source_index)
                    .copied()
                    .flatten()
                    .ok_or_else(|| "Missing an embedded image.".to_string())?;
                xobjects.set(name.clone(), image_id);
                push_placement(&mut operations, &name, placement, 1.0, 1.0, false);
            }
        }
    }
    push_crop_marks(&mut operations, &sheet.crop_marks);

    let bytes = Content { operations }
        .encode()
        .map_err(|error| format!("Could not encode a sheet: {error}"))?;
    let mut content = Stream::new(Dictionary::new(), bytes);
    let _ = content.compress();
    let content_id = out.add_object(content);

    let mut resources = Dictionary::new();
    resources.set("XObject", xobjects);
    resources.set("ProcSet", vec![Object::from("PDF"), Object::from("ImageC")]);

    let page = dictionary_of(&[
        ("Type", Object::from("Page")),
        ("Parent", Object::Reference(pages_id)),
        (
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                number(spec.width_pt),
                number(spec.height_pt),
            ]),
        ),
        ("Contents", Object::Reference(content_id)),
        ("Resources", Object::Dictionary(resources)),
    ]);
    Ok(out.add_object(page))
}

fn push_placement(
    operations: &mut Vec<Operation>,
    name: &str,
    placement: &Placement,
    source_w: f64,
    source_h: f64,
    scale_source: bool,
) {
    let content = placement.content;
    let (scale_x, scale_y) = if scale_source {
        let scale_x = if source_w > 0.0 {
            content.w / source_w
        } else {
            1.0
        };
        let scale_y = if source_h > 0.0 {
            content.h / source_h
        } else {
            1.0
        };
        (scale_x, scale_y)
    } else {
        (content.w, content.h)
    };

    operations.push(Operation::new("q", vec![]));
    let clip = placement.clip;
    operations.push(Operation::new(
        "re",
        vec![
            number(clip.x),
            number(clip.y),
            number(clip.w),
            number(clip.h),
        ],
    ));
    operations.push(Operation::new("W", vec![]));
    operations.push(Operation::new("n", vec![]));
    operations.push(Operation::new(
        "cm",
        vec![
            number(scale_x),
            Object::Integer(0),
            Object::Integer(0),
            number(scale_y),
            number(content.x),
            number(content.y),
        ],
    ));
    operations.push(Operation::new("Do", vec![Object::from(name)]));
    operations.push(Operation::new("Q", vec![]));
}

fn push_crop_marks(operations: &mut Vec<Operation>, marks: &[layout::Segment]) {
    if marks.is_empty() {
        return;
    }
    operations.push(Operation::new("q", vec![]));
    operations.push(Operation::new(
        "w",
        vec![number(layout::CROP_MARK_STROKE_PT)],
    ));
    operations.push(Operation::new(
        "RG",
        vec![Object::Integer(0), Object::Integer(0), Object::Integer(0)],
    ));
    for mark in marks {
        operations.push(Operation::new("m", vec![number(mark.x1), number(mark.y1)]));
        operations.push(Operation::new("l", vec![number(mark.x2), number(mark.y2)]));
        operations.push(Operation::new("S", vec![]));
    }
    operations.push(Operation::new("Q", vec![]));
}

fn embed_jpeg(out: &mut Document, jpeg: &[u8], width: u32, height: u32) -> ObjectId {
    let mut dict = Dictionary::new();
    dict.set("Type", "XObject");
    dict.set("Subtype", "Image");
    dict.set("Width", width as i64);
    dict.set("Height", height as i64);
    dict.set("ColorSpace", "DeviceRGB");
    dict.set("BitsPerComponent", 8_i64);
    dict.set("Filter", "DCTDecode");
    let mut stream = Stream::new(dict, jpeg.to_vec());
    stream.allows_compression = false;
    out.add_object(stream)
}

fn page_box(doc: &Document, page_id: ObjectId) -> Result<(f64, f64, f64, f64), String> {
    let object = if inherited_raw(doc, page_id, b"CropBox").is_some() {
        inherited_raw(doc, page_id, b"CropBox")
    } else {
        inherited_raw(doc, page_id, b"MediaBox")
    };
    let Some(object) = object else {
        return Ok((0.0, 0.0, 595.0, 842.0));
    };
    let array = resolve(doc, object)
        .as_array()
        .map_err(|_| "A page box is not an array.".to_string())?;
    if array.len() != 4 {
        return Err("A page box does not have four numbers.".into());
    }
    let mut values = [0.0; 4];
    for (index, item) in array.iter().enumerate() {
        values[index] = object_f64(resolve(doc, item))
            .ok_or_else(|| "A page box contains a value that is not a number.".to_string())?;
    }
    Ok((values[0], values[1], values[2], values[3]))
}

fn inherited_resources(doc: &Document, page_id: ObjectId) -> Option<OwnedResources> {
    let object = inherited_raw(doc, page_id, b"Resources")?;
    match object {
        Object::Reference(id) => Some(OwnedResources::Ref(*id)),
        Object::Dictionary(dict) => Some(OwnedResources::Inline(dict.clone())),
        _ => None,
    }
}

fn inherited_number(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<f64> {
    let object = inherited_raw(doc, page_id, key)?;
    object_f64(resolve(doc, object))
}

fn inherited_raw<'a>(doc: &'a Document, page_id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut current = page_id;
    for _ in 0..32 {
        let dict = doc.get_object(current).ok()?.as_dict().ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(value);
        }
        current = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn page_content_bytes(doc: &Document, page_id: ObjectId) -> Result<Vec<u8>, String> {
    let mut content = Vec::new();
    for content_id in doc.get_page_contents(page_id) {
        let stream = doc
            .get_object(content_id)
            .and_then(Object::as_stream)
            .map_err(|error| format!("Could not read a page stream: {error}"))?;
        let data = stream
            .decompressed_content()
            .map_err(|error| format!("Could not decode a page stream: {error}"))?;
        content.extend_from_slice(&data);
        content.push(b'\n');
    }
    Ok(content)
}

fn resolve<'a>(doc: &'a Document, object: &'a Object) -> &'a Object {
    doc.dereference(object)
        .map(|(_, resolved)| resolved)
        .unwrap_or(object)
}

fn object_f64(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(*value as f64),
        _ => None,
    }
}

fn number(value: f64) -> Object {
    if value.is_finite() && (value - value.round()).abs() < 1e-4 && value.abs() < 1_000_000.0 {
        Object::Integer(value.round() as i64)
    } else {
        Object::Real(value as f32)
    }
}

fn dictionary_of(entries: &[(&str, Object)]) -> Dictionary {
    let mut dict = Dictionary::new();
    for (key, value) in entries {
        dict.set(*key, value.clone());
    }
    dict
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{FitMode, SheetSpec};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("openimp-{tick}-{}-{name}", std::process::id()))
    }

    fn write_pdf(path: &Path, doc: &mut Document) {
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn blank_pages(sizes: &[(f64, f64)], rotate: Option<i64>) -> Document {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for &(width, height) in sizes {
            let content_id = doc.add_object(Stream::new(Dictionary::new(), b"q\nQ\n".to_vec()));
            let mut page = dictionary_of(&[
                ("Type", Object::from("Page")),
                ("Parent", Object::Reference(pages_id)),
                (
                    "MediaBox",
                    Object::Array(vec![
                        Object::Integer(0),
                        Object::Integer(0),
                        number(width),
                        number(height),
                    ]),
                ),
                ("Contents", Object::Reference(content_id)),
                ("Resources", Object::Dictionary(Dictionary::new())),
            ]);
            if let Some(rotate) = rotate {
                page.set("Rotate", rotate);
            }
            kids.push(Object::Reference(doc.add_object(page)));
        }
        doc.set_object(
            pages_id,
            dictionary_of(&[
                ("Type", Object::from("Pages")),
                ("Count", Object::Integer(kids.len() as i64)),
                ("Kids", Object::Array(kids)),
            ]),
        );
        let catalog_id = doc.add_object(dictionary_of(&[
            ("Type", Object::from("Catalog")),
            ("Pages", Object::Reference(pages_id)),
        ]));
        doc.trailer.set("Root", catalog_id);
        doc
    }

    fn letter_two_up() -> SheetSpec {
        SheetSpec {
            width_pt: 612.0,
            height_pt: 792.0,
            columns: 2,
            rows: 1,
            margin_top: 36.0,
            margin_right: 36.0,
            margin_bottom: 36.0,
            margin_left: 36.0,
            gutter_x: 12.0,
            gutter_y: 0.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: true,
            fit: FitMode::Contain,
            signature_pages: 0,
        }
    }

    #[test]
    fn two_page_pdf_imposes_two_up_on_letter() {
        let path = temp_path("src.pdf");
        let mut src = blank_pages(&[(612.0, 792.0), (612.0, 792.0)], None);
        write_pdf(&path, &mut src);

        let bytes = export_pdf(
            &[
                SourceFile::Pdf {
                    path: path.clone(),
                    page_index: 0,
                },
                SourceFile::Pdf {
                    path: path.clone(),
                    page_index: 1,
                },
            ],
            &[],
            &letter_two_up(),
        )
        .unwrap();
        let _ = fs::remove_file(&path);

        let doc = Document::load_mem(&bytes).unwrap();
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 1);
        let page = doc.get_object(pages[&1]).unwrap().as_dict().unwrap();
        let media = page.get(b"MediaBox").unwrap().as_array().unwrap();
        assert_eq!(media[2].as_i64().unwrap(), 612);
        assert_eq!(media[3].as_i64().unwrap(), 792);
        let content = doc.get_page_content(pages[&1]);
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains(" Do"), "{text}");
        assert!(text.contains(" m\n") || text.contains(" m "), "{text}");
    }

    #[test]
    fn inherited_crop_box_and_rotation_set_visual_size() {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let content_id = doc.add_object(Stream::new(Dictionary::new(), b"q\nQ\n".to_vec()));
        let page_id = doc.add_object(dictionary_of(&[
            ("Type", Object::from("Page")),
            ("Parent", Object::Reference(pages_id)),
            ("Rotate", Object::Integer(90)),
            ("Contents", Object::Reference(content_id)),
            ("Resources", Object::Dictionary(Dictionary::new())),
        ]));
        doc.set_object(
            pages_id,
            dictionary_of(&[
                ("Type", Object::from("Pages")),
                ("Count", Object::Integer(1)),
                (
                    "MediaBox",
                    Object::Array(vec![
                        Object::Integer(0),
                        Object::Integer(0),
                        Object::Integer(300),
                        Object::Integer(400),
                    ]),
                ),
                ("Kids", Object::Array(vec![Object::Reference(page_id)])),
            ]),
        );
        let catalog_id = doc.add_object(dictionary_of(&[
            ("Type", Object::from("Catalog")),
            ("Pages", Object::Reference(pages_id)),
        ]));
        doc.trailer.set("Root", catalog_id);

        let path = temp_path("rotated.pdf");
        write_pdf(&path, &mut doc);
        let pages = read_pdf_pages(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(pages.len(), 1);
        assert!((pages[0].width_pt - 400.0).abs() < 1e-3);
        assert!((pages[0].height_pt - 300.0).abs() < 1e-3);
    }

    #[test]
    fn image_exports_as_one_sheet() {
        let path = temp_path("dot.png");
        let image = image::RgbImage::from_pixel(4, 2, image::Rgb([200, 10, 10]));
        image.save(&path).unwrap();

        let spec = SheetSpec {
            width_pt: 612.0,
            height_pt: 792.0,
            columns: 1,
            rows: 1,
            margin_top: 36.0,
            margin_right: 36.0,
            margin_bottom: 36.0,
            margin_left: 36.0,
            gutter_x: 0.0,
            gutter_y: 0.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: false,
            fit: FitMode::Contain,
            signature_pages: 0,
        };
        let bytes = export_pdf(&[SourceFile::Image { path: path.clone() }], &[], &spec).unwrap();
        let _ = fs::remove_file(&path);
        let doc = Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
    }

    #[test]
    fn print_bitmap_keeps_red_in_the_red_channel() {
        let Ok(pdfium) = crate::pdfium_lib::open() else {
            return;
        };
        let path = temp_path("red.png");
        let image = image::RgbImage::from_pixel(8, 8, image::Rgb([220, 20, 20]));
        image.save(&path).unwrap();
        let spec = SheetSpec {
            width_pt: 200.0,
            height_pt: 200.0,
            columns: 1,
            rows: 1,
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            gutter_x: 0.0,
            gutter_y: 0.0,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: false,
            fit: FitMode::Fill,
            signature_pages: 0,
        };
        let bytes = export_pdf(&[SourceFile::Image { path: path.clone() }], &[], &spec).unwrap();
        let _ = fs::remove_file(&path);
        let pdf_path = temp_path("red.pdf");
        fs::write(&pdf_path, &bytes).unwrap();
        let document = pdfium
            .load_pdf_from_file(&pdf_path, None)
            .expect("imposed PDF opens");
        let (width, height, pixels) =
            crate::pdfium_lib::render_page_bgra(&document, 0, 32, 32).unwrap();
        let _ = fs::remove_file(&pdf_path);
        let offset = ((height as usize / 2) * width as usize + width as usize / 2) * 4;
        let blue = pixels[offset];
        let green = pixels[offset + 1];
        let red = pixels[offset + 2];
        assert!(
            red > 180 && red > blue + 80 && red > green + 80,
            "expected BGRA red, got b={blue} g={green} r={red}"
        );
    }
}
