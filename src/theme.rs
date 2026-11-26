//! The app's look: colours for light and dark, type, spacing, fonts and the
//! painted pieces (buttons, chips, the segmented control, status badges,
//! progress, icons). It follows Windows 11's Fluent conventions: neutral
//! surfaces, 1 px strokes instead of shadows, small corner radii, Segoe UI
//! Variable, and a teal-green accent. Everything visual that is not layout
//! lives here, so `app.rs` only says what goes where.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, TextStyle, Theme, Vec2, pos2, vec2,
};

use crate::i18n::Lang;

/// Corner radius of cards, the drop zone and flyouts.
pub const CARD_RADIUS: u8 = 8;
/// Corner radius of buttons, inputs, chips and the segmented control.
pub const CONTROL_RADIUS: u8 = 4;
/// Height of buttons and single-line controls.
pub const CONTROL_HEIGHT: f32 = 32.0;
/// Height of a segmented control whose choices have a second line.
pub const SEGMENTED_TALL: f32 = 48.0;
/// Height of the main button and the progress shown in its place.
pub const PRIMARY_HEIGHT: f32 = 40.0;
/// Type scale (Fluent: caption 12, body 14, subtitle 20).
pub const BODY_SIZE: f32 = 14.0;
pub const SMALL_SIZE: f32 = 12.0;
pub const SUBTITLE_SIZE: f32 = 20.0;

/// Font family for headings, labels that need weight and the main button.
/// Always defined; it falls back to the regular face when no semibold face
/// is found.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn body() -> FontId {
    FontId::new(BODY_SIZE, FontFamily::Proportional)
}

pub fn small() -> FontId {
    FontId::new(SMALL_SIZE, FontFamily::Proportional)
}

pub fn strong() -> FontId {
    FontId::new(BODY_SIZE, semibold())
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Window background.
    pub bg: Color32,
    /// Cards, flyouts and the drop zone.
    pub surface: Color32,
    /// A list row under the pointer.
    pub hover: Color32,
    /// Card and divider strokes.
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Text of disabled controls (exempt from contrast rules, still legible).
    pub disabled: Color32,
    /// Fills: the main button, checked boxes, progress, the selection mark.
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    /// Accent-coloured text, readable on `bg`, `surface` and `accent_soft`.
    pub accent_text: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Accent at low strength, only for small selected or busy elements.
    pub accent_soft: Color32,
    /// Resting fill of standard buttons.
    pub control: Color32,
    pub control_hover: Color32,
    pub control_pressed: Color32,
    /// Outline of standard buttons, and their slightly darker bottom edge.
    pub control_border: Color32,
    pub control_edge: Color32,
    /// Outlines that must stand out (unchecked boxes, progress rails).
    pub strong_border: Color32,
    /// The segmented control's well and neutral chips.
    pub track: Color32,
    /// The chosen segment, raised from the well.
    pub raised: Color32,
    /// Fill of neutral badges (a waiting file).
    pub neutral_soft: Color32,
    pub success: Color32,
    pub success_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub error: Color32,
    pub error_soft: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Palette = Palette {
    bg: hex(0xF3F3F3),
    surface: hex(0xFFFFFF),
    hover: hex(0xF5F5F5),
    border: hex(0xE5E5E5),
    text: hex(0x1B1B1B),
    muted: hex(0x5F5F5F),
    disabled: hex(0xA0A0A0),
    accent: hex(0x0C7D66),
    accent_hover: hex(0x0B715D),
    accent_pressed: hex(0x0A6553),
    accent_text: hex(0x0A6E5A),
    on_accent: hex(0xFFFFFF),
    accent_soft: hex(0xE0F2EC),
    control: hex(0xFBFBFB),
    control_hover: hex(0xF5F5F5),
    control_pressed: hex(0xEFEFEF),
    control_border: hex(0xE3E3E3),
    control_edge: hex(0xCCCCCC),
    strong_border: hex(0x8A8A8A),
    track: hex(0xF0F0F0),
    raised: hex(0xFFFFFF),
    neutral_soft: hex(0xEDEDED),
    success: hex(0x0F7B3F),
    success_soft: hex(0xDFF6E5),
    warn: hex(0x9D5D00),
    warn_soft: hex(0xFFF4CE),
    error: hex(0xC42B1C),
    error_soft: hex(0xFDE7E9),
};

pub const DARK: Palette = Palette {
    bg: hex(0x202020),
    surface: hex(0x2B2B2B),
    hover: hex(0x323232),
    border: hex(0x3A3A3A),
    text: hex(0xFFFFFF),
    muted: hex(0xC5C5C5),
    disabled: hex(0x7A7A7A),
    accent: hex(0x3DDBB0),
    accent_hover: hex(0x5FE3C0),
    accent_pressed: hex(0x35C29C),
    accent_text: hex(0x3DDBB0),
    on_accent: hex(0x0B1F19),
    accent_soft: hex(0x1E3B33),
    control: hex(0x373737),
    control_hover: hex(0x3D3D3D),
    control_pressed: hex(0x323232),
    control_border: hex(0x434343),
    control_edge: hex(0x434343),
    strong_border: hex(0x9E9E9E),
    track: hex(0x262626),
    raised: hex(0x3C3C3C),
    neutral_soft: hex(0x3A3A3A),
    success: hex(0x6CCB5F),
    success_soft: hex(0x1F3323),
    warn: hex(0xFCE100),
    warn_soft: hex(0x433519),
    error: hex(0xFF99A4),
    error_soft: hex(0x442726),
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
    v.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
    // Flyouts keep a soft shadow, as Windows' own do; cards have none.
    v.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(40),
    };
    v.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(48),
    };
    v.extreme_bg_color = p.surface;
    v.text_edit_bg_color = Some(p.control);
    v.faint_bg_color = p.bg;
    v.hyperlink_color = p.accent_text;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.text_cursor.stroke = Stroke::new(1.5, p.text);
    // A steady caret: a blinking one redraws the window twice a second for
    // as long as a field keeps focus, even when nothing else happens.
    v.text_cursor.blink = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;

    w.hovered.bg_fill = p.control_hover;
    w.hovered.weak_bg_fill = p.control_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.control_edge);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    // egui draws a focused widget with the active look: give it the focus
    // outline so keyboard focus shows on combo boxes and text fields.
    w.active.bg_fill = p.control_pressed;
    w.active.weak_bg_fill = p.control_pressed;
    w.active.bg_stroke = Stroke::new(2.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.control_pressed;
    w.open.weak_bg_fill = p.control_pressed;
    w.open.bg_stroke = Stroke::new(1.0, p.control_edge);
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
        style.spacing.button_padding = Vec2::new(12.0, 5.0);
        style.spacing.interact_size.y = CONTROL_HEIGHT;
        style.spacing.icon_width = 18.0;
        style.spacing.combo_width = 160.0;
        style.spacing.menu_margin = egui::Margin::same(4);
        for (text_style, size) in [
            (TextStyle::Body, BODY_SIZE),
            (TextStyle::Button, BODY_SIZE),
            (TextStyle::Small, SMALL_SIZE),
            (TextStyle::Heading, SUBTITLE_SIZE),
        ] {
            style
                .text_styles
                .insert(text_style, FontId::new(size, FontFamily::Proportional));
        }
    });
}

