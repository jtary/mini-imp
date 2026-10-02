//! Windows 11 Fluent colors, Segoe UI, and command-bar buttons.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{
    Atoms, Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, Margin, Response, RichText, Stroke, Style, TextStyle, Theme, ThemePreference, Ui,
    Visuals, vec2,
};

const ICON_FAMILY: &str = "FluentIcons";
const FONTS_DIR: &str = r"C:\Windows\Fonts";

static ICONS_READY: AtomicBool = AtomicBool::new(false);

pub fn icons_ready() -> bool {
    ICONS_READY.load(Ordering::Relaxed)
}

/// Fluent palette for the active light or dark theme.
#[derive(Clone, Copy)]
pub struct Fluent {
    pub page: Color32,
    pub card: Color32,
    pub canvas: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    pub on_accent: Color32,
    pub warning_fg: Color32,
    pub warning_bg: Color32,
    pub error_fg: Color32,
    pub error_bg: Color32,
    pub info_fg: Color32,
    pub info_bg: Color32,
    control_hover: Color32,
    control_pressed: Color32,
}

impl Fluent {
    pub fn from_visuals(visuals: &Visuals) -> Self {
        if visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    fn light() -> Self {
        Self {
            page: Color32::from_rgb(243, 243, 243),
            card: Color32::WHITE,
            canvas: Color32::from_rgb(232, 232, 232),
            stroke: Color32::from_rgb(225, 225, 225),
            text: Color32::from_rgb(26, 26, 26),
            text_secondary: Color32::from_rgb(93, 93, 93),
            accent: Color32::from_rgb(0, 95, 184),
            accent_hover: Color32::from_rgb(0, 84, 166),
            accent_pressed: Color32::from_rgb(0, 72, 145),
            on_accent: Color32::WHITE,
            warning_fg: Color32::from_rgb(138, 90, 0),
            warning_bg: Color32::from_rgb(255, 244, 206),
            error_fg: Color32::from_rgb(196, 43, 28),
            error_bg: Color32::from_rgb(253, 231, 233),
            info_fg: Color32::from_rgb(0, 78, 140),
            info_bg: Color32::from_rgb(235, 244, 252),
            control_hover: Color32::from_rgb(245, 245, 245),
            control_pressed: Color32::from_rgb(235, 235, 235),
        }
    }

    fn dark() -> Self {
        Self {
            page: Color32::from_rgb(32, 32, 32),
            card: Color32::from_rgb(44, 44, 44),
            canvas: Color32::from_rgb(26, 26, 26),
            stroke: Color32::from_rgb(59, 59, 59),
            text: Color32::WHITE,
            text_secondary: Color32::from_rgb(180, 180, 180),
            accent: Color32::from_rgb(96, 205, 255),
            accent_hover: Color32::from_rgb(122, 212, 255),
            accent_pressed: Color32::from_rgb(76, 194, 255),
            on_accent: Color32::BLACK,
            warning_fg: Color32::from_rgb(255, 224, 138),
            warning_bg: Color32::from_rgb(67, 53, 25),
            error_fg: Color32::from_rgb(255, 153, 164),
            error_bg: Color32::from_rgb(68, 39, 38),
            info_fg: Color32::from_rgb(156, 215, 255),
            info_bg: Color32::from_rgb(26, 51, 68),
            control_hover: Color32::from_rgb(56, 56, 56),
            control_pressed: Color32::from_rgb(42, 42, 42),
        }
    }
}

pub fn apply(ctx: &Context) {
    install_fonts(ctx);
    ctx.set_theme(ThemePreference::System);
    ctx.style_mut_of(Theme::Light, |style| style_fluent(style, &Fluent::light()));
    ctx.style_mut_of(Theme::Dark, |style| style_fluent(style, &Fluent::dark()));
}

pub fn panel_frame(ui: &Ui) -> Frame {
    let fluent = Fluent::from_visuals(ui.visuals());
    Frame::side_top_panel(ui.style())
        .fill(fluent.page)
        .inner_margin(Margin::symmetric(12, 12))
}

pub fn canvas_frame(ui: &Ui) -> Frame {
    let fluent = Fluent::from_visuals(ui.visuals());
    Frame::central_panel(ui.style())
        .fill(fluent.canvas)
        .inner_margin(Margin::same(16))
}

pub fn primary_button(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    let fluent = Fluent::from_visuals(ui.visuals());
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        for (widget, fill) in [
            (&mut visuals.widgets.inactive, fluent.accent),
            (&mut visuals.widgets.hovered, fluent.accent_hover),
            (&mut visuals.widgets.active, fluent.accent_pressed),
        ] {
            widget.weak_bg_fill = fill;
            widget.bg_fill = fill;
            widget.bg_stroke = Stroke::NONE;
            widget.fg_stroke.color = fluent.on_accent;
        }
        ui.add(Button::new(command_atoms(icon, label)).min_size(vec2(0.0, 32.0)))
    })
    .inner
}

pub fn quiet_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        Button::new(command_atoms(icon, label)).min_size(vec2(0.0, 32.0)),
    )
}

pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, enabled: bool) -> Response {
    let button = if icons_ready() {
        Button::new(RichText::new(icon.glyph()).font(icon_font()))
    } else {
        Button::new(icon.fallback())
    };
    ui.add_enabled(
        enabled,
        button.min_size(vec2(32.0, 32.0)).frame_when_inactive(false),
    )
    .on_hover_text(tip)
    .on_disabled_hover_text(tip)
}

#[derive(Clone, Copy)]
pub enum Icon {
    Add,
    Save,
    Print,
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Delete,
    Warning,
    Info,
    Error,
}

