//! The app's look: colours for light and dark, type, spacing and fonts.
//! Everything visual that is not layout lives here, so the window code in
//! `app.rs` only says what goes where.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, Vec2,
};

use crate::i18n::Lang;

/// Corner radius of cards and the drop zone.
pub const CARD_RADIUS: u8 = 12;
/// Corner radius of buttons and inputs.
pub const CONTROL_RADIUS: u8 = 8;

/// Font family for headings and the main button. Always defined; it falls
/// back to the regular face when Segoe UI Semibold is missing.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

#[derive(Clone, Copy)]
pub struct Palette {
    /// Window background.
    pub bg: Color32,
    /// Cards, inputs and the drop zone.
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Accent at low strength, for badges and the drop zone highlight.
    pub accent_soft: Color32,
    /// Resting fill of secondary buttons.
    pub control: Color32,
    pub control_hover: Color32,
    /// Unchecked boxes and the slider's track.
    pub track: Color32,
    pub success: Color32,
    pub warn: Color32,
    pub error: Color32,
}

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(244, 245, 248),
    surface: Color32::from_rgb(255, 255, 255),
    border: Color32::from_rgb(225, 228, 234),
    text: Color32::from_rgb(24, 28, 37),
    muted: Color32::from_rgb(104, 112, 126),
    accent: Color32::from_rgb(47, 107, 255),
    on_accent: Color32::WHITE,
    accent_soft: Color32::from_rgb(232, 239, 255),
    control: Color32::from_rgb(255, 255, 255),
    control_hover: Color32::from_rgb(236, 239, 245),
    track: Color32::from_rgb(221, 226, 236),
    success: Color32::from_rgb(24, 148, 82),
    warn: Color32::from_rgb(176, 112, 16),
    error: Color32::from_rgb(207, 58, 58),
};

const DARK: Palette = Palette {
    bg: Color32::from_rgb(21, 23, 28),
    surface: Color32::from_rgb(30, 33, 40),
    border: Color32::from_rgb(47, 51, 61),
    text: Color32::from_rgb(232, 234, 239),
    muted: Color32::from_rgb(150, 157, 170),
    accent: Color32::from_rgb(86, 136, 255),
    on_accent: Color32::WHITE,
    accent_soft: Color32::from_rgb(36, 48, 78),
    control: Color32::from_rgb(38, 42, 51),
    control_hover: Color32::from_rgb(48, 53, 64),
    track: Color32::from_rgb(62, 68, 82),
    success: Color32::from_rgb(70, 190, 124),
    warn: Color32::from_rgb(226, 170, 76),
    error: Color32::from_rgb(240, 110, 110),
};

pub fn palette(ctx: &egui::Context) -> Palette {
    match ctx.theme() {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
    }
}

fn visuals(p: Palette, mut v: egui::Visuals) -> egui::Visuals {
    let radius = CornerRadius::same(CONTROL_RADIUS);
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS);
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.bg;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.track;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.control_hover;
    w.hovered.weak_bg_fill = p.control_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.control_hover;
    w.open.weak_bg_fill = p.control_hover;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    v
}

/// Colours, type sizes and spacing. Call once at start.
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Light, visuals(LIGHT, egui::Visuals::light()));
    ctx.set_visuals_of(Theme::Dark, visuals(DARK, egui::Visuals::dark()));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.slider_width = 150.0;
        style.spacing.icon_width = 18.0;
        style.spacing.combo_width = 110.0;
        for (text_style, size) in [
            (TextStyle::Body, 14.5),
            (TextStyle::Button, 14.5),
            (TextStyle::Small, 12.0),
            (TextStyle::Heading, 20.0),
        ] {
            style
                .text_styles
                .insert(text_style, FontId::new(size, FontFamily::Proportional));
        }
    });
}

/// Uses Windows' Segoe UI for the interface, so the app looks at home, and
/// adds Windows' East Asian fonts for the display language and Windows'
/// language (so file names in the user's script show too). egui's built-in
/// fonts stay as the fallback and supply the symbols.
pub fn install_fonts(ctx: &egui::Context, languages: &[Lang]) {
    let fonts_dir = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("Fonts");
    let mut fonts = egui::FontDefinitions::default();
    let add = |fonts: &mut egui::FontDefinitions, file: &str, shift: f32| -> bool {
        if fonts.font_data.contains_key(file) {
            return true;
        }
        let Ok(bytes) = std::fs::read(fonts_dir.join(file)) else {
            return false;
        };
        let data = egui::FontData::from_owned(bytes).tweak(egui::FontTweak {
            y_offset_factor: shift,
            ..Default::default()
        });
        fonts.font_data.insert(file.to_owned(), Arc::new(data));
        true
    };

    let proportional = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut regular = proportional.clone();
    if add(&mut fonts, "segoeui.ttf", 0.0) {
        regular.insert(0, "segoeui.ttf".to_owned());
    }
    let mut strong = regular.clone();
    if add(&mut fonts, "seguisb.ttf", 0.0) {
        strong.insert(0, "seguisb.ttf".to_owned());
    }
    for lang in languages {
        // The shift puts mixed text ("3枚", "IMG_1.jpg") on one baseline:
        // some of these fonts sit higher on the line than the Latin font.
        let found = lang
            .system_fonts()
            .iter()
            .find(|(file, shift)| add(&mut fonts, file, *shift));
        if let Some((file, _)) = found {
            for list in [&mut regular, &mut strong] {
                if !list.iter().any(|name| name == file) {
                    list.push((*file).to_owned());
                }
            }
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push((*file).to_owned());
        }
    }
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(semibold(), strong);
    ctx.set_fonts(fonts);
}

/// A white (or dark) rounded panel that groups related things.
pub fn card(p: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 12))
}

/// The one filled button on the screen: the thing to press next.
pub fn primary_button<'a>(p: Palette, text: impl Into<String>, enabled: bool) -> egui::Button<'a> {
    let label = egui::RichText::new(text)
        .family(semibold())
        .size(15.5)
        .color(if enabled { p.on_accent } else { p.muted });
    egui::Button::new(label)
        .fill(if enabled { p.accent } else { p.control_hover })
        .stroke(Stroke::NONE)
        .corner_radius(10)
        .min_size(Vec2::new(230.0, 40.0))
}

/// A small rounded label, such as a file's format. Painted at its own size,
/// so it stays compact whatever the height of the row it sits in.
pub fn badge(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::new(11.0, semibold()), fg);
    let size = galley.size() + Vec2::new(12.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(size.x.max(40.0), size.y), egui::Sense::hover());
    ui.painter().rect_filled(rect, 5, bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
}

/// A checkbox with squarer corners than the buttons: at this size the
/// button radius would make it look like a radio button.
pub fn checkbox(ui: &mut egui::Ui, checked: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
            state.corner_radius = CornerRadius::same(5);
        }
        ui.checkbox(checked, text)
    })
    .inner
}