/// A font file of Windows', mapped into memory read-only for the rest of
/// the process, once per file. Mapped rather than read, the pages are
/// Windows' shared copy of the file, loaded only where a glyph is used,
/// instead of a private copy of the whole file (Microsoft YaHei alone is
/// 19 MB). `None` when the file is missing or cannot be mapped.
fn system_font(file: &str) -> Option<&'static [u8]> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY};

    static MAPPED: std::sync::Mutex<Vec<(String, Option<&'static [u8]>)>> = std::sync::Mutex::new(Vec::new());
    let mut mapped = MAPPED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, bytes)) = mapped.iter().find(|(name, _)| name == file) {
        return *bytes;
    }
    let dir = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
    let bytes = std::fs::File::open(dir.join(file)).ok().and_then(|f| {
        let len = usize::try_from(f.metadata().ok()?.len()).ok().filter(|n| *n > 0)?;
        unsafe {
            let mapping = CreateFileMappingW(HANDLE(f.as_raw_handle()), None, PAGE_READONLY, 0, 0, None).ok()?;
            let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
            // The view keeps the file mapped after both handles are closed.
            let _ = CloseHandle(mapping);
            (!view.Value.is_null()).then(|| std::slice::from_raw_parts(view.Value as *const u8, len))
        }
    });
    mapped.push((file.to_owned(), bytes));
    bytes
}

/// Windows' fonts for what the interface fonts lack, last in line:
/// symbols, and emoji (drawn in one colour) in file and track names.
const FALLBACK_FONTS: [&str; 2] = ["seguisym.ttf", "seguiemj.ttf"];

/// Uses Windows 11's Segoe UI Variable for the interface (Segoe UI on
/// Windows 10), so the app looks at home, and adds Windows' East Asian fonts
/// for the display language and Windows' language (so file names in the
/// user's script show too), then Windows' symbol and emoji fonts. The app
/// bundles no fonts: egui's built-in ones were a fifth of the download.
pub fn install_fonts(ctx: &egui::Context, languages: &[Lang]) {
    let mut fonts = egui::FontDefinitions::empty();
    let add = |fonts: &mut egui::FontDefinitions, file: &str, shift: f32| -> bool {
        if fonts.font_data.contains_key(file) {
            return true;
        }
        let Some(bytes) = system_font(file) else {
            return false;
        };
        let data = egui::FontData::from_static(bytes).tweak(egui::FontTweak {
            y_offset_factor: shift,
            ..Default::default()
        });
        fonts.font_data.insert(file.to_owned(), Arc::new(data));
        true
    };

    let mut regular: Vec<String> = Vec::new();
    let mut strong: Vec<String> = Vec::new();
    if let Some(bytes) = system_font("SegUIVar.ttf") {
        for (name, weight, list) in [("segui-var-400", 400.0, &mut regular), ("segui-var-600", 600.0, &mut strong)] {
            let data = egui::FontData::from_static(bytes).tweak(egui::FontTweak {
                coords: egui::epaint::text::VariationCoords::new([("wght", weight), ("opsz", 10.5)]),
                ..Default::default()
            });
            fonts.font_data.insert(name.to_owned(), Arc::new(data));
            list.push(name.to_owned());
        }
    } else {
        // Segoe UI is on every Windows since Vista; Arial only in case.
        for file in ["segoeui.ttf", "arial.ttf"] {
            if add(&mut fonts, file, 0.0) {
                regular.push(file.to_owned());
                strong.push(file.to_owned());
                break;
            }
        }
        if add(&mut fonts, "seguisb.ttf", 0.0) {
            strong.insert(0, "seguisb.ttf".to_owned());
        }
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
        }
    }
    for file in FALLBACK_FONTS {
        if add(&mut fonts, file, 0.0) {
            regular.push(file.to_owned());
            strong.push(file.to_owned());
        }
    }
    // Nothing in the window uses a monospace font, but egui needs one.
    fonts.families.insert(FontFamily::Monospace, regular.clone());
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(semibold(), strong);
    ctx.set_fonts(fonts);
}

