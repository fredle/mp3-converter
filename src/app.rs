//! The window: a queue of images, the output options, and the worker threads
//! that convert them. The footer with the auto-update toggle and version is
//! required on every factory app. Colours and type come from `theme.rs`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align, Align2, FontId, Layout, RichText, Stroke, StrokeKind};

use crate::convert::{Codec, ConvertError, Format, Options};
use crate::i18n::{Lang, Strings, fill};
use crate::license;
use crate::theme::{self, Palette};

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 620.0;
const APP_NAME: &str = "MP4 to MP3 & Audio Converter";
/// Marks a decoder crash; shown to the user as the translated "couldn't read".
const PANIC_MESSAGE: &str = "decoder panicked";
/// Space between the window edge and its contents.
const GUTTER: i8 = 16;
const ROW_HEIGHT: f32 = 36.0;
const FORMATS: &str = "HEIC · WebP · AVIF · JXL · JPG · PNG · BMP · TIFF · GIF";

pub fn run(args: Vec<String>) -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([WIDTH, HEIGHT])
        .with_min_inner_size([560.0, 480.0])
        .with_drag_and_drop(true);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(args, &cc.egui_ctx)))),
    )
}

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Waiting,
    Converting,
    Done(PathBuf),
    Failed(ConvertError),
}

struct Item {
    path: PathBuf,
    status: Status,
}

type JobResult = (usize, Result<PathBuf, ConvertError>);

pub struct App {
    settings: crate::settings::Settings,
    version: String,
    items: Vec<Item>,
    results: Option<Receiver<JobResult>>,
    in_flight: usize,
    /// How many files the current run started with, for the progress bar.
    run_total: usize,
    /// Folder of the most recent successful conversion, for "Open folder".
    last_output_dir: Option<PathBuf>,
    licensed_to: Option<String>,
    unlock_open: bool,
    key_entry: String,
    key_message: Option<String>,
    lang: Lang,
    t: &'static Strings,
}

impl App {
    fn new(args: Vec<String>, ctx: &egui::Context) -> Self {
        let settings = crate::settings::load();
        let licensed_to = license::verify(&settings.license_key);
        let lang = settings.language.unwrap_or_else(Lang::system);
        theme::apply(ctx);
        theme::install_fonts(ctx, &[lang, Lang::system()]);
        // For checking both looks without changing Windows' setting.
        match std::env::var("HWC_THEME").as_deref() {
            Ok("dark") => ctx.set_theme(egui::Theme::Dark),
            Ok("light") => ctx.set_theme(egui::Theme::Light),
            _ => {}
        }
        let mut app = Self {
            lang,
            t: lang.strings(),
            settings,
            version: crate::update::current_version(),
            items: Vec::new(),
            results: None,
            in_flight: 0,
            run_total: 0,
            last_output_dir: None,
            licensed_to,
            unlock_open: false,
            key_entry: String::new(),
            key_message: None,
        };
        // Files passed on the command line ("Open with", or dropped on the exe).
        let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
        app.add_paths(&paths);
        app
    }

    fn save(&self) {
        crate::settings::save(&self.settings);
    }

    fn set_language(&mut self, lang: Lang, ctx: &egui::Context) {
        self.lang = lang;
        self.t = lang.strings();
        self.key_message = None;
        theme::install_fonts(ctx, &[lang, Lang::system()]);
        self.settings.language = Some(lang);
        self.save();
    }

    fn format_label(&self, format: Format) -> String {
        match format {
            Format::Webp => format!("{} ({})", format.label(), self.t.lossless),
            _ => format.label().to_owned(),
        }
    }

    fn busy(&self) -> bool {
        self.in_flight > 0
    }

    fn add_paths(&mut self, paths: &[PathBuf]) {
        let existing: Vec<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        for path in crate::convert::collect_inputs(paths, &existing) {
            self.items.push(Item {
                path,
                status: Status::Waiting,
            });
        }
    }

    fn options(&self) -> Options {
        Options {
            format: self.settings.format,
            quality: self.settings.quality,
            max_edge: self.settings.resize.then_some(self.settings.max_edge),
            out_dir: self.settings.out_dir.clone(),
        }
    }

