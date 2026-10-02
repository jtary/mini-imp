//! OpenImp window: source list, sheet settings, and preview.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{
    Align2, Color32, ColorImage, ComboBox, Context, DragValue, FontId, Id, LayerId,
    Order, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
    pos2,
};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, WindowFromPoint};

use crate::layout::{self, DisplayUnit, FitMode, Orientation, SheetPreset, SheetSpec, SourceSize};
use crate::pdf_export::{self, SourceFile};
use crate::print_win::{self, PrintOutcome};
use crate::sources::{self, FileKind, ThumbJob, ThumbUpdate};

const MARGIN_MM: f64 = 12.0;
const GUTTER_MM: f64 = 8.0;

struct Slot {
    id: u64,
    label: String,
    source: SourceFile,
    width_pt: f64,
    height_pt: f64,
    /// How many times this source is placed, in order, before the next source.
    copies: u32,
    use_target: bool,
    target_width_pt: f64,
    target_height_pt: f64,
    thumb: Thumb,
}

impl Slot {
    fn target_points(&self) -> Option<(f64, f64)> {
        if self.use_target {
            Some((self.target_width_pt, self.target_height_pt))
        } else {
            None
        }
    }

    fn source_size(&self) -> SourceSize {
        let mut size = SourceSize::page(self.width_pt, self.height_pt);
        if let Some((width, height)) = self.target_points() {
            size = size.with_target(width, height);
        }
        size
    }
}

enum Thumb {
    Pending,
    Ready(TextureHandle),
    Unavailable,
}

enum PdfiumState {
    Checking,
    Ready,
    Missing,
}

enum Notice {
    None,
    Info(String),
    Error(String),
}

pub struct OpenImpApp {
    slots: Vec<Slot>,
    next_id: u64,
    sheet_index: usize,
    preset: SheetPreset,
    orientation: Orientation,
    custom_width_pt: f64,
    custom_height_pt: f64,
    columns: u32,
    rows: u32,
    margin_top: f64,
    margin_right: f64,
    margin_bottom: f64,
    margin_left: f64,
    gutter_x: f64,
    gutter_y: f64,
    bleed_linked: bool,
    bleed_top: f64,
    bleed_right: f64,
    bleed_bottom: f64,
    bleed_left: f64,
    crop_marks: bool,
    fit: FitMode,
    signature_pages: u32,
    unit: DisplayUnit,
    pdfium: PdfiumState,
    notice: Notice,
    thumb_tx: std::sync::mpsc::Sender<ThumbJob>,
    thumb_rx: std::sync::mpsc::Receiver<ThumbUpdate>,
}