/// A card: a surface with a 1 px stroke, no shadow.
pub fn card(p: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(16, 12))
}

/// A one-line text cut to `width` with an ellipsis.
pub fn truncated(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    painter.layout_job(job)
}

/// Pieces of text in their own colours on one line, cut to `width` with an
/// ellipsis.
pub fn truncated_parts(
    painter: &egui::Painter,
    parts: &[(&str, Color32)],
    font: FontId,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    for (text, color) in parts {
        job.append(text, 0.0, egui::TextFormat::simple(font.clone(), *color));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    painter.layout_job(job)
}

/// Width `text` takes in `font`.
pub fn text_width(ui: &egui::Ui, text: &str, font: FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
        .size()
        .x
}

/// Whether `response`'s widget has keyboard focus, whether or not the
/// window is active (Windows keeps the outline on an inactive window too).
pub fn has_focus(ui: &egui::Ui, response: &Response) -> bool {
    ui.memory(|m| m.has_focus(response.id))
}

/// The keyboard focus outline Windows draws: 2 px in the text colour, just
/// outside the control. egui only gives buttons focus from the keyboard, so
/// a mouse click never shows it.
pub fn focus_ring(ui: &egui::Ui, response: &Response, radius: u8) {
    if has_focus(ui, response) {
        let p = palette(ui.ctx());
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            radius + 2,
            Stroke::new(2.0, p.text),
            StrokeKind::Outside,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one accent-filled button: the thing to press next.
    Primary,
    /// A bordered neutral button.
    Standard,
    /// Text in the accent colour, underlined on hover.
    Link,
    /// Borderless text that gains a fill under the pointer.
    Subtle,
}

/// Fill, stroke and text colour of a button in a state, so every state is
/// defined in one place (and tested).
pub fn button_colors(p: Palette, kind: ButtonKind, enabled: bool, hovered: bool, pressed: bool) -> (Color32, Color32, Color32) {
    match kind {
        ButtonKind::Primary if !enabled => (p.track, p.border, p.disabled),
        ButtonKind::Primary if pressed => (p.accent_pressed, Color32::TRANSPARENT, p.on_accent),
        ButtonKind::Primary if hovered => (p.accent_hover, Color32::TRANSPARENT, p.on_accent),
        ButtonKind::Primary => (p.accent, Color32::TRANSPARENT, p.on_accent),
        ButtonKind::Standard if !enabled => (p.control, p.control_border, p.disabled),
        ButtonKind::Standard if pressed => (p.control_pressed, p.control_border, p.muted),
        ButtonKind::Standard if hovered => (p.control_hover, p.control_border, p.text),
        ButtonKind::Standard => (p.control, p.control_border, p.text),
        ButtonKind::Link if !enabled => (Color32::TRANSPARENT, Color32::TRANSPARENT, p.disabled),
        ButtonKind::Link => (Color32::TRANSPARENT, Color32::TRANSPARENT, p.accent_text),
        ButtonKind::Subtle if !enabled => (Color32::TRANSPARENT, Color32::TRANSPARENT, p.disabled),
        ButtonKind::Subtle if pressed => (p.control_pressed, Color32::TRANSPARENT, p.muted),
        ButtonKind::Subtle if hovered => (p.control_hover, Color32::TRANSPARENT, p.text),
        ButtonKind::Subtle => (Color32::TRANSPARENT, Color32::TRANSPARENT, p.text),
    }
}

/// A button drawn the Fluent way, with rest, hover, pressed, disabled and
/// keyboard-focus looks. `width` 0 sizes it to its text.
pub fn button(ui: &mut egui::Ui, kind: ButtonKind, text: &str, width: f32, height: f32) -> Response {
    let font = if kind == ButtonKind::Primary { strong() } else { body() };
    button_in(ui, kind, text, font, width, height)
}

/// A text link in a given font, sized to its text.
pub fn link(ui: &mut egui::Ui, text: &str, font: FontId) -> Response {
    let height = ui.fonts_mut(|f| f.row_height(&font)) + 4.0;
    button_in(ui, ButtonKind::Link, text, font, 0.0, height)
}

fn button_in(ui: &mut egui::Ui, kind: ButtonKind, text: &str, font: FontId, width: f32, height: f32) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let pad = if kind == ButtonKind::Link { 2.0 } else { 12.0 };
    let natural = text_width(ui, text, font.clone()) + pad * 2.0;
    let width = if width > 0.0 { width } else { natural.min(ui.available_width().max(40.0)) };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let response = if kind == ButtonKind::Link {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, stroke, fg) = button_colors(p, kind, enabled, hovered, pressed);
    let painter = ui.painter();
    if kind != ButtonKind::Link {
        painter.rect(rect, CONTROL_RADIUS, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
        // Fluent's elevation edge: the bottom of a resting standard button
        // is a shade darker.
        if kind == ButtonKind::Standard && enabled && !pressed {
            painter.hline(
                (rect.left() + 3.0)..=(rect.right() - 3.0),
                rect.bottom() - 0.5,
                Stroke::new(1.0, p.control_edge),
            );
        }
    }
    let galley = truncated(painter, text, font, fg, rect.width() - pad * 2.0);
    let pos = rect.center() - galley.size() / 2.0;
    if kind == ButtonKind::Link && hovered {
        let y = pos.y + galley.size().y - 1.0;
        painter.hline(pos.x..=(pos.x + galley.size().x), y, Stroke::new(1.0, fg));
    }
    painter.galley(pos, galley, fg);
    focus_ring(ui, &response, CONTROL_RADIUS);
    response
}

/// Icons drawn with lines, so they never depend on a font having a glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Close,
    Play,
    Folder,
    Retry,
    Gear,
    Check,
    Clock,
    Alert,
    Info,
    Warning,
    Arrow,
    ChevronDown,
    ChevronRight,
}

pub fn paint_icon(painter: &egui::Painter, centre: Pos2, icon: Icon, color: Color32) {
    paint_icon_sized(painter, centre, icon, color, 1.0);
}

/// An icon at `s` times its normal (about 14 px) size.
pub fn paint_icon_sized(painter: &egui::Painter, centre: Pos2, icon: Icon, color: Color32, s: f32) {
    let stroke = Stroke::new(1.4 * s.sqrt(), color);
    let c = centre;
    let vec2 = |x: f32, y: f32| egui::vec2(x * s, y * s);
    let arc = |radius: f32, from: f32, to: f32, steps: usize| -> Vec<Pos2> {
        (0..=steps)
            .map(|i| {
                let a = from + (to - from) * i as f32 / steps as f32;
                c + vec2(a.cos() * radius, a.sin() * radius)
            })
            .collect()
    };
    match icon {
        Icon::Close => {
            let d = 4.0;
            painter.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
            painter.line_segment([c + vec2(d, -d), c + vec2(-d, d)], stroke);
        }
        Icon::Play => {
            let points = vec![c + vec2(-3.0, -5.0), c + vec2(5.0, 0.0), c + vec2(-3.0, 5.0)];
            painter.add(Shape::convex_polygon(points, Color32::TRANSPARENT, stroke));
        }
        Icon::Folder => {
            let body = Rect::from_center_size(c + vec2(0.0, 1.0), vec2(13.0, 9.5));
            painter.rect_stroke(body, 1.5, stroke, StrokeKind::Middle);
            let tab = [
                body.left_top(),
                body.left_top() + vec2(0.0, -2.0),
                body.left_top() + vec2(4.5, -2.0),
                body.left_top() + vec2(6.0, 0.0),
            ];
            painter.add(Shape::line(tab.to_vec(), stroke));
        }
        Icon::Retry => {
            let r = 5.0;
            painter.add(Shape::line(arc(r, -0.4, -0.4 + std::f32::consts::PI * 1.6, 18), stroke));
            let end = c + vec2((-0.4f32).cos() * r, (-0.4f32).sin() * r);
            painter.line_segment([end, end + vec2(-0.5, -3.5)], stroke);
            painter.line_segment([end, end + vec2(-3.5, 0.5)], stroke);
        }
        Icon::Gear => {
            // A hub inside a ring of eight teeth.
            painter.circle_stroke(c, 2.4 * s, stroke);
            let mut points = Vec::new();
            for i in 0..16 {
                let a = i as f32 / 16.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_8 / 2.0;
                let r = if i % 2 == 0 { 7.0 } else { 5.0 };
                points.push(c + vec2(a.cos() * r, a.sin() * r));
                let a2 = (i as f32 + 1.0) / 16.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_8 / 2.0;
                points.push(c + vec2(a2.cos() * r, a2.sin() * r));
            }
            points.push(points[0]);
            painter.add(Shape::line(points, stroke));
        }
        Icon::Check => {
            let points = vec![c + vec2(-4.0, 0.2), c + vec2(-1.2, 3.0), c + vec2(4.2, -3.2)];
            painter.add(Shape::line(points, Stroke::new(1.6, color)));
        }
        Icon::Clock => {
            painter.circle_stroke(c, 5.0 * s, stroke);
            painter.line_segment([c, c + vec2(0.0, -3.0)], stroke);
            painter.line_segment([c, c + vec2(2.2, 1.2)], stroke);
        }
        Icon::Alert => {
            painter.circle_stroke(c, 5.0 * s, stroke);
            painter.line_segment([c + vec2(-2.0, -2.0), c + vec2(2.0, 2.0)], stroke);
            painter.line_segment([c + vec2(2.0, -2.0), c + vec2(-2.0, 2.0)], stroke);
        }
        Icon::Info => {
            painter.circle_stroke(c, 6.0 * s, stroke);
            painter.line_segment([c + vec2(0.0, -0.5), c + vec2(0.0, 3.0)], stroke);
            painter.circle_filled(c + vec2(0.0, -2.8), 0.9 * s, color);
        }
        Icon::Warning => {
            let points = vec![c + vec2(0.0, -5.5), c + vec2(5.8, 4.5), c + vec2(-5.8, 4.5), c + vec2(0.0, -5.5)];
            painter.add(Shape::line(points, stroke));
            painter.line_segment([c + vec2(0.0, -1.8), c + vec2(0.0, 1.4)], stroke);
            painter.circle_filled(c + vec2(0.0, 3.0), 0.8 * s, color);
        }
        Icon::Arrow => {
            // Into a tray: the drop zone's motif.
            painter.line_segment([c + vec2(0.0, -7.0), c + vec2(0.0, 2.5)], stroke);
            painter.add(Shape::line(
                vec![c + vec2(-4.0, -1.5), c + vec2(0.0, 2.5), c + vec2(4.0, -1.5)],
                stroke,
            ));
            painter.add(Shape::line(
                vec![c + vec2(-8.0, 1.0), c + vec2(-8.0, 7.0), c + vec2(8.0, 7.0), c + vec2(8.0, 1.0)],
                stroke,
            ));
        }
        Icon::ChevronDown => {
            let points = vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)];
            painter.add(Shape::line(points, stroke));
        }
        Icon::ChevronRight => {
            let points = vec![c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)];
            painter.add(Shape::line(points, stroke));
        }
    }
}