    fn waiting(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status == Status::Waiting)
            .count()
    }

    fn start(&mut self, ctx: &egui::Context) {
        let allowance = license::run_allowance(self.waiting(), self.licensed_to.is_some());
        let mut jobs = VecDeque::new();
        for (index, item) in self.items.iter_mut().enumerate() {
            if jobs.len() == allowance {
                break;
            }
            if item.status == Status::Waiting {
                item.status = Status::Converting;
                jobs.push_back((index, item.path.clone()));
            }
        }
        if jobs.is_empty() {
            return;
        }
        self.in_flight = jobs.len();
        self.run_total = jobs.len();
        let (sender, receiver) = channel();
        self.results = Some(receiver);
        spawn_workers(jobs, self.options(), sender, ctx.clone());
    }

    fn collect_results(&mut self) {
        let Some(receiver) = &self.results else {
            return;
        };
        while let Ok((index, result)) = receiver.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            if let Some(item) = self.items.get_mut(index) {
                item.status = match result {
                    Ok(path) => {
                        self.last_output_dir = path.parent().map(Path::to_path_buf);
                        Status::Done(path)
                    }
                    Err(error) => Status::Failed(error),
                };
            }
        }
        if self.in_flight == 0 {
            self.results = None;
        }
    }

    fn retry_failed(&mut self) {
        for item in &mut self.items {
            if matches!(item.status, Status::Failed(_)) {
                item.status = Status::Waiting;
            }
        }
    }

    fn missing_codecs(&self) -> Vec<Codec> {
        let mut codecs = Vec::new();
        for item in &self.items {
            if let Status::Failed(ConvertError::MissingCodec(codec)) = item.status
                && !codecs.contains(&codec)
            {
                codecs.push(codec);
            }
        }
        codecs
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        ui.horizontal(|ui| {
            let idle = !self.busy();
            if ui
                .add_enabled(idle, egui::Button::new(t.add_photos))
                .clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .add_filter(
                        t.images,
                        &[
                            "heic", "heif", "hif", "avif", "jxl", "webp", "jpg", "jpeg", "jfif",
                            "png", "bmp", "tif", "tiff", "gif",
                        ],
                    )
                    .add_filter(t.all_files, &["*"])
                    .pick_files()
            {
                self.add_paths(&files);
            }
            if ui
                .add_enabled(idle, egui::Button::new(t.add_folder))
                .clicked()
                && let Some(folder) = rfd::FileDialog::new().pick_folder()
            {
                self.add_paths(&[folder]);
            }
            if !self.items.is_empty()
                && ui
                    .add_enabled(idle, egui::Button::new(t.clear).frame(false))
                    .clicked()
            {
                self.items.clear();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(email) = &self.licensed_to {
                    ui.label(
                        RichText::new(fill(t.unlocked_for, &[("email", email)]))
                            .small()
                            .color(p.muted),
                    );
                } else if license::paid_tier_enabled() {
                    let unlock = egui::Button::new(RichText::new(t.unlock).color(p.accent))
                        .stroke(Stroke::new(1.0, p.accent));
                    if ui.add(unlock).clicked() {
                        self.unlock_open = true;
                    }
                    ui.label(
                        RichText::new(fill(
                            t.free_limit,
                            &[("n", &license::FREE_BATCH.to_string())],
                        ))
                        .small()
                        .color(p.muted),
                    );
                }
            });
        });
    }

    /// The empty state: one big target that says what the app wants.
    fn drop_zone(&self, ui: &mut egui::Ui, p: Palette, hovering: bool) {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            theme::CARD_RADIUS,
            if hovering { p.accent_soft } else { p.surface },
        );
        painter.rect_stroke(
            rect,
            theme::CARD_RADIUS,
            if hovering {
                Stroke::new(2.0, p.accent)
            } else {
                Stroke::new(1.0, p.border)
            },
            StrokeKind::Inside,
        );
        let centre = rect.center();
        // A down arrow in a soft disc: "put things here".
        let disc = centre - egui::vec2(0.0, 44.0);
        painter.circle_filled(disc, 30.0, if hovering { p.surface } else { p.accent_soft });
        let arrow = Stroke::new(2.5, p.accent);
        painter.line_segment([disc - egui::vec2(0.0, 12.0), disc + egui::vec2(0.0, 11.0)], arrow);
        painter.line_segment([disc + egui::vec2(-9.0, 2.0), disc + egui::vec2(0.0, 11.5)], arrow);
        painter.line_segment([disc + egui::vec2(9.0, 2.0), disc + egui::vec2(0.0, 11.5)], arrow);
        painter.text(
            centre + egui::vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            self.t.drop_here,
            FontId::new(19.0, theme::semibold()),
            p.text,
        );
        painter.text(
            centre + egui::vec2(0.0, 46.0),
            Align2::CENTER_CENTER,
            FORMATS,
            FontId::new(12.5, egui::FontFamily::Proportional),
            p.muted,
        );
    }

    fn list(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) && !self.busy();
        if self.items.is_empty() {
            self.drop_zone(ui, p, hovering);
            return;
        }
        let mut remove = None;
        let idle = !self.busy();
        let mut frame = theme::card(p).inner_margin(egui::Margin::symmetric(0, 4));
        if hovering {
            frame = frame.stroke(Stroke::new(2.0, p.accent));
        }
        frame.show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let last = self.items.len() - 1;
                    for (index, item) in self.items.iter().enumerate() {
                        let width = ui.available_width();
                        let row = ui.allocate_ui_with_layout(
                            egui::vec2(width, ROW_HEIGHT),
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                ui.add_space(8.0);
                                let close = egui::Button::new(RichText::new("✖").color(p.muted))
                                    .frame(false);
                                if ui
                                    .add_enabled(idle, close)
                                    .on_hover_text(t.remove_hint)
                                    .clicked()
                                {
                                    remove = Some(index);
                                }
                                status_ui(ui, &item.status, p, t);
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    ui.add_space(14.0);
                                    let extension = item
                                        .path
                                        .extension()
                                        .map(|e| e.to_string_lossy().to_uppercase())
                                        .unwrap_or_default();
                                    theme::badge(ui, &extension, p.accent, p.accent_soft);
                                    let name = item
                                        .path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    ui.add(egui::Label::new(name).truncate())
                                        .on_hover_text(item.path.display().to_string());
                                });
                            },
                        );
                        if index != last {
                            let line = row.response.rect;
                            ui.painter().hline(
                                (line.left() + 14.0)..=(line.right() - 14.0),
                                line.bottom(),
                                Stroke::new(1.0, p.border),
                            );
                        }
                    }
                });
        });
        if let Some(index) = remove {
            self.items.remove(index);
        }
    }

    fn codec_help(&mut self, ui: &mut egui::Ui, p: Palette) {
        let codecs = self.missing_codecs();
        if codecs.is_empty() {
            return;
        }
        let t = self.t;
        theme::card(p)
            .stroke(Stroke::new(1.0, p.warn))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for codec in codecs {
                    ui.label(match codec {
                        Codec::Heic => t.need_heic,
                        Codec::Avif => t.need_avif,
                        Codec::Jxl => t.need_jxl,
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (name, product, free) in codec.store_products() {
                            let label =
                                fill(if *free { t.get_free } else { t.get }, &[("name", name)]);
                            if ui.button(label).clicked() {
                                open(&format!("ms-windows-store://pdp/?ProductId={product}"));
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.install_then).color(p.muted));
                    if ui
                        .add_enabled(!self.busy(), egui::Button::new(t.try_again))
                        .clicked()
                    {
                        self.retry_failed();
                    }
                });
            });
        ui.add_space(4.0);
    }

    fn options_card(&mut self, ui: &mut egui::Ui, p: Palette) {
        let before = (
            self.settings.format,
            self.settings.quality,
            self.settings.resize,
            self.settings.max_edge,
        );
        let t = self.t;
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let right_edge = ui.max_rect().right();
            ui.add_enabled_ui(!self.busy(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.convert_to).color(p.muted));
                    // The three formats side by side: one click, no menu.
                    for format in Format::ALL {
                        let selected = self.settings.format == format;
                        let text = RichText::new(self.format_label(format)).color(if selected {
                            p.on_accent
                        } else {
                            p.text
                        });
                        let mut button = egui::Button::new(text);
                        if selected {
                            button = button.fill(p.accent).stroke(Stroke::NONE);
                        }
                        if ui.add(button).clicked() {
                            self.settings.format = format;
                        }
                    }
                    if self.settings.format == Format::Jpg {
                        ui.add_space(6.0);
                        // Long labels (Russian) leave less room: the slider
                        // gives way so the row stays inside the card.
                        let label_width = ui
                            .painter()
                            .layout_no_wrap(
                                t.quality.to_owned(),
                                egui::TextStyle::Body.resolve(ui.style()),
                                p.muted,
                            )
                            .size()
                            .x;
                        ui.spacing_mut().slider_width =
                            (right_edge - ui.cursor().left() - label_width - 84.0).clamp(70.0, 150.0);
                        ui.add(
                            egui::Slider::new(&mut self.settings.quality, 40..=100)
                                .text(RichText::new(t.quality).color(p.muted)),
                        );
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    theme::checkbox(ui, &mut self.settings.resize, t.shrink);
                    ui.add_enabled(
                        self.settings.resize,
                        egui::DragValue::new(&mut self.settings.max_edge)
                            .range(64..=20000)
                            .speed(8)
                            .suffix(" px"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t.save_to).color(p.muted));
                    let same = self.settings.out_dir.is_none();
                    if ui.radio(same, t.same_folder).clicked() && !same {
                        self.settings.out_dir = None;
                        self.save();
                    }
                    let label = match &self.settings.out_dir {
                        Some(dir) => dir.display().to_string(),
                        None => t.another_folder.to_owned(),
                    };
                    if ui.radio(!same, label).clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        self.settings.out_dir = Some(dir);
                        self.save();
                    }
                });
            });
        });
        let after = (
            self.settings.format,
            self.settings.quality,
            self.settings.resize,
            self.settings.max_edge,
        );
        if before != after {
            self.save();
        }
    }

    fn action_row(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        ui.horizontal(|ui| {
            let waiting = self.waiting();
            let allowance = license::run_allowance(waiting, self.licensed_to.is_some());
            let label = if self.busy() {
                fill(t.converting_left, &[("n", &self.in_flight.to_string())])
            } else if allowance < waiting {
                fill(
                    t.convert_next,
                    &[("a", &allowance.to_string()), ("b", &waiting.to_string())],
                )
            } else if waiting == 0 {
                t.convert.to_owned()
            } else if waiting == 1 {
                t.convert_one.to_owned()
            } else {
                fill(t.convert_n, &[("n", &waiting.to_string())])
            };
            let enabled = !self.busy() && waiting > 0;
            if ui
                .add_enabled(enabled, theme::primary_button(p, label, enabled))
                .clicked()
            {
                self.start(ui.ctx());
            }
            if let Some(dir) = &self.last_output_dir
                && ui.button(t.open_folder).clicked()
            {
                open(&dir.display().to_string());
            }
            let done = self
                .items
                .iter()
                .filter(|i| matches!(i.status, Status::Done(_)))
                .count();
            if done > 0 {
                ui.label(
                    RichText::new(fill(t.n_converted, &[("n", &done.to_string())]))
                        .color(p.success),
                );
            }
        });
        if self.busy() && self.run_total > 0 {
            let progress = (self.run_total - self.in_flight) as f32 / self.run_total as f32;
            ui.add(
                egui::ProgressBar::new(progress)
                    .desired_height(4.0)
                    .corner_radius(2)
                    .fill(p.accent),
            );
        }
    }

    fn unlock_window(&mut self, ctx: &egui::Context, p: Palette) {
        if !self.unlock_open {
            return;
        }
        let mut open_flag = true;
        let t = self.t;
        egui::Window::new(RichText::new(t.unlock_title).family(theme::semibold()))
            .id(egui::Id::new("unlock"))
            .open(&mut open_flag)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(18)),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(fill(
                    t.unlock_body,
                    &[("n", &license::FREE_BATCH.to_string())],
                ));
                ui.add_space(4.0);
                if ui.add(theme::primary_button(p, t.buy, true)).clicked() {
                    open(license::BUY_URL);
                }
                ui.add_space(10.0);
                ui.label(RichText::new(t.paste_key).color(p.muted));
                ui.add(
                    egui::TextEdit::singleline(&mut self.key_entry)
                        .hint_text("MAC1.…")
                        .desired_width(f32::INFINITY),
                );
                if ui.button(t.activate).clicked() {
                    match license::verify(&self.key_entry) {
                        Some(email) => {
                            self.settings.license_key = self.key_entry.trim().to_owned();
                            self.save();
                            self.key_message = Some(fill(t.unlocked_thanks, &[("email", &email)]));
                            self.licensed_to = Some(email);
                        }
                        None => self.key_message = Some(t.bad_key.to_owned()),
                    }
                }
                if let Some(message) = &self.key_message {
                    let colour = if self.licensed_to.is_some() {
                        p.success
                    } else {
                        p.error
                    };
                    ui.label(RichText::new(message).color(colour));
                }
            });
        if !open_flag {
            self.unlock_open = false;
            self.key_message = None;
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.horizontal(|ui| {
            let mut auto_update = self.settings.auto_update;
            let label = RichText::new(self.t.auto_update).small().color(p.muted);
            if theme::checkbox(ui, &mut auto_update, label)
                .on_hover_text(self.t.auto_update_hint)
                .changed()
            {
                self.settings.auto_update = auto_update;
                crate::settings::save(&self.settings);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("v{}", self.version))
                        .small()
                        .color(p.muted),
                );
                let mut chosen = self.lang;
                egui::ComboBox::from_id_salt("language")
                    .selected_text(RichText::new(self.lang.name()).small())
                    .show_ui(ui, |ui| {
                        for lang in Lang::ALL {
                            ui.selectable_value(&mut chosen, lang, lang.name());
                        }
                    })
                    .response
                    .on_hover_text(self.t.language);
                if chosen != self.lang {
                    self.set_language(chosen, ui.ctx());
                }
            });
        });
    }
}