impl Icon {
    fn glyph(self) -> &'static str {
        match self {
            Self::Add => "\u{E710}",
            Self::Save => "\u{E74E}",
            Self::Print => "\u{E749}",
            Self::ChevronUp => "\u{E70E}",
            Self::ChevronDown => "\u{E70D}",
            Self::ChevronLeft => "\u{E76B}",
            Self::ChevronRight => "\u{E76C}",
            Self::Delete => "\u{E74D}",
            Self::Warning => "\u{E7BA}",
            Self::Info => "\u{E946}",
            Self::Error => "\u{E783}",
        }
    }

    fn fallback(self) -> &'static str {
        match self {
            Self::Add => "Add",
            Self::Save => "Save",
            Self::Print => "Print",
            Self::ChevronUp => "Up",
            Self::ChevronDown => "Down",
            Self::ChevronLeft => "Previous",
            Self::ChevronRight => "Next",
            Self::Delete => "Remove",
            Self::Warning | Self::Error => "!",
            Self::Info => "i",
        }
    }
}

fn command_atoms(icon: Icon, label: &str) -> Atoms<'static> {
    let mut atoms = Atoms::default();
    if icons_ready() {
        atoms.push_right(RichText::new(icon.glyph()).font(icon_font()));
    }
    atoms.push_right(label.to_owned());
    atoms
}

pub fn icon_label(icon: Icon, color: Color32) -> RichText {
    RichText::new(icon.glyph()).font(icon_font()).color(color)
}

fn icon_font() -> FontId {
    FontId::new(16.0, FontFamily::Name(ICON_FAMILY.into()))
}

fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = read_font("segoeui.ttf") {
        fonts
            .font_data
            .insert("Segoe UI".to_owned(), Arc::new(FontData::from_owned(bytes)));
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "Segoe UI".to_owned());
        }
    }
    let icon_file = ["SegoeIcons.ttf", "segmdl2.ttf"]
        .into_iter()
        .find_map(read_font);
    if let Some(bytes) = icon_file {
        fonts.font_data.insert(
            "Segoe Fluent Icons".to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        fonts.families.insert(
            FontFamily::Name(ICON_FAMILY.into()),
            vec!["Segoe Fluent Icons".to_owned()],
        );
        ICONS_READY.store(true, Ordering::Relaxed);
    }
    ctx.set_fonts(fonts);
}

fn read_font(file_name: &str) -> Option<Vec<u8>> {
    std::fs::read(Path::new(FONTS_DIR).join(file_name)).ok()
}

fn style_fluent(style: &mut Style, fluent: &Fluent) {
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(12.0, 8.0);
    style.spacing.interact_size = vec2(32.0, 32.0);
    style.spacing.window_margin = Margin::same(12);
    if let Some(font) = style.text_styles.get_mut(&TextStyle::Small) {
        font.size = 12.0;
    }
    if let Some(font) = style.text_styles.get_mut(&TextStyle::Body) {
        font.size = 14.0;
    }
    if let Some(font) = style.text_styles.get_mut(&TextStyle::Button) {
        font.size = 14.0;
    }
    if let Some(font) = style.text_styles.get_mut(&TextStyle::Heading) {
        font.size = 20.0;
    }

    let radius = CornerRadius::same(4);
    let visuals = &mut style.visuals;
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.panel_fill = fluent.page;
    visuals.window_fill = fluent.page;
    visuals.extreme_bg_color = fluent.card;
    visuals.faint_bg_color = fluent.canvas;
    visuals.hyperlink_color = fluent.accent;
    visuals.warn_fg_color = fluent.warning_fg;
    visuals.error_fg_color = fluent.error_fg;
    visuals.weak_text_color = Some(fluent.text_secondary);
    visuals.window_stroke = Stroke::new(1.0, fluent.stroke);
    visuals.selection.bg_fill = fluent.accent;
    visuals.selection.stroke = Stroke::new(1.0, fluent.on_accent);

    let hairline = Stroke::new(1.0, fluent.stroke);
    let text = Stroke::new(1.0, fluent.text);
    visuals.widgets.noninteractive.weak_bg_fill = fluent.page;
    visuals.widgets.noninteractive.bg_fill = fluent.page;
    visuals.widgets.noninteractive.bg_stroke = hairline;
    visuals.widgets.noninteractive.fg_stroke = text;
    visuals.widgets.noninteractive.corner_radius = radius;

    visuals.widgets.inactive.weak_bg_fill = fluent.card;
    visuals.widgets.inactive.bg_fill = fluent.card;
    visuals.widgets.inactive.bg_stroke = hairline;
    visuals.widgets.inactive.fg_stroke = text;
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.weak_bg_fill = fluent.control_hover;
    visuals.widgets.hovered.bg_fill = fluent.control_hover;
    visuals.widgets.hovered.bg_stroke = hairline;
    visuals.widgets.hovered.fg_stroke = text;
    visuals.widgets.hovered.corner_radius = radius;

    visuals.widgets.active.weak_bg_fill = fluent.control_pressed;
    visuals.widgets.active.bg_fill = fluent.control_pressed;
    visuals.widgets.active.bg_stroke = hairline;
    visuals.widgets.active.fg_stroke = text;
    visuals.widgets.active.corner_radius = radius;

    visuals.widgets.open.weak_bg_fill = fluent.card;
    visuals.widgets.open.bg_fill = fluent.card;
    visuals.widgets.open.bg_stroke = hairline;
    visuals.widgets.open.fg_stroke = text;
    visuals.widgets.open.corner_radius = radius;
}