/// A square, borderless icon button with a subtle hover fill.
pub fn icon_button(ui: &mut egui::Ui, rect: Rect, id: egui::Id, icon: Icon, p: Palette) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    let enabled = ui.is_enabled();
    if enabled && response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, CONTROL_RADIUS, p.control_pressed);
    } else if enabled && response.hovered() {
        ui.painter().rect_filled(rect, CONTROL_RADIUS, p.control_hover);
    }
    let color = if !enabled {
        p.disabled
    } else if response.hovered() {
        p.text
    } else {
        p.muted
    };
    paint_icon(ui.painter(), rect.center(), icon, color);
    focus_ring(ui, &response, CONTROL_RADIUS);
    response
}

/// How a status badge looks: its icon, text colour and fill.
pub fn status_look(p: Palette, status: StatusKind) -> (Option<Icon>, Color32, Color32) {
    match status {
        StatusKind::Waiting => (Some(Icon::Clock), p.muted, p.neutral_soft),
        StatusKind::Busy => (None, p.accent_text, p.accent_soft),
        StatusKind::Done => (Some(Icon::Check), p.success, p.success_soft),
        StatusKind::Warning => (Some(Icon::Warning), p.warn, p.warn_soft),
        StatusKind::Failed => (Some(Icon::Alert), p.error, p.error_soft),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Waiting,
    Busy,
    Done,
    Warning,
    Failed,
}

/// A status badge: icon and word on a soft fill, 4 px corners. The icon
/// and word say the state, so it never rests on colour alone. Painted with
/// its right edge at `right.x`, centred on `right.y`; returns its rect.
pub fn status_badge(painter: &egui::Painter, right: Pos2, text: &str, status: StatusKind, p: Palette, max_width: f32) -> Rect {
    let (icon, fg, bg) = status_look(p, status);
    let icon_w = if icon.is_some() { 16.0 } else { 0.0 };
    let galley = truncated(painter, text, FontId::new(SMALL_SIZE, semibold()), fg, max_width - 16.0 - icon_w);
    let size = vec2(galley.size().x + 16.0 + icon_w, 22.0);
    let rect = Rect::from_min_size(pos2(right.x - size.x, right.y - size.y / 2.0), size);
    painter.rect_filled(rect, CONTROL_RADIUS, bg);
    if let Some(icon) = icon {
        paint_icon(painter, pos2(rect.left() + 8.0 + 6.0, rect.center().y), icon, fg);
    }
    painter.galley(
        pos2(rect.left() + 8.0 + icon_w, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    rect
}

/// A neutral tag laid out like a widget (the licence state).
pub fn tag(ui: &mut egui::Ui, icon: Icon, text: &str, fg: Color32, bg: Color32) -> Response {
    let max = ui.available_width().max(60.0);
    let galley = truncated(ui.painter(), text, FontId::new(SMALL_SIZE, semibold()), fg, max - 34.0);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 34.0, 24.0), Sense::hover());
    ui.painter().rect_filled(rect, CONTROL_RADIUS, bg);
    paint_icon(ui.painter(), pos2(rect.left() + 14.0, rect.center().y), icon, fg);
    ui.painter().galley(
        pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    response
}

/// A toggle chip, such as a purpose preset. Selected: accent outline, soft
/// fill and a check mark, so the choice shows without relying on colour.
pub fn chip(ui: &mut egui::Ui, id: egui::Id, text: &str, selected: bool) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let check_w = if selected { 18.0 } else { 0.0 };
    let width = text_width(ui, text, body()) + 24.0 + check_w;
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, stroke, fg) = if !enabled {
        (p.control, p.control_border, p.disabled)
    } else if selected {
        (p.accent_soft, p.accent, p.accent_text)
    } else if pressed {
        (p.control_pressed, p.control_border, p.muted)
    } else if hovered {
        (p.control_hover, p.control_edge, p.text)
    } else {
        (p.control, p.control_border, p.text)
    };
    let painter = ui.painter();
    painter.rect(rect, CONTROL_RADIUS, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let mut x = rect.left() + 12.0;
    if selected {
        paint_icon(painter, pos2(x + 6.0, rect.center().y), Icon::Check, fg);
        x += check_w;
    }
    let galley = truncated(painter, text, body(), fg, rect.right() - x - 10.0);
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    focus_ring(ui, &response, CONTROL_RADIUS);
    response
}

/// The leading badge of a file row: a neutral tile with a small glyph (a
/// film frame for video, a note for audio) and the file's extension.
pub fn file_badge(painter: &egui::Painter, rect: Rect, video: bool, extension: &str, p: Palette) {
    painter.rect(rect, CONTROL_RADIUS, p.bg, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let g = pos2(rect.center().x, rect.top() + 13.0);
    let stroke = Stroke::new(1.3, p.accent_text);
    if video {
        let frame = Rect::from_center_size(g, vec2(14.0, 10.0));
        painter.rect_stroke(frame, 1.5, stroke, StrokeKind::Middle);
        let points = vec![g + vec2(-1.8, -2.6), g + vec2(2.8, 0.0), g + vec2(-1.8, 2.6)];
        painter.add(Shape::convex_polygon(points, p.accent_text, Stroke::NONE));
    } else {
        // A pair of beamed notes.
        painter.line_segment([g + vec2(-3.0, -4.5), g + vec2(-3.0, 3.0)], stroke);
        painter.line_segment([g + vec2(4.0, -5.5), g + vec2(4.0, 2.0)], stroke);
        painter.line_segment([g + vec2(-3.0, -4.5), g + vec2(4.0, -5.5)], Stroke::new(2.0, p.accent_text));
        painter.circle_filled(g + vec2(-4.6, 3.4), 2.0, p.accent_text);
        painter.circle_filled(g + vec2(2.4, 2.4), 2.0, p.accent_text);
    }
    let galley = truncated(painter, extension, FontId::new(9.5, semibold()), p.muted, rect.width() - 4.0);
    painter.galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - 3.0 - galley.size().y),
        galley,
        p.muted,
    );
}