impl OpenImpApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (thumb_tx, thumb_rx) = sources::spawn_thumbnail_worker();
        let margin = MARGIN_MM * layout::PT_PER_MM;
        let gutter = GUTTER_MM * layout::PT_PER_MM;
        Self {
            slots: Vec::new(),
            next_id: 1,
            sheet_index: 0,
            preset: SheetPreset::Letter,
            orientation: Orientation::Portrait,
            custom_width_pt: 612.0,
            custom_height_pt: 792.0,
            columns: 2,
            rows: 1,
            margin_top: margin,
            margin_right: margin,
            margin_bottom: margin,
            margin_left: margin,
            gutter_x: gutter,
            gutter_y: gutter,
            bleed_linked: true,
            bleed_top: 0.0,
            bleed_right: 0.0,
            bleed_bottom: 0.0,
            bleed_left: 0.0,
            crop_marks: true,
            fit: FitMode::Contain,
            signature_pages: 0,
            unit: DisplayUnit::Millimetres,
            pdfium: PdfiumState::Checking,
            notice: Notice::None,
            thumb_tx,
            thumb_rx,
        }
    }

    fn spec(&self) -> SheetSpec {
        let (mut width, mut height) = self
            .preset
            .base_points()
            .unwrap_or((self.custom_width_pt, self.custom_height_pt));
        if self.orientation == Orientation::Landscape {
            std::mem::swap(&mut width, &mut height);
        }
        SheetSpec {
            width_pt: width,
            height_pt: height,
            columns: self.columns.max(1),
            rows: self.rows.max(1),
            margin_top: self.margin_top,
            margin_right: self.margin_right,
            margin_bottom: self.margin_bottom,
            margin_left: self.margin_left,
            gutter_x: self.gutter_x,
            gutter_y: self.gutter_y,
            bleed_top: self.bleed_top,
            bleed_right: self.bleed_right,
            bleed_bottom: self.bleed_bottom,
            bleed_left: self.bleed_left,
            crop_marks: self.crop_marks,
            fit: self.fit,
            signature_pages: self.signature_pages,
        }
    }

    fn poll_thumbs(&mut self, ctx: &Context) {
        while let Ok(update) = self.thumb_rx.try_recv() {
            match update {
                ThumbUpdate::Ready { id, image } => {
                    if let Some(slot) = self.slots.iter_mut().find(|slot| slot.id == id) {
                        let color = ColorImage::from_rgba_unmultiplied(
                            [image.width as usize, image.height as usize],
                            &image.rgba,
                        );
                        slot.thumb = Thumb::Ready(ctx.load_texture(
                            format!("slot-{id}"),
                            color,
                            TextureOptions::LINEAR,
                        ));
                    }
                }
                ThumbUpdate::Failed { id } => {
                    if let Some(slot) = self.slots.iter_mut().find(|slot| slot.id == id) {
                        slot.thumb = Thumb::Unavailable;
                    }
                }
                ThumbUpdate::PdfiumMissing => self.pdfium = PdfiumState::Missing,
                ThumbUpdate::PdfiumReady => {
                    if matches!(self.pdfium, PdfiumState::Checking) {
                        self.pdfium = PdfiumState::Ready;
                    }
                }
            }
        }
        if matches!(self.pdfium, PdfiumState::Checking)
            || self
                .slots
                .iter()
                .any(|slot| matches!(slot.thumb, Thumb::Pending))
        {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn add_files(&mut self) {
        let files = rfd::FileDialog::new()
            .add_filter(
                "PDFs and images",
                &[
                    "pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "gif",
                ],
            )
            .pick_files();
        let Some(files) = files else {
            return;
        };
        for path in files {
            if let Err(error) = self.add_path(path) {
                self.notice = Notice::Error(error);
            }
        }
    }

    fn add_path(&mut self, path: PathBuf) -> Result<(), String> {
        match sources::file_kind(&path) {
            Some(FileKind::Pdf) => {
                let pages = pdf_export::read_pdf_pages(&path)?;
                if pages.is_empty() {
                    return Err(format!("{} has no pages.", path.display()));
                }
                for (index, page) in pages.into_iter().enumerate() {
                    let id = self.alloc_id();
                    let label = format!("{}  ·  page {}", file_name(&path), index + 1);
                    let _ = self.thumb_tx.send(ThumbJob::Pdf {
                        id,
                        path: path.clone(),
                        page_index: index,
                    });
                    self.slots.push(Slot {
                        id,
                        label,
                        source: SourceFile::Pdf {
                            path: path.clone(),
                            page_index: index,
                        },
                        width_pt: page.width_pt,
                        height_pt: page.height_pt,
                        copies: 1,
                        use_target: false,
                        target_width_pt: 0.0,
                        target_height_pt: 0.0,
                        thumb: Thumb::Pending,
                    });
                }
                Ok(())
            }
            Some(FileKind::Image) => {
                let (width, height) = sources::image_pixel_size(&path)?;
                let id = self.alloc_id();
                let _ = self.thumb_tx.send(ThumbJob::Image {
                    id,
                    path: path.clone(),
                });
                self.slots.push(Slot {
                    id,
                    label: file_name(&path),
                    source: SourceFile::Image { path },
                    width_pt: width as f64,
                    height_pt: height as f64,
                    copies: 1,
                    use_target: false,
                    target_width_pt: 0.0,
                    target_height_pt: 0.0,
                    thumb: Thumb::Pending,
                });
                Ok(())
            }
            None => Err(format!(
                "{} is not a PDF or a supported image.",
                path.display()
            )),
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn instance_indices(&self) -> Vec<usize> {
        expand_instances(self.slots.iter().map(|slot| slot.copies))
    }

    fn export_sources(&self) -> (Vec<SourceFile>, Vec<Option<(f64, f64)>>) {
        let mut files = Vec::new();
        let mut targets = Vec::new();
        for index in self.instance_indices() {
            let slot = &self.slots[index];
            files.push(slot.source.clone());
            targets.push(slot.target_points());
        }
        (files, targets)
    }

    fn export_pdf(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("imposed.pdf")
            .add_filter("PDF", &["pdf"])
            .save_file()
        else {
            return;
        };
        let (files, targets) = self.export_sources();
        match pdf_export::export_pdf(&files, &targets, &self.spec()) {
            Ok(bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => {
                    self.notice = Notice::Info(format!("Exported {}", path.display()));
                }
                Err(error) => {
                    self.notice = Notice::Error(format!("Could not save the PDF: {error}"));
                }
            },
            Err(error) => self.notice = Notice::Error(error),
        }
    }

    fn print(&mut self) {
        if !matches!(self.pdfium, PdfiumState::Ready) {
            self.notice = Notice::Error(
                "Place pdfium.dll next to OpenImp.exe to print. Export PDF still works.".into(),
            );
            return;
        }
        let (files, targets) = self.export_sources();
        match pdf_export::export_pdf(&files, &targets, &self.spec()) {
            Ok(bytes) => match print_win::print_pdf(&bytes) {
                Ok(PrintOutcome::Printed) => {
                    self.notice = Notice::Info("Sent the imposed sheets to the printer.".into());
                }
                Ok(PrintOutcome::Cancelled) => {}
                Err(error) => self.notice = Notice::Error(error),
            },
            Err(error) => self.notice = Notice::Error(error),
        }
    }
}

impl eframe::App for OpenImpApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_thumbs(&ctx);
        let spec = self.spec();
        let slot_indices = self.instance_indices();
        let sizes: Vec<SourceSize> = slot_indices
            .iter()
            .map(|&index| self.slots[index].source_size())
            .collect();
        let cell_size = layout::trim_size(&spec).ok();
        let layout = layout::impose(&sizes, &spec);
        let sheet_count = layout.as_ref().map(|sheets| sheets.len()).unwrap_or(0);
        if sheet_count == 0 {
            self.sheet_index = 0;
        } else if self.sheet_index >= sheet_count {
            self.sheet_index = sheet_count - 1;
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("OpenImp");
                ui.separator();
                if ui.button("Add files").clicked() {
                    self.add_files();
                }
                let has_sources = !self.slots.is_empty();
                if ui
                    .add_enabled(has_sources, egui::Button::new("Export PDF"))
                    .clicked()
                {
                    self.export_pdf();
                }
                let print = ui
                    .add_enabled(
                        has_sources && matches!(self.pdfium, PdfiumState::Ready),
                        egui::Button::new("Print"),
                    )
                    .on_disabled_hover_text(match self.pdfium {
                        PdfiumState::Missing => "Place pdfium.dll next to OpenImp.exe to print.",
                        PdfiumState::Checking => "Checking for pdfium.dll…",
                        PdfiumState::Ready => "Add a PDF or an image first.",
                    });
                if print.clicked() {
                    self.print();
                }
                if matches!(self.pdfium, PdfiumState::Missing) {
                    ui.label(
                        RichText::new("pdfium.dll not found — PDF previews and printing are off.")
                            .color(Color32::from_rgb(180, 120, 40)),
                    );
                }
            });
            if let Some((text, color)) = match &self.notice {
                Notice::None => None,
                Notice::Info(text) => Some((text.as_str(), Color32::GRAY)),
                Notice::Error(text) => Some((text.as_str(), Color32::from_rgb(200, 80, 70))),
            } {
                ui.label(RichText::new(text).color(color));
            }
            ui.add_space(4.0);
        });

        egui::Panel::bottom("settings").show(ui, |ui| {
            ui.add_space(4.0);
            self.settings_ui(ui, &spec, &sizes);
            ui.add_space(4.0);
        });

        let sources_panel = egui::Panel::left("sources")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                ui.heading(format!("Sources ({})", self.slots.len()));
                ui.label(if self.signature_pages >= 4 && self.columns >= 2 {
                    "Drop PDFs or images here. Signature order runs left to right, then down the sheet."
                } else {
                    "Drop PDFs or images here. Pages run left to right, then down the sheet."
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut action = None;
                    let slot_count = self.slots.len();
                    let unit = self.unit;
                    for (index, slot) in self.slots.iter_mut().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                                action = Some(ListAction::Up(index));
                            }
                            if ui
                                .add_enabled(index + 1 < slot_count, egui::Button::new("Down"))
                                .clicked()
                            {
                                action = Some(ListAction::Down(index));
                            }
                            if ui.button("Remove").clicked() {
                                action = Some(ListAction::Remove(index));
                            }
                            ui.label("Instances");
                            ui.add(DragValue::new(&mut slot.copies).range(1..=999).speed(0.1));
                        });
                        ui.label(&slot.label);
                        let turned_on = ui.checkbox(&mut slot.use_target, "Target size").changed()
                            && slot.use_target;
                        if turned_on
                            && slot.target_width_pt <= 1.0
                            && slot.target_height_pt <= 1.0
                        {
                            if let Some((width, height)) = cell_size {
                                slot.target_width_pt = width;
                                slot.target_height_pt = height;
                            }
                        }
                        if slot.use_target {
                            ui.horizontal_wrapped(|ui| {
                                drag_points(ui, unit, "Width", &mut slot.target_width_pt);
                                drag_points(ui, unit, "Height", &mut slot.target_height_pt);
                            });
                            if let Some((cell_w, cell_h)) = cell_size {
                                if !layout::target_fits_cell(
                                    cell_w,
                                    cell_h,
                                    slot.target_width_pt,
                                    slot.target_height_pt,
                                ) {
                                    ui.label(
                                        RichText::new(target_warning(
                                            unit,
                                            slot.target_width_pt,
                                            slot.target_height_pt,
                                            cell_w,
                                            cell_h,
                                        ))
                                        .color(Color32::from_rgb(180, 120, 40)),
                                    );
                                }
                            }
                        }
                        ui.separator();
                    }
                    if let Some(action) = action {
                        match action {
                            ListAction::Up(index) => self.slots.swap(index, index - 1),
                            ListAction::Down(index) => self.slots.swap(index, index + 1),
                            ListAction::Remove(index) => {
                                self.slots.remove(index);
                            }
                        }
                    }
                });
            });
        let sources_rect = sources_panel.response.rect;
        let file_drag = ctx.input(|input| !input.raw.hovered_files.is_empty());
        let pointer = cursor_in_ui(&ctx);
        let over_sources = pointer_hits_rect(pointer, sources_rect);
        if file_drag {
            ctx.request_repaint();
            if over_sources {
                let painter =
                    ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("source-drop")));
                painter.rect_filled(
                    sources_rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(90, 150, 220, 48),
                );
                painter.rect_stroke(
                    sources_rect,
                    0.0,
                    Stroke::new(2.0, Color32::from_rgb(120, 170, 230)),
                    StrokeKind::Inside,
                );
                painter.text(
                    sources_rect.center(),
                    Align2::CENTER_CENTER,
                    "Drop to add",
                    FontId::proportional(16.0),
                    Color32::WHITE,
                );
            }
        }
        if over_sources {
            let dropped: Vec<PathBuf> = ctx.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect()
            });
            for path in dropped {
                if let Err(error) = self.add_path(path) {
                    self.notice = Notice::Error(error);
                }
            }
        }

        egui::CentralPanel::default().show(ui, |ui| match &layout {
            Ok(sheets) => {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.sheet_index > 0, egui::Button::new("Previous"))
                        .clicked()
                    {
                        self.sheet_index -= 1;
                    }
                    ui.label(format!(
                        "Sheet {} of {}",
                        self.sheet_index + 1,
                        sheets.len().max(1)
                    ));
                    if ui
                        .add_enabled(
                            self.sheet_index + 1 < sheets.len(),
                            egui::Button::new("Next"),
                        )
                        .clicked()
                    {
                        self.sheet_index += 1;
                    }
                    ui.label(format!(
                        "{} × {} pt",
                        spec.width_pt.round(),
                        spec.height_pt.round()
                    ));
                });
                if self.slots.is_empty() {
                    ui.label("Add PDFs or images. The grid shows where they will land.");
                }
                paint_sheet(
                    ui,
                    &spec,
                    &sheets[self.sheet_index],
                    &self.slots,
                    &slot_indices,
                );
            }
            Err(error) => {
                ui.colored_label(Color32::from_rgb(200, 80, 70), error);
            }
        });
    }
}

