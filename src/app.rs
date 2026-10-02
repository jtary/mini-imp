//! OpenImp window: source list, sheet settings, and preview.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{
    Align2, Color32, ColorImage, ComboBox, Context, DragValue, FontId, Frame, Id, LayerId, Margin,
    Order, Pos2, Rect, RichText, ScrollArea, Sense, Shadow, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2, pos2, vec2,
};

use crate::theme::{self, Fluent, Icon};
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
    /// Clockwise turns, in degrees: 0, 90, 180, or 270.
    rotation: i64,
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
        let (width, height) =
            layout::visual_page_size(self.width_pt, self.height_pt, self.rotation);
        let mut size = SourceSize::page(width, height);
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
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
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
                        rotation: 0,
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
                    rotation: 0,
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

    fn export_sources(&self) -> (Vec<SourceFile>, Vec<Option<(f64, f64)>>, Vec<i64>) {
        let mut files = Vec::new();
        let mut targets = Vec::new();
        let mut rotations = Vec::new();
        for index in self.instance_indices() {
            let slot = &self.slots[index];
            files.push(slot.source.clone());
            targets.push(slot.target_points());
            rotations.push(slot.rotation);
        }
        (files, targets, rotations)
    }

    fn export_pdf(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("imposed.pdf")
            .add_filter("PDF", &["pdf"])
            .save_file()
        else {
            return;
        };
        let (files, targets, rotations) = self.export_sources();
        match pdf_export::export_pdf(&files, &targets, &rotations, &self.spec()) {
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
        let (files, targets, rotations) = self.export_sources();
        match pdf_export::export_pdf(&files, &targets, &rotations, &self.spec()) {
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

        self.command_bar(ui);
        self.notice_bar(ui);

        let sources_rect = self.sources_panel(ui, cell_size);
        self.accept_dropped_files(&ctx, sources_rect);

        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(300.0)
            .min_size(260.0)
            .frame(theme::panel_frame(ui))
            .show(ui, |ui| {
                ui.label(RichText::new("Settings").strong().size(18.0));
                ui.add_space(8.0);
                ScrollArea::vertical().show(ui, |ui| {
                    self.settings_ui(ui, &spec, &sizes);
                });
            });

        egui::CentralPanel::default()
            .frame(theme::canvas_frame(ui))
            .show(ui, |ui| match &layout {
                Ok(sheets) => {
                    let fluent = Fluent::from_visuals(ui.visuals());
                    ui.horizontal(|ui| {
                        if theme::icon_button(
                            ui,
                            Icon::ChevronLeft,
                            "Previous",
                            self.sheet_index > 0,
                        )
                        .clicked()
                        {
                            self.sheet_index -= 1;
                        }
                        ui.label(format!(
                            "Sheet {} of {}",
                            self.sheet_index + 1,
                            sheets.len().max(1)
                        ));
                        if theme::icon_button(
                            ui,
                            Icon::ChevronRight,
                            "Next",
                            self.sheet_index + 1 < sheets.len(),
                        )
                        .clicked()
                        {
                            self.sheet_index += 1;
                        }
                        ui.label(
                            RichText::new(format!(
                                "{} × {} pt",
                                spec.width_pt.round(),
                                spec.height_pt.round()
                            ))
                            .color(fluent.text_secondary),
                        );
                    });
                    if self.slots.is_empty() {
                        ui.label(
                            RichText::new(
                                "Add PDFs or images. The grid shows where they will land.",
                            )
                            .color(fluent.text_secondary),
                        );
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
                    ui.colored_label(Fluent::from_visuals(ui.visuals()).error_fg, error);
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
    fn command_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("command")
            .show_separator_line(false)
            .frame(theme::panel_frame(ui))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, Icon::Add, "Add files").clicked() {
                        self.add_files();
                    }
                    let has_sources = !self.slots.is_empty();
                    if theme::quiet_button(ui, Icon::Save, "Export PDF", has_sources).clicked() {
                        self.export_pdf();
                    }
                    let print = theme::quiet_button(
                        ui,
                        Icon::Print,
                        "Print",
                        has_sources && matches!(self.pdfium, PdfiumState::Ready),
                    )
                    .on_disabled_hover_text(match self.pdfium {
                        PdfiumState::Missing => "Place pdfium.dll next to OpenImp.exe to print.",
                        PdfiumState::Checking => "Checking for pdfium.dll…",
                        PdfiumState::Ready => "Add a PDF or an image first.",
                    });
                    if print.clicked() {
                        self.print();
                    }
                });
            });
    }

    fn notice_bar(&mut self, ui: &mut egui::Ui) {
        let pdfium_missing = matches!(self.pdfium, PdfiumState::Missing);
        let notice = match &self.notice {
            Notice::None => None,
            Notice::Info(text) => Some((text.clone(), BannerKind::Info)),
            Notice::Error(text) => Some((text.clone(), BannerKind::Error)),
        };
        if !pdfium_missing && notice.is_none() {
            return;
        }
        egui::Panel::top("notices")
            .show_separator_line(false)
            .frame(theme::panel_frame(ui))
            .show(ui, |ui| {
                if pdfium_missing {
                    banner(
                        ui,
                        BannerKind::Warning,
                        "pdfium.dll not found — PDF previews and printing are off.",
                    );
                }
                if let Some((text, kind)) = notice {
                    if pdfium_missing {
                        ui.add_space(6.0);
                    }
                    banner(ui, kind, &text);
                }
            });
    }

    fn sources_panel(&mut self, ui: &mut egui::Ui, cell_size: Option<(f64, f64)>) -> Rect {
        egui::Panel::left("sources")
            .resizable(true)
            .default_size(340.0)
            .min_size(280.0)
            .frame(theme::panel_frame(ui))
            .show(ui, |ui| {
                let fluent = Fluent::from_visuals(ui.visuals());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Sources").strong().size(18.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(self.slots.len().to_string())
                                .color(fluent.text_secondary),
                        );
                    });
                });
                ui.label(
                    RichText::new(if self.signature_pages >= 4 && self.columns >= 2 {
                        "Signature order runs left to right, then down the sheet."
                    } else {
                        "Pages run left to right, then down the sheet."
                    })
                    .color(fluent.text_secondary)
                    .size(12.0),
                );
                ui.add_space(8.0);
                if self.slots.is_empty() {
                    let mut add = false;
                    empty_sources(ui, &mut add);
                    if add {
                        self.add_files();
                    }
                } else {
                    ScrollArea::vertical().show(ui, |ui| {
                        self.source_cards(ui, cell_size);
                    });
                }
            })
            .response
            .rect
    }

    fn source_cards(&mut self, ui: &mut egui::Ui, cell_size: Option<(f64, f64)>) {
        let mut action = None;
        let slot_count = self.slots.len();
        let unit = self.unit;
        let fluent = Fluent::from_visuals(ui.visuals());
        for (index, slot) in self.slots.iter_mut().enumerate() {
            card(ui, |ui| {
                ui.horizontal_top(|ui| {
                    source_thumb(ui, slot);
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new(&slot.label).strong());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("Instances").color(fluent.text_secondary));
                            ui.add(DragValue::new(&mut slot.copies).range(1..=999).speed(0.1));
                            if theme::icon_button(ui, Icon::ChevronUp, "Move up", index > 0)
                                .clicked()
                            {
                                action = Some(ListAction::Up(index));
                            }
                            if theme::icon_button(
                                ui,
                                Icon::ChevronDown,
                                "Move down",
                                index + 1 < slot_count,
                            )
                            .clicked()
                            {
                                action = Some(ListAction::Down(index));
                            }
                            if theme::icon_button(ui, Icon::Delete, "Remove", true).clicked() {
                                action = Some(ListAction::Remove(index));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [96.0, 0.0],
                                egui::Label::new(
                                    RichText::new("Rotation").color(fluent.text_secondary),
                                ),
                            );
                            if ui
                                .button(format!("{}°", slot.rotation))
                                .on_hover_text("Turn this source clockwise.")
                                .clicked()
                            {
                                slot.rotation = (slot.rotation + 90).rem_euclid(360);
                            }
                        });
                        let turned_on = ui.checkbox(&mut slot.use_target, "Target size").changed()
                            && slot.use_target;
                        if turned_on && slot.target_width_pt <= 1.0 && slot.target_height_pt <= 1.0
                        {
                            if let Some((width, height)) = cell_size {
                                slot.target_width_pt = width;
                                slot.target_height_pt = height;
                            }
                        }
                        if slot.use_target {
                            drag_points(ui, unit, "Width", &mut slot.target_width_pt);
                            drag_points(ui, unit, "Height", &mut slot.target_height_pt);
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
                                        .color(fluent.warning_fg)
                                        .size(12.0),
                                    );
                                }
                            }
                        }
                    });
                });
            });
            ui.add_space(8.0);
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
    }

    fn accept_dropped_files(&mut self, ctx: &Context, sources_rect: Rect) {
        let file_drag = ctx.input(|input| !input.raw.hovered_files.is_empty());
        let pointer = cursor_in_ui(ctx);
        let over_sources = pointer_hits_rect(pointer, sources_rect);
        if file_drag {
            ctx.request_repaint();
            if over_sources {
                let fluent = Fluent::from_visuals(&ctx.global_style().visuals);
                let painter =
                    ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("source-drop")));
                painter.rect_filled(sources_rect, 0.0, fluent.accent.gamma_multiply(0.16));
                painter.rect_stroke(
                    sources_rect,
                    0.0,
                    Stroke::new(2.0, fluent.accent),
                    StrokeKind::Inside,
                );
                painter.text(
                    sources_rect.center(),
                    Align2::CENTER_CENTER,
                    "Drop to add",
                    FontId::proportional(16.0),
                    fluent.text,
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
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, spec: &SheetSpec, sources: &[SourceSize]) {
        let fluent = Fluent::from_visuals(ui.visuals());
        section(ui, "Sheet", |ui| {
            combo_row(ui, "sheet", "Sheet", self.preset.label(), |ui| {
                for preset in SheetPreset::ALL {
                    ui.selectable_value(&mut self.preset, preset, preset.label());
                }
            });
            let orientation = match self.orientation {
                Orientation::Portrait => "Portrait",
                Orientation::Landscape => "Landscape",
            };
            combo_row(ui, "orientation", "Orientation", orientation, |ui| {
                ui.selectable_value(&mut self.orientation, Orientation::Portrait, "Portrait");
                ui.selectable_value(&mut self.orientation, Orientation::Landscape, "Landscape");
            });
            let units = match self.unit {
                DisplayUnit::Millimetres => "Millimetres",
                DisplayUnit::Inches => "Inches",
            };
            combo_row(ui, "units", "Units", units, |ui| {
                ui.selectable_value(&mut self.unit, DisplayUnit::Millimetres, "Millimetres");
                ui.selectable_value(&mut self.unit, DisplayUnit::Inches, "Inches");
            });
            ui.label(
                RichText::new(format!(
                    "Sheet size: {:.1} × {:.1}{}",
                    self.unit.from_pt(spec.width_pt),
                    self.unit.from_pt(spec.height_pt),
                    self.unit.suffix()
                ))
                .color(fluent.text_secondary)
                .size(12.0),
            );
            if self.preset == SheetPreset::Custom {
                ui.add_space(4.0);
                drag_points(ui, self.unit, "Width", &mut self.custom_width_pt);
                drag_points(ui, self.unit, "Height", &mut self.custom_height_pt);
            }
        });

        section(ui, "Grid", |ui| {
            number_row(ui, "Columns", &mut self.columns, 1..=32, 0.05);
            number_row(ui, "Rows", &mut self.rows, 1..=32, 0.05);
            combo_row(ui, "fit", "Fit", self.fit.label(), |ui| {
                ui.selectable_value(&mut self.fit, FitMode::Contain, FitMode::Contain.label());
                ui.selectable_value(&mut self.fit, FitMode::Fill, FitMode::Fill.label());
            });
            number_row(ui, "Signature", &mut self.signature_pages, 0..=128, 0.15).on_hover_text(
                "Pages in one folded section. 0 keeps left-to-right order. 8 on a 2×2 sheet is 8, 1, 6, 3 then 2, 7, 4, 5.",
            );
            ui.checkbox(&mut self.crop_marks, "Crop marks");
            if let Ok((width, height)) = layout::trim_size(spec) {
                ui.label(
                    RichText::new(format!(
                        "Cell size: {:.1} × {:.1}{}",
                        self.unit.from_pt(width),
                        self.unit.from_pt(height),
                        self.unit.suffix()
                    ))
                    .color(fluent.text_secondary)
                    .size(12.0),
                )
                .on_hover_text(
                    "Largest trim that fits in the sheet, margins, and gutters. A target size has to fit inside this.",
                );
            }
            if self.signature_pages >= 4 && self.columns < 2 {
                ui.label(
                    RichText::new("Signature order needs 2 or more columns.")
                        .color(fluent.warning_fg)
                        .size(12.0),
                );
            } else if self.signature_pages >= 4 && self.signature_pages % 4 != 0 {
                let padded = self.signature_pages.div_ceil(4) * 4;
                ui.label(
                    RichText::new(format!(
                        "Signature size is padded to {padded} pages, a multiple of 4."
                    ))
                    .color(fluent.warning_fg)
                    .size(12.0),
                );
            }
        });

        section(ui, "Spacing", |ui| {
            ui.label(RichText::new("Margins").color(fluent.text_secondary));
            drag_points(ui, self.unit, "Top", &mut self.margin_top);
            drag_points(ui, self.unit, "Right", &mut self.margin_right);
            drag_points(ui, self.unit, "Bottom", &mut self.margin_bottom);
            drag_points(ui, self.unit, "Left", &mut self.margin_left);
            ui.add_space(4.0);
            ui.label(RichText::new("Gutters").color(fluent.text_secondary));
            drag_points(ui, self.unit, "Horizontal", &mut self.gutter_x);
            drag_points(ui, self.unit, "Vertical", &mut self.gutter_y);
            ui.add_space(4.0);
            ui.label(RichText::new("Bleed").color(fluent.text_secondary));
            if self.bleed_linked {
                drag_points(ui, self.unit, "All sides", &mut self.bleed_top);
                let bleed = self.bleed_top;
                self.bleed_right = bleed;
                self.bleed_bottom = bleed;
                self.bleed_left = bleed;
            }
            ui.checkbox(&mut self.bleed_linked, "Same on all sides");
            if !self.bleed_linked {
                drag_points(ui, self.unit, "Top", &mut self.bleed_top);
                drag_points(ui, self.unit, "Right", &mut self.bleed_right);
                drag_points(ui, self.unit, "Bottom", &mut self.bleed_bottom);
                drag_points(ui, self.unit, "Left", &mut self.bleed_left);
            }
            if layout::spacing_is_tight(spec, sources) {
                ui.label(
                    RichText::new(
                        "Bleed and crop marks need more margin or gutter, or they will be clipped.",
                    )
                    .color(fluent.warning_fg)
                    .size(12.0),
                );
            }
        });
    }
}