/// A dashed outline with rounded corners.
pub fn dashed_rect(painter: &egui::Painter, rect: Rect, radius: f32, stroke: Stroke) {
    let mut points = Vec::new();
    let corners = [
        (pos2(rect.right() - radius, rect.top() + radius), -90.0f32),
        (pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
        (pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
        (pos2(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    for (centre, start) in corners {
        for step in 0..=6 {
            let a = (start + step as f32 * 15.0).to_radians();
            points.push(centre + vec2(a.cos(), a.sin()) * radius);
        }
    }
    points.push(points[0]);
    painter.extend(Shape::dashed_line(&points, stroke, 6.0, 4.0));
}

/// The x-range an unknown amount of progress occupies at `time`: a short
/// segment sweeping along the rail.
pub fn sweep(left: f32, width: f32, time: f64) -> (f32, f32) {
    let span = width * 0.3;
    let t = (time * 0.7).fract() as f32;
    let start = left - span + (width + span) * t;
    (start.max(left), (start + span).min(left + width))
}

/// A Fluent progress bar: a 1 px rail and a 3 px rounded fill. `None` is an
/// unknown amount (only drawn while a run is going, which repaints anyway).
pub fn progress_bar(painter: &egui::Painter, rect: Rect, fraction: Option<f32>, time: f64, p: Palette) {
    let cy = rect.center().y;
    painter.hline(rect.x_range(), cy, Stroke::new(1.0, p.strong_border));
    let (from, to) = match fraction {
        Some(f) => (rect.left(), rect.left() + rect.width() * f.clamp(0.0, 1.0)),
        None => sweep(rect.left(), rect.width(), time),
    };
    if to - from > 0.5 {
        let fill = Rect::from_x_y_ranges(from..=to, (cy - 1.5)..=(cy + 1.5));
        painter.rect_filled(fill, 1.5, p.accent);
    }
}

/// One choice of a segmented control: a label and an optional second,
/// smaller line (such as a bitrate).
pub struct Segment<'a> {
    pub label: &'a str,
    pub detail: Option<String>,
}

/// Choices side by side in a well; the chosen one is raised (a lighter
/// tile with a stroke) and marked with a short accent bar, as in Windows'
/// own selector bars. Returns the newly chosen index, if any.
pub fn segmented(ui: &mut egui::Ui, id: egui::Id, segments: &[Segment], selected: usize, p: Palette) -> Option<usize> {
    let two_lines = segments.iter().any(|s| s.detail.is_some());
    let height = if two_lines { SEGMENTED_TALL } else { CONTROL_HEIGHT };
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let enabled = ui.is_enabled();
    ui.painter()
        .rect(rect, CONTROL_RADIUS, p.track, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let n = segments.len().max(1) as f32;
    let inner = rect.shrink(2.0);
    let seg_w = inner.width() / n;
    let mut chosen = None;
    for (i, segment) in segments.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(inner.left() + seg_w * i as f32, inner.top()), vec2(seg_w, inner.height()));
        let response = ui.interact(cell, id.with(i), Sense::click());
        if response.clicked() && i != selected {
            chosen = Some(i);
        }
        let on = i == selected;
        let painter = ui.painter();
        if on {
            painter.rect(cell, CONTROL_RADIUS - 1, p.raised, Stroke::new(1.0, p.control_border), StrokeKind::Inside);
            let mark = Rect::from_center_size(pos2(cell.center().x, cell.bottom() - 3.0), vec2(16.0, 3.0));
            painter.rect_filled(mark, 1.5, if enabled { p.accent } else { p.disabled });
        } else if enabled && response.is_pointer_button_down_on() {
            painter.rect_filled(cell, CONTROL_RADIUS - 1, p.control_pressed);
        } else if enabled && response.hovered() {
            painter.rect_filled(cell, CONTROL_RADIUS - 1, p.control_hover);
        }
        let fg = if !enabled { p.disabled } else { p.text };
        let sub = if !enabled { p.disabled } else { p.muted };
        let font = if on { strong() } else { body() };
        let label = truncated(painter, segment.label, font, fg, cell.width() - 8.0);
        // The chosen label moves up a little to clear the accent mark.
        let lift = match (on, two_lines) {
            (false, _) => 0.0,
            (true, false) => 1.5,
            (true, true) => 2.5,
        };
        match &segment.detail {
            Some(detail) => {
                let small = truncated(painter, detail, small(), sub, cell.width() - 8.0);
                let top = cell.center().y - (label.size().y + small.size().y) / 2.0 - lift;
                painter.galley(pos2(cell.center().x - label.size().x / 2.0, top), label.clone(), fg);
                painter.galley(
                    pos2(cell.center().x - small.size().x / 2.0, top + label.size().y),
                    small,
                    sub,
                );
            }
            None => {
                painter.galley(cell.center() - label.size() / 2.0 - vec2(0.0, lift), label, fg);
            }
        }
        focus_ring(ui, &response, CONTROL_RADIUS);
    }
    chosen
}

/// A Fluent text box, `width` wide and a control high: a control fill, a
/// 1 px outline and a bottom edge that turns into a 2 px accent line while
/// typing. `invalid` draws the outline and edge in the error colour (the
/// message itself goes next to it, so it never rests on colour alone).
pub fn text_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    width: f32,
    invalid: bool,
) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::hover());
    let focused = enabled && ui.memory(|m| m.has_focus(id));
    let hovered = enabled && ui.rect_contains_pointer(rect);
    let fill = if focused {
        p.surface
    } else if hovered {
        p.control_hover
    } else {
        p.control
    };
    let outline = if invalid && enabled { p.error } else { p.control_border };
    ui.painter()
        .rect(rect, CONTROL_RADIUS, fill, Stroke::new(1.0, outline), StrokeKind::Inside);
    let (edge_w, edge) = match (enabled, invalid, focused) {
        (false, _, _) => (1.0, p.control_border),
        (true, true, _) => (2.0, p.error),
        (true, false, true) => (2.0, p.accent),
        (true, false, false) => (1.0, p.strong_border),
    };
    // The edge follows the rounded corners: the box's own shape, clipped
    // to its bottom pixels.
    let bottom = Rect::from_min_max(pos2(rect.left(), rect.bottom() - edge_w), rect.max);
    ui.painter().with_clip_rect(bottom).rect_filled(rect, CONTROL_RADIUS, edge);
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .hint_text(egui::RichText::new(hint).color(if enabled { p.muted } else { p.disabled }))
        .font(body())
        .text_color(if enabled { p.text } else { p.disabled })
        // A frame of our own replaces the margin; this one only pads.
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(9, 0)))
        .vertical_align(egui::Align::Center)
        .char_limit(16)
        .min_size(vec2(width, CONTROL_HEIGHT))
        .desired_width(width - 16.0);
    ui.put(rect, edit)
}