/// The right-hand side of a file row: where that file has got to.
fn status_ui(ui: &mut egui::Ui, status: &Status, p: Palette, t: &Strings) {
    match status {
        Status::Waiting => {
            ui.label(RichText::new(t.waiting).color(p.muted));
        }
        Status::Converting => {
            ui.label(RichText::new(t.converting).color(p.accent));
            ui.add(egui::Spinner::new().size(14.0).color(p.accent));
        }
        Status::Done(path) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.add(egui::Label::new(RichText::new(name).color(p.success)).truncate())
                .on_hover_text(path.display().to_string());
            ui.label(RichText::new("✔").color(p.success));
        }
        Status::Failed(ConvertError::MissingCodec(_)) => {
            ui.label(RichText::new(t.needs_extension).color(p.warn));
        }
        Status::Failed(ConvertError::Other(message)) => {
            ui.label(RichText::new(short_error(message, t.cant_read)).color(p.error))
                .on_hover_text(message);
        }
    }
}

/// One line, short enough to sit beside the file name. Details from Windows
/// and the decoders stay as they are; `fallback` covers a missing message.
fn short_error(message: &str, fallback: &str) -> String {
    let line = message.lines().next().unwrap_or("").trim();
    let text = if line.is_empty() || line == PANIC_MESSAGE {
        fallback
    } else {
        line
    };
    let mut short: String = text.chars().take(60).collect();
    if text.chars().count() > 60 {
        short.push('…');
    }
    short
}