enum ListAction {
    Up(usize),
    Down(usize),
    Remove(usize),
}

impl OpenImpApp {
    fn settings_ui(&mut self, ui: &mut egui::Ui, spec: &SheetSpec, sources: &[SourceSize]) {
        ui.horizontal_wrapped(|ui| {
            ComboBox::from_label("Sheet")
                .selected_text(self.preset.label())
                .show_ui(ui, |ui| {
                    for preset in SheetPreset::ALL {
                        ui.selectable_value(&mut self.preset, preset, preset.label());
                    }
                });
            ComboBox::from_label("Orientation")
                .selected_text(match self.orientation {
                    Orientation::Portrait => "Portrait",
                    Orientation::Landscape => "Landscape",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.orientation, Orientation::Portrait, "Portrait");
                    ui.selectable_value(&mut self.orientation, Orientation::Landscape, "Landscape");
                });
            ComboBox::from_label("Units")
                .selected_text(match self.unit {
                    DisplayUnit::Millimetres => "Millimetres",
                    DisplayUnit::Inches => "Inches",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.unit, DisplayUnit::Millimetres, "Millimetres");
                    ui.selectable_value(&mut self.unit, DisplayUnit::Inches, "Inches");
                });
            ComboBox::from_label("Fit")
                .selected_text(self.fit.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.fit, FitMode::Contain, FitMode::Contain.label());
                    ui.selectable_value(&mut self.fit, FitMode::Fill, FitMode::Fill.label());
                });
            ui.checkbox(&mut self.crop_marks, "Crop marks");
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Columns");
            ui.add(DragValue::new(&mut self.columns).range(1..=32).speed(0.05));
            ui.label("Rows");
            ui.add(DragValue::new(&mut self.rows).range(1..=32).speed(0.05));
            ui.label("Signature");
            ui.add(
                DragValue::new(&mut self.signature_pages)
                    .range(0..=128)
                    .speed(0.15),
            )
            .on_hover_text(
                "Pages in one folded section. 0 keeps left-to-right order. 8 on a 2×2 sheet is 8, 1, 6, 3 then 2, 7, 4, 5.",
            );
            ui.label(format!(
                "Sheet size: {:.1} × {:.1}{}",
                self.unit.from_pt(spec.width_pt),
                self.unit.from_pt(spec.height_pt),
                self.unit.suffix()
            ));
            if let Ok((width, height)) = layout::trim_size(spec) {
                ui.label(format!(
                    "Cell size: {:.1} × {:.1}{}",
                    self.unit.from_pt(width),
                    self.unit.from_pt(height),
                    self.unit.suffix()
                ))
                .on_hover_text(
                    "Largest trim that fits in the sheet, margins, and gutters. A target size has to fit inside this.",
                );
            }
        });

        if self.preset == SheetPreset::Custom {
            ui.horizontal_wrapped(|ui| {
                drag_points(ui, self.unit, "Width", &mut self.custom_width_pt);
                drag_points(ui, self.unit, "Height", &mut self.custom_height_pt);
            });
        }

        ui.horizontal_wrapped(|ui| {
            ui.label("Margins");
            drag_points(ui, self.unit, "Top", &mut self.margin_top);
            drag_points(ui, self.unit, "Right", &mut self.margin_right);
            drag_points(ui, self.unit, "Bottom", &mut self.margin_bottom);
            drag_points(ui, self.unit, "Left", &mut self.margin_left);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Gutters");
            drag_points(ui, self.unit, "Horizontal", &mut self.gutter_x);
            drag_points(ui, self.unit, "Vertical", &mut self.gutter_y);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Bleed");
            if self.bleed_linked {
                drag_points(ui, self.unit, "All sides", &mut self.bleed_top);
                let bleed = self.bleed_top;
                self.bleed_right = bleed;
                self.bleed_bottom = bleed;
                self.bleed_left = bleed;
            }
            ui.checkbox(&mut self.bleed_linked, "Same on all sides");
        });
        if !self.bleed_linked {
            ui.horizontal_wrapped(|ui| {
                drag_points(ui, self.unit, "Top", &mut self.bleed_top);
                drag_points(ui, self.unit, "Right", &mut self.bleed_right);
                drag_points(ui, self.unit, "Bottom", &mut self.bleed_bottom);
                drag_points(ui, self.unit, "Left", &mut self.bleed_left);
            });
        }

        if self.signature_pages >= 4 && self.columns < 2 {
            ui.label(
                RichText::new("Signature order needs 2 or more columns.")
                    .color(Color32::from_rgb(180, 120, 40)),
            );
        } else if self.signature_pages >= 4 && self.signature_pages % 4 != 0 {
            let padded = self.signature_pages.div_ceil(4) * 4;
            ui.label(
                RichText::new(format!(
                    "Signature size is padded to {padded} pages, a multiple of 4."
                ))
                .color(Color32::from_rgb(180, 120, 40)),
            );
        }

        if layout::spacing_is_tight(spec, sources) {
            ui.label(
                RichText::new(
                    "Bleed and crop marks need more margin or gutter, or they will be clipped.",
                )
                .color(Color32::from_rgb(180, 120, 40)),
            );
        }
    }
}