enum BannerKind {
    Info,
    Warning,
    Error,
}

fn banner(ui: &mut egui::Ui, kind: BannerKind, text: &str) {
    let fluent = Fluent::from_visuals(ui.visuals());
    let (background, foreground, icon) = match kind {
        BannerKind::Info => (fluent.info_bg, fluent.info_fg, Icon::Info),
        BannerKind::Warning => (fluent.warning_bg, fluent.warning_fg, Icon::Warning),
        BannerKind::Error => (fluent.error_bg, fluent.error_fg, Icon::Error),
    };
    Frame::new()
        .fill(background)
        .corner_radius(4)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                if theme::icons_ready() {
                    ui.label(theme::icon_label(icon, foreground));
                }
                ui.label(RichText::new(text).color(foreground));
            });
        });
}

fn empty_sources(ui: &mut egui::Ui, add_files: &mut bool) {
    let fluent = Fluent::from_visuals(ui.visuals());
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("No sources yet").strong().size(16.0));
        ui.label(RichText::new("Drop PDFs or images here.").color(fluent.text_secondary));
        ui.add_space(8.0);
        if theme::primary_button(ui, Icon::Add, "Add files").clicked() {
            *add_files = true;
        }
    });
}

fn source_thumb(ui: &mut egui::Ui, slot: &Slot) {
    let size = vec2(52.0, 52.0);
    match &slot.thumb {
        Thumb::Ready(texture) => {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().rect_filled(rect, 4.0, Color32::WHITE);
            paint_rotated_texture(&ui.painter_at(rect), texture.id(), rect, slot.rotation);
        }
        Thumb::Pending => thumb_placeholder(ui, size, "…"),
        Thumb::Unavailable => thumb_placeholder(ui, size, "—"),
    }
}