fn spawn_workers(
    jobs: VecDeque<(usize, PathBuf)>,
    options: Options,
    sender: Sender<JobResult>,
    ctx: egui::Context,
) {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8)
        .min(jobs.len());
    let jobs = Arc::new(Mutex::new(jobs));
    for _ in 0..threads {
        let (jobs, options, sender, ctx) =
            (jobs.clone(), options.clone(), sender.clone(), ctx.clone());
        std::thread::spawn(move || {
            loop {
                let next = jobs.lock().ok().and_then(|mut queue| queue.pop_front());
                let Some((index, path)) = next else { break };
                // A decoder panic must not leave the row stuck on "Converting".
                let result = std::panic::catch_unwind(|| crate::convert::convert_file(&path, &options))
                    .unwrap_or_else(|_| Err(ConvertError::Other(PANIC_MESSAGE.to_owned())));
                if sender.send((index, result)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }
}

/// Opens a folder, web page or Store page with whatever Windows uses for it.
fn open(target: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui_root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.collect_results();
        if !self.busy() {
            let dropped: Vec<PathBuf> = ui_root.ctx().input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect()
            });
            if !dropped.is_empty() {
                self.add_paths(&dropped);
            }
        }

        let p = theme::palette(ui_root.ctx());
        let bare = |top: i8, bottom: i8| {
            egui::Frame::new().fill(p.bg).inner_margin(egui::Margin {
                left: GUTTER,
                right: GUTTER,
                top,
                bottom,
            })
        };
        egui::Panel::bottom("footer")
            .frame(bare(6, 8))
            .show_separator_line(false)
            .show(ui_root, |ui| self.footer(ui, p));
        egui::Panel::bottom("controls")
            .frame(bare(10, 6))
            .show_separator_line(false)
            .show(ui_root, |ui| {
                self.codec_help(ui, p);
                self.options_card(ui, p);
                ui.add_space(4.0);
                self.action_row(ui, p);
            });
        egui::Panel::top("toolbar")
            .frame(bare(14, 10))
            .show_separator_line(false)
            .show(ui_root, |ui| self.toolbar(ui, p));
        egui::CentralPanel::default()
            .frame(bare(0, 0))
            .show(ui_root, |ui| self.list(ui, p));
        let ctx = ui_root.ctx().clone();
        self.unlock_window(&ctx, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_error_is_one_trimmed_line() {
        assert_eq!(short_error("  bad header\nmore", "fallback"), "bad header");
        assert_eq!(short_error("", "fallback"), "fallback");
        assert_eq!(short_error(PANIC_MESSAGE, "fallback"), "fallback");
        let long = "x".repeat(100);
        assert_eq!(short_error(&long, "fallback").chars().count(), 61);
    }
}