/// A Fluent checkbox: a 18 px box with a strong outline, accent-filled with
/// a check mark when on, and the label beside it. Clicking the label
/// toggles it too.
pub fn checkbox(ui: &mut egui::Ui, checked: &mut bool, text: &str, font: FontId) -> Response {
    let p = palette(ui.ctx());
    let enabled = ui.is_enabled();
    let label_w = text_width(ui, text, font.clone()).min((ui.available_width() - 28.0).max(20.0));
    let (rect, mut response) = ui.allocate_exact_size(vec2(26.0 + label_w, 28.0), Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let boxr = Rect::from_center_size(pos2(rect.left() + 9.0, rect.center().y), vec2(18.0, 18.0));
    let painter = ui.painter();
    if *checked {
        let fill = match (enabled, pressed, hovered) {
            (false, _, _) => p.disabled,
            (_, true, _) => p.accent_pressed,
            (_, _, true) => p.accent_hover,
            _ => p.accent,
        };
        painter.rect_filled(boxr, CONTROL_RADIUS, fill);
        paint_icon(painter, boxr.center(), Icon::Check, p.on_accent);
    } else {
        let fill = if pressed {
            p.control_pressed
        } else if hovered {
            p.control_hover
        } else {
            p.control
        };
        let edge = if enabled { p.strong_border } else { p.disabled };
        painter.rect(boxr, CONTROL_RADIUS, fill, Stroke::new(1.0, edge), StrokeKind::Inside);
    }
    let fg = if enabled { p.text } else { p.disabled };
    let galley = truncated(painter, text, font, fg, label_w);
    painter.galley(pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    if has_focus(ui, &response) {
        painter.rect_stroke(boxr.expand(2.0), CONTROL_RADIUS + 2, Stroke::new(2.0, p.text), StrokeKind::Outside);
    }
    response
}

/// WCAG 2 contrast ratio between two colours (1 to 21).
#[cfg(test)]
fn contrast(a: Color32, b: Color32) -> f32 {
    fn channel(c: u8) -> f32 {
        let c = c as f32 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminance(c: Color32) -> f32 {
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_of_known_pairs() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.01);
    }

    /// Every text colour reaches WCAG AA (4.5:1) on what it is drawn on.
    #[test]
    fn text_is_readable_in_both_palettes() {
        for (name, p) in [("light", LIGHT), ("dark", DARK)] {
            let pairs = [
                ("text/bg", p.text, p.bg),
                ("text/surface", p.text, p.surface),
                ("text/hover", p.text, p.hover),
                ("text/track", p.text, p.track),
                ("text/raised", p.text, p.raised),
                ("text/control", p.text, p.control),
                ("text/control_hover", p.text, p.control_hover),
                ("muted/bg", p.muted, p.bg),
                ("muted/surface", p.muted, p.surface),
                ("muted/hover", p.muted, p.hover),
                ("muted/track", p.muted, p.track),
                ("muted/raised", p.muted, p.raised),
                ("muted/control_pressed", p.muted, p.control_pressed),
                ("muted/neutral_soft", p.muted, p.neutral_soft),
                ("text/warn_soft", p.text, p.warn_soft),
                ("muted/warn_soft", p.muted, p.warn_soft),
                ("on_accent/accent", p.on_accent, p.accent),
                ("on_accent/accent_hover", p.on_accent, p.accent_hover),
                ("on_accent/accent_pressed", p.on_accent, p.accent_pressed),
                ("accent_text/bg", p.accent_text, p.bg),
                ("accent_text/surface", p.accent_text, p.surface),
                ("accent_text/accent_soft", p.accent_text, p.accent_soft),
                ("success/surface", p.success, p.surface),
                ("warn/surface", p.warn, p.surface),
                ("error/surface", p.error, p.surface),
                ("success/success_soft", p.success, p.success_soft),
                ("warn/warn_soft", p.warn, p.warn_soft),
                ("error/error_soft", p.error, p.error_soft),
            ];
            for (pair, fg, bg) in pairs {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{name} {pair}: {ratio:.2}");
            }
        }
    }

    /// Outlines that carry meaning (an unchecked box, the progress rail, the
    /// accent mark) reach WCAG's 3:1 for non-text contrast.
    #[test]
    fn meaningful_outlines_are_visible() {
        for (name, p) in [("light", LIGHT), ("dark", DARK)] {
            for (pair, fg, bg) in [
                ("strong_border/surface", p.strong_border, p.surface),
                ("strong_border/control", p.strong_border, p.control),
                ("accent/surface", p.accent, p.surface),
                ("accent/raised", p.accent, p.raised),
                ("text/bg (focus ring)", p.text, p.bg),
            ] {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 3.0, "{name} {pair}: {ratio:.2}");
            }
        }
    }

    /// No violet, purple or lavender: every colour's hue stays out of the
    /// 250 to 330 degree band unless it is a near-grey.
    #[test]
    fn palette_has_no_purple() {
        fn hue_and_chroma(c: Color32) -> (f32, f32) {
            let (r, g, b) = (c.r() as f32, c.g() as f32, c.b() as f32);
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let chroma = max - min;
            if chroma == 0.0 {
                return (0.0, 0.0);
            }
            let h = if max == r {
                ((g - b) / chroma).rem_euclid(6.0)
            } else if max == g {
                (b - r) / chroma + 2.0
            } else {
                (r - g) / chroma + 4.0
            };
            (h * 60.0, chroma)
        }
        for p in [LIGHT, DARK] {
            for c in [
                p.bg, p.surface, p.hover, p.border, p.text, p.muted, p.accent, p.accent_text,
                p.accent_soft, p.control, p.track, p.raised,
            ] {
                let (hue, chroma) = hue_and_chroma(c);
                assert!(chroma < 8.0 || !(250.0..=330.0).contains(&hue), "{c:?}");
            }
        }
    }

    #[test]
    fn every_button_state_is_distinct() {
        for p in [LIGHT, DARK] {
            for kind in [ButtonKind::Primary, ButtonKind::Standard] {
                let rest = button_colors(p, kind, true, false, false);
                let hover = button_colors(p, kind, true, true, false);
                let pressed = button_colors(p, kind, true, true, true);
                let disabled = button_colors(p, kind, false, false, false);
                assert_ne!(rest.0, hover.0, "{kind:?}");
                assert_ne!(hover.0, pressed.0, "{kind:?}");
                assert_ne!(rest, disabled, "{kind:?}");
            }
        }
    }

    #[test]
    fn sweep_stays_on_the_rail() {
        for t in [0.0, 0.3, 0.71, 1.4, 99.9] {
            let (from, to) = sweep(10.0, 200.0, t);
            assert!(from >= 10.0 && to <= 210.0 && from <= to.max(from), "{t}: {from} {to}");
        }
    }
}