fn drag_points(ui: &mut egui::Ui, unit: DisplayUnit, label: &str, points: &mut f64) {
    let mut display = unit.from_pt(*points);
    let decimals = match unit {
        DisplayUnit::Millimetres => 2,
        DisplayUnit::Inches => 3,
    };
    let response = ui.add(
        DragValue::new(&mut display)
            .speed(match unit {
                DisplayUnit::Millimetres => 0.2,
                DisplayUnit::Inches => 0.01,
            })
            .range(0.0..=unit.from_pt(72.0 * 40.0))
            .max_decimals(decimals)
            .suffix(unit.suffix())
            .prefix(format!("{label} ")),
    );
    if response.changed() {
        *points = unit.to_pt(display).max(0.0);
    }
}

fn target_warning(
    unit: DisplayUnit,
    target_w: f64,
    target_h: f64,
    cell_w: f64,
    cell_h: f64,
) -> String {
    if target_w > cell_w + 1e-4 || target_h > cell_h + 1e-4 {
        format!(
            "Target {:.1} × {:.1}{} is larger than the {:.1} × {:.1}{} cell, so this piece stays at the cell size.",
            unit.from_pt(target_w),
            unit.from_pt(target_h),
            unit.suffix(),
            unit.from_pt(cell_w),
            unit.from_pt(cell_h),
            unit.suffix()
        )
    } else {
        format!(
            "Enter a target width and height that fits in the {:.1} × {:.1}{} cell.",
            unit.from_pt(cell_w),
            unit.from_pt(cell_h),
            unit.suffix()
        )
    }
}