fn thumb_placeholder(ui: &mut egui::Ui, size: Vec2, text: &str) {
    let fluent = Fluent::from_visuals(ui.visuals());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 4.0, fluent.card);
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, fluent.stroke),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        fluent.text_secondary,
    );
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let fluent = Fluent::from_visuals(ui.visuals());
    Frame::new()
        .fill(fluent.card)
        .stroke(Stroke::new(1.0, fluent.stroke))
        .corner_radius(8)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui);
        });
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let fluent = Fluent::from_visuals(ui.visuals());
    Frame::new()
        .fill(fluent.card)
        .stroke(Stroke::new(1.0, fluent.stroke))
        .corner_radius(8)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.add_space(8.0);
            add(ui);
        });
    ui.add_space(8.0);
}

fn combo_row(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    selected: &str,
    mut show: impl FnMut(&mut egui::Ui),
) {
    let fluent = Fluent::from_visuals(ui.visuals());
    ui.label(RichText::new(label).color(fluent.text_secondary).size(12.0));
    ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(ui.available_width())
        .show_ui(ui, |ui| show(ui));
}

fn number_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    speed: f64,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 0.0], egui::Label::new(label));
        ui.add(DragValue::new(value).range(range).speed(speed))
    })
    .inner
}

fn drag_points(ui: &mut egui::Ui, unit: DisplayUnit, label: &str, points: &mut f64) {
    let mut display = unit.from_pt(*points);
    let decimals = match unit {
        DisplayUnit::Millimetres => 2,
        DisplayUnit::Inches => 3,
    };
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 0.0], egui::Label::new(label));
        let response = ui.add(
            DragValue::new(&mut display)
                .speed(match unit {
                    DisplayUnit::Millimetres => 0.2,
                    DisplayUnit::Inches => 0.01,
                })
                .range(0.0..=unit.from_pt(72.0 * 40.0))
                .max_decimals(decimals)
                .suffix(unit.suffix()),
        );
        if response.changed() {
            *points = unit.to_pt(display).max(0.0);
        }
    });
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
    let hwnd = unsafe {
        WindowFromPoint(POINT {
            x: point.x,
            y: point.y,
        })
    };
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
    let (area, _) = ui.allocate_exact_size(available, Sense::hover());
    let pad = 28.0;
    let fit = Vec2::new(
        (area.width() - pad * 2.0).max(1.0),
        (area.height() - pad * 2.0).max(1.0),
    );
    let scale = (fit.x / spec.width_pt as f32)
        .min(fit.y / spec.height_pt as f32)
        .max(0.01);
    let sheet_size = Vec2::new(spec.width_pt as f32 * scale, spec.height_pt as f32 * scale);
    let rect = Rect::from_center_size(area.center(), sheet_size);
    let painter = ui.painter_at(area);
    let shadow = Shadow {
        offset: [0, 8],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(48),
    };
    painter.add(shadow.as_shape(rect, 2.0));
    painter.rect_filled(rect, 2.0, Color32::WHITE);
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_gray(180)),
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
                let clipped = painter.with_clip_rect(clip_rect);
                paint_rotated_texture(&clipped, texture.id(), content_rect, slot.rotation);
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

fn paint_rotated_texture(
    painter: &egui::Painter,
    texture: egui::TextureId,
    rect: Rect,
    rotation: i64,
) {
    let top_left = rect.left_top();
    let top_right = rect.right_top();
    let bottom_right = rect.right_bottom();
    let bottom_left = rect.left_bottom();
    // Positions follow the source image corners: top-left, top-right, bottom-right, bottom-left.
    let positions = match rotation.rem_euclid(360) {
        90 => [top_right, bottom_right, bottom_left, top_left],
        180 => [bottom_right, bottom_left, top_left, top_right],
        270 => [bottom_left, top_left, top_right, bottom_right],
        _ => [top_left, top_right, bottom_right, bottom_left],
    };
    let uvs = [
        pos2(0.0, 0.0),
        pos2(1.0, 0.0),
        pos2(1.0, 1.0),
        pos2(0.0, 1.0),
    ];
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, uv) in positions.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
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