fn pointer_hits_rect(pointer: Option<Pos2>, rect: Rect) -> bool {
    pointer.is_some_and(|pos| rect.contains(pos))
}

/// Cursor position in egui points. File drags from Explorer do not update egui's pointer.
fn cursor_in_ui(ctx: &Context) -> Option<Pos2> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    let hwnd = unsafe { WindowFromPoint(POINT { x: point.x, y: point.y }) };
    if hwnd.is_invalid() {
        return None;
    }
    if !unsafe { ScreenToClient(hwnd, &mut point) }.as_bool() {
        return None;
    }
    let origin = ctx.viewport_rect().min;
    let scale = ctx.pixels_per_point();
    Some(pos2(
        origin.x + point.x as f32 / scale,
        origin.y + point.y as f32 / scale,
    ))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_string()
}

fn expand_instances(copies: impl IntoIterator<Item = u32>) -> Vec<usize> {
    let mut indices = Vec::new();
    for (index, count) in copies.into_iter().enumerate() {
        indices.extend(std::iter::repeat_n(index, count.max(1) as usize));
    }
    indices
}

fn paint_sheet(
    ui: &mut egui::Ui,
    spec: &SheetSpec,
    sheet: &layout::SheetLayout,
    slots: &[Slot],
    slot_indices: &[usize],
) {
    let available = ui.available_size();
    let scale = (available.x / spec.width_pt as f32)
        .min(available.y / spec.height_pt as f32)
        .max(0.01)
        * 0.92;
    let sheet_size = Vec2::new(spec.width_pt as f32 * scale, spec.height_pt as f32 * scale);
    let (rect, _) = ui.allocate_exact_size(sheet_size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::WHITE);
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_gray(80)),
        StrokeKind::Outside,
    );

    for (cell_index, placement) in sheet.placements.iter().enumerate() {
        let Some(cell) = sheet.cells.get(cell_index) else {
            continue;
        };
        let cell_rect = pdf_rect_to_screen(rect, spec, *cell);
        let Some(slot_index) = slot_indices.get(placement.source_index).copied() else {
            continue;
        };
        let Some(slot) = slots.get(slot_index) else {
            continue;
        };
        let content_rect = pdf_rect_to_screen(rect, spec, placement.content);
        let clip_rect = pdf_rect_to_screen(rect, spec, placement.clip);
        match &slot.thumb {
            Thumb::Ready(texture) => {
                painter.with_clip_rect(clip_rect).image(
                    texture.id(),
                    content_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            Thumb::Pending => {
                painter.text(
                    cell_rect.center(),
                    Align2::CENTER_CENTER,
                    "…",
                    FontId::proportional(14.0),
                    Color32::GRAY,
                );
            }
            Thumb::Unavailable => {
                painter.text(
                    cell_rect.center(),
                    Align2::CENTER_CENTER,
                    &slot.label,
                    FontId::proportional(11.0),
                    Color32::from_gray(90),
                );
            }
        }
    }

    for (cell_index, cell) in sheet.cells.iter().enumerate() {
        if let Some(placement) = sheet.placements.get(cell_index) {
            if (placement.clip.w - placement.trim.w).abs() > 0.05
                || (placement.clip.h - placement.trim.h).abs() > 0.05
            {
                let clip_rect = pdf_rect_to_screen(rect, spec, placement.clip);
                painter.rect_stroke(
                    clip_rect,
                    0.0,
                    Stroke::new(1.0, Color32::from_rgb(196, 154, 108)),
                    StrokeKind::Inside,
                );
            }
            if (placement.trim.w - cell.w).abs() > 0.05 || (placement.trim.h - cell.h).abs() > 0.05
            {
                let trim_rect = pdf_rect_to_screen(rect, spec, placement.trim);
                painter.rect_stroke(
                    trim_rect,
                    0.0,
                    Stroke::new(1.5, Color32::from_gray(40)),
                    StrokeKind::Inside,
                );
            }
        }
        let cell_rect = pdf_rect_to_screen(rect, spec, *cell);
        painter.rect_stroke(
            cell_rect,
            0.0,
            Stroke::new(1.0, Color32::from_gray(170)),
            StrokeKind::Inside,
        );
    }

    for mark in &sheet.crop_marks {
        let a = pdf_point_to_screen(rect, spec, mark.x1, mark.y1);
        let b = pdf_point_to_screen(rect, spec, mark.x2, mark.y2);
        painter.line_segment([a, b], Stroke::new(1.0, Color32::BLACK));
    }
}

fn pdf_point_to_screen(sheet_rect: Rect, spec: &SheetSpec, x: f64, y: f64) -> Pos2 {
    let sx = sheet_rect.width() / spec.width_pt as f32;
    let sy = sheet_rect.height() / spec.height_pt as f32;
    pos2(
        sheet_rect.min.x + x as f32 * sx,
        sheet_rect.max.y - y as f32 * sy,
    )
}

fn pdf_rect_to_screen(sheet_rect: Rect, spec: &SheetSpec, pdf: layout::Rect) -> Rect {
    let top_left = pdf_point_to_screen(sheet_rect, spec, pdf.x, pdf.y + pdf.h);
    let bottom_right = pdf_point_to_screen(sheet_rect, spec, pdf.x + pdf.w, pdf.y);
    Rect::from_two_pos(top_left, bottom_right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_repeat_each_source_in_order() {
        assert_eq!(expand_instances([1, 3, 2]), vec![0, 1, 1, 1, 2, 2]);
        assert_eq!(expand_instances([0]), vec![0]);
    }

    #[test]
    fn drop_is_accepted_only_on_the_sources_panel() {
        let sources = Rect::from_min_max(pos2(0.0, 40.0), pos2(300.0, 800.0));
        assert!(pointer_hits_rect(Some(pos2(10.0, 50.0)), sources));
        assert!(!pointer_hits_rect(Some(pos2(400.0, 50.0)), sources));
        assert!(!pointer_hits_rect(None, sources));
    }
}
