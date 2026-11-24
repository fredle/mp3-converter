//! The window: a queue of video and audio files, the output options, and the worker threads
//! that convert them. The footer with the auto-update toggle and version is
//! required on every factory app. Colours, type and painted pieces come from
//! `theme.rs`; this file only lays out.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, FontId, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, pos2, vec2,
};

use crate::convert::{Codec, ConvertError, Format, Options, format_duration, format_size};
use crate::i18n::{Lang, Strings, fill};
use crate::license;
use crate::mf::{Info, Progress};
use crate::theme::{self, Icon, Palette, Segment};

const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 680.0;
const MIN_SIZE: [f32; 2] = [640.0, 620.0];
const APP_NAME: &str = "MP4 to MP3 & Audio Converter";
/// Marks a decoder crash; shown to the user as the translated "couldn't read".
const PANIC_MESSAGE: &str = "decoder panicked";
/// Space between the window edge and its contents.
const GUTTER: i8 = 16;
const ROW_HEIGHT: f32 = 56.0;
/// Width of one row icon button with its gap.
const ACTION_SLOT: f32 = 32.0;
const FORMATS: [&str; 11] = [
    "MP4", "MOV", "MKV", "AVI", "WMV", "M4A", "AAC", "WAV", "FLAC", "WMA", "MP3",
];
/// How often the window redraws while converting.
const BUSY_REPAINT: Duration = Duration::from_millis(100);

pub fn run(args: Vec<String>) -> eframe::Result<()> {
    let size = if Shots::from_env().is_some() { SHOT_SIZE } else { [WIDTH, HEIGHT] };
    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size(size)
        .with_min_inner_size(MIN_SIZE)
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
    /// Queued or converting; `Item::progress` says which.
    Converting,
    Done(PathBuf),
    Failed(ConvertError),
}

/// What is known about a file's audio before converting it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Probe {
    Pending,
    Known(Info),
    /// Windows could not read it; converting will say why.
    Unknown,
}

struct Item {
    /// Stable across removals, unlike the position in the list.
    id: u64,
    path: PathBuf,
    status: Status,
    probe: Probe,
    /// Bytes of the original, once known.
    size: Option<u64>,
    /// Bytes of the converted file.
    out_size: Option<u64>,
    progress: Arc<Progress>,
}

struct Job {
    id: u64,
    path: PathBuf,
    progress: Arc<Progress>,
}

type JobResult = (u64, Result<(PathBuf, u64), ConvertError>);
type ProbeResult = (u64, Option<Info>, Option<u64>);

/// One press of Convert, until its last file finishes.
struct Run {
    /// Files no worker has started yet; emptied by Cancel.
    queue: Arc<Mutex<VecDeque<Job>>>,
    results: Receiver<JobResult>,
    ids: HashSet<u64>,
    in_flight: usize,
}

/// What the last run did, for the line under the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Summary {
    converted: usize,
    failed: usize,
    in_bytes: u64,
    out_bytes: u64,
}

/// Store screenshots mode: with `MAC_SHOTS=<folder>` set, the app shows three
/// made-up scenes (a queue ready to convert, the same queue converted, the
/// empty window), writes each as `<n>_<w>x<h>.rgba` into the folder and
/// closes. `MAC_SHOTS_LANG=<locale>` picks the language. Nothing is converted
/// and no setting is saved. app-factory's `tools/compose-screenshot.ps1`
/// turns the frames into the Store's images.
struct Shots {
    dir: PathBuf,
    /// 0-based scene being shown.
    scene: usize,
    /// Frames drawn of this scene, so fonts and layout settle before capture.
    frames: u32,
    requested: bool,
}

const SHOT_SIZE: [f32; 2] = [680.0, 640.0];
const SHOT_SCENES: usize = 3;
/// (file name, seconds, bytes) of the made-up queue.
const SHOT_FILES: [(&str, f64, u64); 3] = [
    ("Guitar lesson.mov", 768.0, 432_013_312),
    ("Wedding speech.mp4", 365.0, 195_035_136),
    ("Live set.flac", 3500.0, 357_564_416),
];

impl Shots {
    fn from_env() -> Option<Self> {
        let dir = std::env::var_os("MAC_SHOTS").filter(|d| !d.is_empty())?;
        Some(Self {
            dir: PathBuf::from(dir),
            scene: 0,
            frames: 0,
            requested: false,
        })
    }
}

enum RowAction {
    Remove,
    Retry,
    Play,
    Reveal,
}

pub struct App {
    settings: crate::settings::Settings,
    version: String,
    items: Vec<Item>,
    next_id: u64,
    probe_requests: Sender<(u64, PathBuf)>,
    probe_results: Receiver<ProbeResult>,
    run: Option<Run>,
    summary: Option<Summary>,
    /// Folder of the most recent successful conversion, for "Open folder".
    last_output_dir: Option<PathBuf>,
    licensed_to: Option<String>,
    unlock_open: bool,
    key_entry: String,
    key_message: Option<String>,
    lang: Lang,
    t: &'static Strings,
    shots: Option<Shots>,
}

impl App {
    fn new(args: Vec<String>, ctx: &egui::Context) -> Self {
        let settings = crate::settings::load();
        let licensed_to = license::verify(&settings.license_key);
        let lang = settings.language.unwrap_or_else(Lang::system);
        let shots = Shots::from_env();
        let (settings, licensed_to, lang) = match &shots {
            // Screenshots show the app as a new user first sees it.
            Some(_) => (
                crate::settings::Settings::default(),
                None,
                std::env::var("MAC_SHOTS_LANG")
                    .map_or(Lang::En, |locale| Lang::from_locale(&locale)),
            ),
            None => (settings, licensed_to, lang),
        };
        theme::apply(ctx);
        theme::install_fonts(ctx, &[lang, Lang::system()]);
        // For checking both looks without changing Windows' setting.
        match std::env::var("HWC_THEME").as_deref() {
            Ok("dark") => ctx.set_theme(egui::Theme::Dark),
            Ok("light") => ctx.set_theme(egui::Theme::Light),
            _ => {}
        }
        let (probe_requests, probe_results) = spawn_prober(ctx.clone());
        let mut app = Self {
            lang,
            t: lang.strings(),
            settings,
            version: crate::update::current_version(),
            items: Vec::new(),
            next_id: 0,
            probe_requests,
            probe_results,
            run: None,
            summary: None,
            last_output_dir: None,
            licensed_to,
            unlock_open: false,
            key_entry: String::new(),
            key_message: None,
            shots,
        };
        if app.shots.is_some() {
            ctx.set_pixels_per_point(1.0);
            // The version being released, not this build's "(dev)".
            app.version = std::env::var("MAC_SHOTS_VERSION")
                .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned());
            return app;
        }
        // Files passed on the command line ("Open with", or dropped on the exe).
        let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
        app.add_paths(&paths);
        app
    }

    fn save(&self) {
        if self.shots.is_some() {
            return;
        }
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

    fn busy(&self) -> bool {
        self.run.is_some()
    }

    fn add_paths(&mut self, paths: &[PathBuf]) {
        let existing: Vec<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        for path in crate::convert::collect_inputs(paths, &existing) {
            let id = self.next_id;
            self.next_id += 1;
            // Length and size are read on the prober's thread, so a folder
            // of hundreds of files never stalls the window.
            let _ = self.probe_requests.send((id, path.clone()));
            self.items.push(Item {
                id,
                path,
                status: Status::Waiting,
                probe: Probe::Pending,
                size: None,
                out_size: None,
                progress: Arc::default(),
            });
        }
    }

    fn pick_files(&mut self) {
        let t = self.t;
        if let Some(files) = rfd::FileDialog::new()
            .add_filter(t.media, &crate::convert::extensions())
            .add_filter(t.all_files, &["*"])
            .pick_files()
        {
            self.add_paths(&files);
        }
    }

    fn options(&self) -> Options {
        Options {
            format: self.settings.format,
            kbps: self.settings.kbps,
            mono: self.settings.mono,
            out_dir: self.settings.out_dir.clone(),
        }
    }

    fn waiting(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status == Status::Waiting)
            .count()
    }

    fn allowance(&self) -> usize {
        license::run_allowance(self.waiting(), self.licensed_to.is_some())
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let allowance = self.allowance();
        let mut jobs = VecDeque::new();
        for item in &mut self.items {
            if jobs.len() == allowance {
                break;
            }
            if item.status == Status::Waiting {
                item.status = Status::Converting;
                item.progress = Arc::default();
                jobs.push_back(Job {
                    id: item.id,
                    path: item.path.clone(),
                    progress: item.progress.clone(),
                });
            }
        }
        if jobs.is_empty() {
            return;
        }
        let ids: HashSet<u64> = jobs.iter().map(|job| job.id).collect();
        let (sender, receiver) = channel();
        let queue = Arc::new(Mutex::new(jobs));
        spawn_workers(queue.clone(), ids.len(), self.options(), sender, ctx.clone());
        self.summary = None;
        self.run = Some(Run {
            queue,
            results: receiver,
            in_flight: ids.len(),
            ids,
        });
    }

    /// Stops the run: files not started go back to waiting, and the ones
    /// converting stop at their next block and delete what they wrote.
    fn cancel(&mut self) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let unstarted: HashSet<u64> = run
            .queue
            .lock()
            .map(|mut queue| queue.drain(..).map(|job| job.id).collect())
            .unwrap_or_default();
        run.in_flight = run.in_flight.saturating_sub(unstarted.len());
        for item in &mut self.items {
            if unstarted.contains(&item.id) {
                item.status = Status::Waiting;
            } else if item.status == Status::Converting && run.ids.contains(&item.id) {
                item.progress.cancel();
            }
        }
    }

    /// Sets up scene `scene` of the Store screenshots.
    fn show_scene(&mut self, scene: usize) {
        self.items.clear();
        self.summary = None;
        self.last_output_dir = None;
        if scene == 2 {
            return;
        }
        let folder = Path::new("C:\\Media");
        let options = self.options();
        for (id, (name, seconds, bytes)) in SHOT_FILES.into_iter().enumerate() {
            let path = folder.join(name);
            let info = Info {
                seconds: Some(seconds),
                rate: 48_000,
                channels: 2,
            };
            let (status, out_size) = if scene == 1 {
                let out = crate::convert::output_path(&path, &options, |_| false);
                let size = crate::convert::estimate_bytes(options.format, options.kbps, false, &info);
                (Status::Done(out), size)
            } else {
                (Status::Waiting, None)
            };
            self.items.push(Item {
                id: id as u64,
                path,
                status,
                probe: Probe::Known(info),
                size: Some(bytes),
                out_size,
                progress: Arc::default(),
            });
        }
        if scene == 1 {
            self.summary = Some(summarise(
                self.items.iter().map(|i| (&i.status, i.size, i.out_size)),
            ));
            self.last_output_dir = Some(folder.to_path_buf());
        }
    }

    /// Steps through the Store screenshots: show a scene, let it settle,
    /// capture it, write it, move on; close after the last.
    fn drive_shots(&mut self, ctx: &egui::Context) {
        let Some(shots) = self.shots.as_mut() else {
            return;
        };
        let image = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let name = format!("{}_{}x{}.rgba", shots.scene + 1, image.size[0], image.size[1]);
            let _ = std::fs::create_dir_all(&shots.dir);
            let _ = std::fs::write(shots.dir.join(name), image.as_raw());
            shots.scene += 1;
            shots.frames = 0;
            shots.requested = false;
        }
        let (scene, frames, requested) = (shots.scene, shots.frames, shots.requested);
        if scene >= SHOT_SCENES {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if frames == 0 {
            self.show_scene(scene);
        } else if frames >= 5 && !requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if let Some(shots) = self.shots.as_mut() {
            shots.requested |= frames >= 5;
            shots.frames += 1;
        }
        ctx.request_repaint();
    }

    fn collect_probes(&mut self) {
        while let Ok((id, info, size)) = self.probe_results.try_recv() {
            if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                item.probe = info.map_or(Probe::Unknown, Probe::Known);
                item.size = size;
            }
        }
    }

    fn collect_results(&mut self, ctx: &egui::Context) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let mut finished = Vec::new();
        while let Ok(result) = run.results.try_recv() {
            run.in_flight = run.in_flight.saturating_sub(1);
            finished.push(result);
        }
        let over = run.in_flight == 0;
        for (id, result) in finished {
            let Some(item) = self.items.iter_mut().find(|i| i.id == id) else {
                continue;
            };
            item.status = match result {
                Ok((path, bytes)) => {
                    item.out_size = Some(bytes);
                    self.last_output_dir = path.parent().map(Path::to_path_buf);
                    Status::Done(path)
                }
                Err(ConvertError::Cancelled) => Status::Waiting,
                Err(error) => Status::Failed(error),
            };
        }
        if !over {
            return;
        }
        let Some(run) = self.run.take() else { return };
        let summary = summarise(
            self.items
                .iter()
                .filter(|i| run.ids.contains(&i.id))
                .map(|i| (&i.status, i.size, i.out_size)),
        );
        self.summary = (summary.converted + summary.failed > 0).then_some(summary);
        // Flash the taskbar button when the run ends behind other windows.
        if self.summary.is_some() && !ctx.input(|i| i.focused) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }
    }

    /// The whole run's progress, each file weighted by how far it has got.
    fn run_progress(&self) -> f32 {
        let Some(run) = &self.run else { return 0.0 };
        let parts: Vec<f32> = self
            .items
            .iter()
            .filter(|i| run.ids.contains(&i.id))
            .map(|i| match i.status {
                Status::Done(_) | Status::Failed(_) => 1.0,
                Status::Converting => i.progress.fraction().unwrap_or(0.0),
                Status::Waiting => 0.0,
            })
            .collect();
        overall(&parts)
    }

    /// Output size of the files the next run will convert, when every one
    /// of them can be estimated; `None` rather than a wrong number.
    fn estimate(&self) -> Option<u64> {
        let (format, kbps, mono) = (self.settings.format, self.settings.kbps, self.settings.mono);
        let mut total = 0;
        let mut any = false;
        for item in self
            .items
            .iter()
            .filter(|i| i.status == Status::Waiting)
            .take(self.allowance())
        {
            match item.probe {
                Probe::Pending => return None,
                Probe::Unknown => {}
                Probe::Known(info) => {
                    total += crate::convert::estimate_bytes(format, kbps, mono, &info)?;
                    any = true;
                }
            }
        }
        any.then_some(total)
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

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Each key's own modifiers, not the state at the end of the frame: a
        // quick Ctrl+O can press and release Ctrl within one frame.
        let pressed = |i: &egui::InputState, key: Key, with: Modifiers| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. }
                    if *k == key && modifiers.matches_exact(with))
            })
        };
        let (open_files, enter, escape) = ctx.input(|i| {
            (
                pressed(i, Key::O, Modifiers::COMMAND),
                pressed(i, Key::Enter, Modifiers::NONE),
                pressed(i, Key::Escape, Modifiers::NONE),
            )
        });
        if escape {
            if self.unlock_open {
                self.unlock_open = false;
                self.key_message = None;
            } else if self.busy() {
                self.cancel();
            }
            return;
        }
        if self.unlock_open || self.busy() {
            return;
        }
        if open_files {
            ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O));
            self.pick_files();
        } else if enter
            && self.waiting() > 0
            && ctx.memory(|m| m.focused().is_none())
            && !ctx.any_popup_open()
        {
            self.start(ctx);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        ui.horizontal(|ui| {
            let idle = !self.busy();
            if ui
                .add_enabled(idle, theme::secondary_button(p, t.add_files))
                .on_hover_text("Ctrl+O")
                .clicked()
            {
                self.pick_files();
            }
            if ui
                .add_enabled(idle, theme::secondary_button(p, t.add_folder))
                .clicked()
                && let Some(folder) = rfd::FileDialog::new().pick_folder()
            {
                self.add_paths(&[folder]);
            }
            if !self.items.is_empty()
                && ui
                    .add_enabled(
                        idle,
                        egui::Button::new(RichText::new(t.clear).color(p.muted))
                            .frame(false)
                            .min_size(vec2(0.0, theme::CONTROL_HEIGHT)),
                    )
                    .clicked()
            {
                self.items.clear();
                self.summary = None;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(email) = &self.licensed_to {
                    theme::pill_widget(
                        ui,
                        &fill(t.unlocked_for, &[("email", email)]),
                        p.success,
                        p.success_soft,
                    );
                } else if license::paid_tier_enabled() {
                    let unlock = egui::Button::new(
                        RichText::new(t.unlock).color(p.accent_text).family(theme::semibold()),
                    )
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.accent))
                    .min_size(vec2(0.0, theme::CONTROL_HEIGHT));
                    let limit = fill(t.free_limit, &[("n", &license::FREE_BATCH.to_string())]);
                    let response = ui.add(unlock).on_hover_text(&limit);
                    if response.clicked() {
                        self.unlock_open = true;
                    }
                    // The limit as a pill when there is room for it; long
                    // translations keep it in the tooltip instead.
                    if ui.available_width() >= theme::pill_width(ui, &limit) + 8.0 {
                        theme::pill_widget(ui, &limit, p.muted, p.track);
                    }
                }
            });
        });
    }

    /// The empty state: one big target that says what the app wants.
    fn drop_zone(&mut self, ui: &mut egui::Ui, p: Palette, hovering: bool) {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let ctx = ui.ctx().clone();
        let glow = ctx.animate_bool_with_time(egui::Id::new("drop-glow"), hovering, 0.18);
        let painter = ui.painter().clone();
        painter.rect_filled(rect, theme::CARD_RADIUS, p.surface.lerp_to_gamma(p.accent_soft, glow));
        let border = p.border.lerp_to_gamma(p.accent, glow);
        theme::dashed_rect(
            &painter,
            rect.shrink(1.0),
            theme::CARD_RADIUS as f32,
            Stroke::new(1.5 + glow * 0.5, border),
        );

        // Chips for the formats, wrapped into centred lines.
        let chip_font = FontId::new(11.5, theme::semibold());
        let max_line = (rect.width() - 64.0).clamp(120.0, 620.0);
        let chips: Vec<_> = FORMATS
            .iter()
            .map(|f| painter.layout_no_wrap((*f).to_owned(), chip_font.clone(), p.muted))
            .collect();
        let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
        let mut line_width = 0.0;
        for (i, chip) in chips.iter().enumerate() {
            let w = chip.size().x + 16.0;
            if line_width + w > max_line && lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(Vec::new());
                line_width = 0.0;
            }
            if let Some(line) = lines.last_mut() {
                line.push(i);
            }
            line_width += w + 6.0;
        }
        let chip_h = 22.0;
        let chips_h = lines.len() as f32 * (chip_h + 6.0) - 6.0;

        let wave_h = 56.0;
        let headline_h = 26.0;
        let button_h = 38.0;
        let total = wave_h + 20.0 + headline_h + 14.0 + chips_h + 22.0 + button_h;
        let mut y = rect.center().y - total / 2.0;

        let time = ctx.input(|i| i.time) as f32;
        let wave_rect = Rect::from_center_size(pos2(rect.center().x, y + wave_h / 2.0), vec2(200.0, wave_h));
        let wave_colour = p.accent.gamma_multiply(0.55 + 0.45 * glow);
        theme::waveform(&painter, wave_rect, wave_colour, time, glow);
        if hovering || glow > 0.0 {
            ctx.request_repaint();
        }
        y += wave_h + 20.0;

        let headline = theme::truncated(
            &painter,
            self.t.drop_here,
            FontId::new(19.0, theme::semibold()),
            p.text,
            rect.width() - 48.0,
        );
        painter.galley(pos2(rect.center().x - headline.size().x / 2.0, y), headline, p.text);
        y += headline_h + 14.0;

        for line in &lines {
            let width: f32 = line.iter().map(|i| chips[*i].size().x + 16.0).sum::<f32>()
                + 6.0 * (line.len().saturating_sub(1)) as f32;
            let mut x = rect.center().x - width / 2.0;
            for i in line {
                let chip = &chips[*i];
                let r = Rect::from_min_size(pos2(x, y), vec2(chip.size().x + 16.0, chip_h));
                painter.rect_filled(r, 6, p.bg.lerp_to_gamma(p.surface, glow));
                painter.rect_stroke(r, 6, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                painter.galley(r.center() - chip.size() / 2.0, chip.clone(), p.muted);
                x += r.width() + 6.0;
            }
            y += chip_h + 6.0;
        }
        y += 16.0;

        let label = RichText::new(self.t.add_files)
            .family(theme::semibold())
            .color(p.on_accent);
        let text_w = painter
            .layout_no_wrap(self.t.add_files.to_owned(), FontId::new(14.5, theme::semibold()), p.text)
            .size()
            .x;
        let button_w = (text_w + 40.0).min(rect.width() - 48.0);
        let button_rect = Rect::from_min_size(pos2(rect.center().x - button_w / 2.0, y), vec2(button_w, button_h));
        let button = egui::Button::new(label)
            .fill(p.accent)
            .stroke(Stroke::NONE)
            .corner_radius(theme::CONTROL_RADIUS);
        if ui.put(button_rect, button).on_hover_text("Ctrl+O").clicked() {
            self.pick_files();
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, p: Palette) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty()) && !self.busy();
        if self.items.is_empty() {
            self.drop_zone(ui, p, hovering);
            return;
        }
        let t = self.t;
        let idle = !self.busy();
        let time = ui.ctx().input(|i| i.time);
        let mut actions: Vec<(u64, RowAction)> = Vec::new();
        let slots = self
            .items
            .iter()
            .map(|i| action_count(&i.status, idle))
            .max()
            .unwrap_or(0);
        let actions_width = slots as f32 * ACTION_SLOT;
        let mut frame = theme::card(p).inner_margin(egui::Margin::symmetric(0, 4));
        if hovering {
            frame = frame
                .stroke(Stroke::new(2.0, p.accent))
                .fill(p.surface.lerp_to_gamma(p.accent_soft, 0.4));
        }
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let count = self.items.len();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, ROW_HEIGHT, count, |ui, range| {
                    for index in range {
                        let item = &self.items[index];
                        let last = index + 1 == count;
                        if let Some(action) = row(ui, item, p, t, idle, last, time, actions_width) {
                            actions.push((item.id, action));
                        }
                    }
                });
        });
        for (id, action) in actions {
            let Some(position) = self.items.iter().position(|i| i.id == id) else {
                continue;
            };
            match action {
                RowAction::Remove => {
                    self.items.remove(position);
                }
                RowAction::Retry => self.items[position].status = Status::Waiting,
                RowAction::Play => {
                    if let Status::Done(path) = &self.items[position].status {
                        open(&path.display().to_string());
                    }
                }
                RowAction::Reveal => {
                    if let Status::Done(path) = &self.items[position].status {
                        reveal(path);
                    }
                }
            }
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
                        Codec::WebMedia => t.need_webmedia,
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (name, product, free) in codec.store_products() {
                            let label =
                                fill(if *free { t.get_free } else { t.get }, &[("name", name)]);
                            if ui.add(theme::secondary_button(p, label)).clicked() {
                                open(&format!("ms-windows-store://pdp/?ProductId={product}"));
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.install_then).color(p.muted));
                    if ui
                        .add_enabled(!self.busy(), theme::secondary_button(p, t.try_again))
                        .clicked()
                    {
                        self.retry_failed();
                    }
                });
            });
        ui.add_space(8.0);
    }

    fn options_card(&mut self, ui: &mut egui::Ui, p: Palette) {
        let before = (self.settings.format, self.settings.kbps, self.settings.mono);
        let t = self.t;
        let idle = !self.busy();
        let estimate = self.estimate();
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            let label_w = label_width(ui, &[t.convert_to, t.quality, t.save_to]);
            ui.add_enabled_ui(idle, |ui| {
                option_row(ui, label_w, t.convert_to, theme::CONTROL_HEIGHT, p, |ui| {
                    let segments: Vec<Segment> = Format::ALL
                        .iter()
                        .map(|f| Segment {
                            label: f.label(),
                            detail: None,
                        })
                        .collect();
                    let selected = Format::ALL
                        .iter()
                        .position(|f| *f == self.settings.format)
                        .unwrap_or(0);
                    if let Some(i) = theme::segmented(ui, egui::Id::new("format"), &segments, selected, p) {
                        self.settings.format = Format::ALL[i];
                    }
                });
                let format = self.settings.format;
                let lossy = !format.bitrates().is_empty();
                let height = if lossy { 44.0 } else { theme::CONTROL_HEIGHT };
                option_row(ui, label_w, t.quality, height, p, |ui| {
                    if !lossy {
                        ui.add(egui::Label::new(RichText::new(t.lossless).color(p.muted)).truncate());
                        return;
                    }
                    let names = [t.q_smaller, t.q_standard, t.q_high, t.q_best];
                    let rates = if self.settings.mono {
                        format.mono_bitrates()
                    } else {
                        format.bitrates()
                    };
                    let segments: Vec<Segment> = names
                        .iter()
                        .zip(rates)
                        .map(|(name, rate)| Segment {
                            label: name,
                            detail: Some(format!("{rate} kbps")),
                        })
                        .collect();
                    let tier = format.tier(self.settings.kbps);
                    if let Some(i) = theme::segmented(ui, egui::Id::new("quality"), &segments, tier, p) {
                        self.settings.kbps = format.bitrates()[i];
                    }
                });
                option_row(ui, label_w, "", theme::CONTROL_HEIGHT, p, |ui| {
                    let mut mono = self.settings.mono;
                    if theme::checkbox(ui, &mut mono, t.mono)
                        .on_hover_text(t.mono_hint)
                        .changed()
                    {
                        self.settings.mono = mono;
                    }
                    if let (Some(bytes), true) = (estimate, idle) {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let text = fill(t.estimate, &[("size", &format_size(bytes))]);
                            ui.add(egui::Label::new(RichText::new(text).color(p.muted)).truncate());
                        });
                    }
                });
            });
            ui.add_enabled_ui(idle, |ui| {
            option_row(ui, label_w, t.save_to, theme::CONTROL_HEIGHT, p, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(idle, theme::secondary_button(p, t.change_folder))
                        .clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        self.settings.out_dir = Some(dir);
                        self.save();
                    }
                    if self.settings.out_dir.is_some() && idle {
                        let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                        if theme::icon_button(ui, rect, egui::Id::new("same-folder"), Icon::Close, p)
                            .on_hover_text(t.use_same_folder)
                            .clicked()
                        {
                            self.settings.out_dir = None;
                            self.save();
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        match &self.settings.out_dir {
                            Some(dir) => {
                                let text = dir.display().to_string();
                                ui.add(egui::Label::new(RichText::new(&text).color(p.text)).truncate())
                                    .on_hover_text(text);
                            }
                            None => {
                                ui.add(
                                    egui::Label::new(RichText::new(t.same_folder).color(p.text))
                                        .truncate(),
                                );
                            }
                        }
                    });
                });
            });
            });
        });
        let after = (self.settings.format, self.settings.kbps, self.settings.mono);
        if before != after {
            self.save();
        }
    }

    fn action_area(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        let width = ui.available_width();
        if let (Some(summary), false) = (self.summary, self.busy()) {
            let done = self.items.iter().any(|i| matches!(i.status, Status::Done(_)));
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if done && ui.add(theme::secondary_button(p, t.clear_finished)).clicked() {
                        self.items.retain(|i| !matches!(i.status, Status::Done(_)));
                    }
                    if let Some(dir) = &self.last_output_dir
                        && ui.add(theme::secondary_button(p, t.open_folder)).clicked()
                    {
                        open(&dir.display().to_string());
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let (dot_colour, colour) = if summary.failed == 0 {
                            (p.success, p.success)
                        } else {
                            (p.error, p.text)
                        };
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.0, dot_colour);
                        ui.add(
                            egui::Label::new(RichText::new(summary_text(t, &summary)).color(colour))
                                .truncate(),
                        );
                    });
                });
            });
            ui.add_space(4.0);
        }
        if let Some(run) = &self.run {
            let fraction = self.run_progress();
            let in_flight = run.in_flight;
            ui.horizontal(|ui| {
                let cancel_w = 120.0;
                let (rect, _) = ui.allocate_exact_size(
                    vec2((width - cancel_w - 8.0).max(80.0), theme::PRIMARY_HEIGHT),
                    Sense::hover(),
                );
                let painter = ui.painter();
                painter.rect_filled(rect, 10, p.accent_soft);
                let fill_rect = Rect::from_min_size(rect.min, vec2(rect.width() * fraction, rect.height()));
                if fill_rect.width() > 1.0 {
                    painter.rect_filled(fill_rect, 10, p.accent);
                }
                let text = format!(
                    "{} · {}%",
                    fill(t.converting_left, &[("n", &in_flight.to_string())]),
                    (fraction * 100.0).floor() as u32
                );
                theme::text_over_fill(painter, rect, fill_rect.right(), &text, p.accent_text, p.on_accent);
                let (cancel_rect, _) =
                    ui.allocate_exact_size(vec2(cancel_w, theme::PRIMARY_HEIGHT), Sense::hover());
                let cancel = theme::secondary_button(p, t.cancel).min_size(vec2(cancel_w, theme::PRIMARY_HEIGHT));
                if ui.put(cancel_rect, cancel).on_hover_text("Esc").clicked() {
                    self.cancel();
                }
            });
            return;
        }
        let waiting = self.waiting();
        let allowance = self.allowance();
        let label = if allowance < waiting {
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
        let enabled = waiting > 0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, theme::PRIMARY_HEIGHT), Sense::hover());
        let clicked = ui
            .add_enabled_ui(enabled, |ui| {
                ui.put(rect, theme::primary_button(p, label, enabled, width))
                    .on_hover_text("Enter")
                    .clicked()
            })
            .inner;
        if clicked {
            self.start(ui.ctx());
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
                if ui.add(theme::primary_button(p, t.buy, true, 230.0)).clicked() {
                    open(license::BUY_URL);
                }
                ui.add_space(10.0);
                ui.label(RichText::new(t.paste_key).color(p.muted));
                ui.add(
                    egui::TextEdit::singleline(&mut self.key_entry)
                        .hint_text("MAC1.…")
                        .desired_width(f32::INFINITY),
                );
                if ui.add(theme::secondary_button(p, t.activate)).clicked() {
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

/// Width for the option labels: the longest, within limits, so the
/// controls line up whatever the language.
fn label_width(ui: &egui::Ui, labels: &[&str]) -> f32 {
    labels
        .iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap((*l).to_owned(), theme::body(), egui::Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max)
        .clamp(64.0, 150.0)
        + 12.0
}

/// A muted label in a fixed-width column, then the row's controls.
fn option_row(
    ui: &mut egui::Ui,
    label_w: f32,
    label: &str,
    height: f32,
    p: Palette,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(label_w, height), Sense::hover());
        if !label.is_empty() {
            let galley = theme::truncated(ui.painter(), label, theme::body(), p.muted, label_w - 12.0);
            let pos = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(pos, galley, p.muted);
        }
        ui.spacing_mut().item_spacing.x = 8.0;
        add_contents(ui);
    });
}

/// One file in the list: its tile, name, details, status and actions.
fn row(
    ui: &mut egui::Ui,
    item: &Item,
    p: Palette,
    t: &Strings,
    idle: bool,
    last: bool,
    time: f64,
    actions_width: f32,
) -> Option<RowAction> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    let id = egui::Id::new(("row", item.id));
    let painter = ui.painter().clone();
    if ui.rect_contains_pointer(rect) {
        painter.rect_filled(rect.shrink2(vec2(6.0, 2.0)), 8, p.hover);
    }
    if !last {
        painter.hline(
            (rect.left() + 16.0)..=(rect.right() - 16.0),
            rect.bottom() - 0.5,
            Stroke::new(1.0, p.border),
        );
    }
    let inner = rect.shrink2(vec2(16.0, 0.0));
    let cy = rect.center().y;

    let extension = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    let tile = Rect::from_min_size(pos2(inner.left(), cy - 20.0), vec2(40.0, 40.0));
    theme::file_tile(&painter, tile, crate::convert::is_video(&item.path), &extension, p);

    // Icon buttons from the right edge in.
    let mut action = None;
    let mut slot = inner.right();
    let mut button = |ui: &mut egui::Ui, icon: Icon, tip: &str, what: RowAction| {
        let r = Rect::from_center_size(pos2(slot - 14.0, cy), vec2(28.0, 28.0));
        slot -= ACTION_SLOT;
        if theme::icon_button(ui, r, id.with(tip), icon, p)
            .on_hover_text(tip)
            .clicked()
        {
            action = Some(what);
        }
    };
    if idle && item.status != Status::Converting {
        button(ui, Icon::Close, t.remove_hint, RowAction::Remove);
    }
    match &item.status {
        Status::Done(_) => {
            button(ui, Icon::Folder, t.show_in_folder, RowAction::Reveal);
            button(ui, Icon::Play, t.play, RowAction::Play);
        }
        Status::Failed(_) if idle => button(ui, Icon::Retry, t.try_again, RowAction::Retry),
        _ => {}
    }

    // The status pill, lined up across rows.
    let started = item.progress.started();
    let fraction = item.progress.fraction();
    let (pill_text, fg, bg) = match &item.status {
        Status::Waiting => (t.waiting.to_owned(), p.muted, p.track),
        Status::Converting if !started => (t.waiting.to_owned(), p.muted, p.track),
        Status::Converting => match fraction {
            Some(f) => (format!("{}%", (f * 100.0).floor() as u32), p.accent_text, p.accent_soft),
            None => (t.converting.to_owned(), p.accent_text, p.accent_soft),
        },
        Status::Done(_) => (t.done.to_owned(), p.success, p.success_soft),
        Status::Failed(ConvertError::MissingCodec(_)) => (t.failed.to_owned(), p.warn, p.warn_soft),
        Status::Failed(_) => (t.failed.to_owned(), p.error, p.error_soft),
    };
    // Room for the most buttons any row has, so the pills line up.
    let pill_right = pos2(inner.right() - actions_width - 4.0, cy);
    let pill = theme::pill(&painter, pill_right, &pill_text, fg, bg, 150.0);

    // Name and details.
    let text_left = tile.right() + 12.0;
    let text_w = (pill.left() - 12.0 - text_left).max(20.0);
    let name = item
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name_galley = theme::truncated(&painter, &name, theme::body(), p.text, text_w);
    let (meta, meta_colour, detail) = match &item.status {
        Status::Failed(error) => {
            let (text, detail) = error_text(error, t);
            let colour = if matches!(error, ConvertError::MissingCodec(_)) {
                p.warn
            } else {
                p.error
            };
            (text, colour, detail)
        }
        _ => (meta_line(item.probe, item.size, item.out_size), p.muted, None),
    };
    let meta_galley = theme::truncated(&painter, &meta, theme::small(), meta_colour, text_w);
    let name_pos = pos2(text_left, cy - 3.0 - name_galley.size().y);
    let meta_pos = pos2(text_left, cy);
    let text_rect = Rect::from_min_size(
        name_pos,
        vec2(text_w, name_galley.size().y + meta_galley.size().y + 3.0),
    );
    painter.galley(name_pos, name_galley, p.text);
    painter.galley(meta_pos, meta_galley, meta_colour);
    let tip = match detail {
        Some(detail) => format!("{}\n{detail}", item.path.display()),
        None => match &item.status {
            Status::Done(out) => format!("{}\n→ {}", item.path.display(), out.display()),
            _ => item.path.display().to_string(),
        },
    };
    ui.interact(text_rect, id.with("text"), Sense::hover())
        .on_hover_text(tip);

    if item.status == Status::Converting && started {
        let bar = Rect::from_min_size(
            pos2(text_left, rect.bottom() - 7.0),
            vec2(pill.right() - text_left, 3.0),
        );
        theme::progress_track(&painter, bar, fraction, time, p);
    }
    action
}

/// How many icon buttons a row in this state shows; must match `row`.
fn action_count(status: &Status, idle: bool) -> usize {
    let remove = usize::from(idle && *status != Status::Converting);
    remove
        + match status {
            Status::Done(_) => 2,
            Status::Failed(_) if idle => 1,
            _ => 0,
        }
}

/// The second line of a row: length, size, and the new size once done,
/// e.g. "3:42 · 48.1 MB → 5.1 MB".
fn meta_line(probe: Probe, size: Option<u64>, out_size: Option<u64>) -> String {
    let mut parts = Vec::new();
    if let Probe::Known(Info {
        seconds: Some(seconds),
        ..
    }) = probe
    {
        parts.push(format_duration(seconds));
    }
    match (size, out_size) {
        (Some(a), Some(b)) => parts.push(format!("{} → {}", format_size(a), format_size(b))),
        (Some(a), None) => parts.push(format_size(a)),
        (None, Some(b)) => parts.push(format!("→ {}", format_size(b))),
        (None, None) => {}
    }
    if parts.is_empty() {
        return if probe == Probe::Pending {
            "…".to_owned()
        } else {
            String::new()
        };
    }
    parts.join(" · ")
}

/// What went wrong, in one translated line, plus Windows' own words when
/// they add something.
fn error_text(error: &ConvertError, t: &Strings) -> (String, Option<String>) {
    match error {
        ConvertError::MissingCodec(_) => (t.needs_extension.to_owned(), None),
        ConvertError::NoAudio => (t.no_audio.to_owned(), None),
        ConvertError::Unreadable(message) => (t.cant_read.to_owned(), Some(message.clone())),
        ConvertError::Other(message) => (short_error(message, t.cant_read), Some(message.clone())),
        ConvertError::Cancelled => (String::new(), None),
    }
}

/// "3 converted · 212 MB → 18 MB · 1 failed".
fn summary_text(t: &Strings, s: &Summary) -> String {
    let mut parts = Vec::new();
    if s.converted > 0 {
        parts.push(fill(t.n_converted, &[("n", &s.converted.to_string())]));
        if s.in_bytes > 0 && s.out_bytes > 0 {
            parts.push(format!("{} → {}", format_size(s.in_bytes), format_size(s.out_bytes)));
        }
    }
    if s.failed > 0 {
        parts.push(fill(t.n_failed, &[("n", &s.failed.to_string())]));
    }
    parts.join(" · ")
}

/// Totals of a finished run from each file's (status, original size,
/// converted size). Files sent back to waiting by Cancel count as neither.
fn summarise<'a>(files: impl Iterator<Item = (&'a Status, Option<u64>, Option<u64>)>) -> Summary {
    let mut s = Summary {
        converted: 0,
        failed: 0,
        in_bytes: 0,
        out_bytes: 0,
    };
    for (status, size, out_size) in files {
        match status {
            Status::Done(_) => {
                s.converted += 1;
                s.in_bytes += size.unwrap_or(0);
                s.out_bytes += out_size.unwrap_or(0);
            }
            Status::Failed(_) => s.failed += 1,
            _ => {}
        }
    }
    s
}

/// The mean of each file's progress, 0 to 1.
fn overall(parts: &[f32]) -> f32 {
    if parts.is_empty() {
        return 0.0;
    }
    (parts.iter().map(|f| f.clamp(0.0, 1.0)).sum::<f32>() / parts.len() as f32).clamp(0.0, 1.0)
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

/// One background thread that reads each added file's length and size.
fn spawn_prober(ctx: egui::Context) -> (Sender<(u64, PathBuf)>, Receiver<ProbeResult>) {
    let (request_tx, request_rx) = channel::<(u64, PathBuf)>();
    let (result_tx, result_rx) = channel();
    std::thread::spawn(move || {
        while let Ok((id, path)) = request_rx.recv() {
            let size = std::fs::metadata(&path).ok().map(|m| m.len());
            let info = std::panic::catch_unwind(|| crate::mf::probe(&path))
                .ok()
                .flatten();
            if result_tx.send((id, info, size)).is_err() {
                break;
            }
            // Batches the redraws when many files are probed in a row.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    });
    (request_tx, result_rx)
}

fn spawn_workers(
    queue: Arc<Mutex<VecDeque<Job>>>,
    count: usize,
    options: Options,
    sender: Sender<JobResult>,
    ctx: egui::Context,
) {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 4)
        .min(count);
    for _ in 0..threads {
        let (queue, options, sender, ctx) =
            (queue.clone(), options.clone(), sender.clone(), ctx.clone());
        std::thread::spawn(move || {
            loop {
                let next = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                let Some(job) = next else { break };
                // A decoder panic must not leave the row stuck on "Converting".
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::convert::convert_file(&job.path, &options, &job.progress)
                }))
                .unwrap_or_else(|_| Err(ConvertError::Other(PANIC_MESSAGE.to_owned())))
                .map(|path| {
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    (path, bytes)
                });
                if sender.send((job.id, result)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }
}

/// Opens a file, folder, web page or Store page with whatever Windows uses
/// for it.
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

/// Opens Explorer on the file's folder with the file selected.
fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
}

impl eframe::App for App {
    fn ui(&mut self, ui_root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui_root.ctx().clone();
        self.drive_shots(&ctx);
        self.collect_probes();
        self.collect_results(&ctx);
        self.shortcuts(&ctx);
        if !self.busy() {
            let dropped: Vec<PathBuf> = ctx.input(|input| {
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
        if self.busy() {
            // Progress comes from the workers; a steady, modest redraw rate
            // shows it without drawing on every block of audio.
            ctx.request_repaint_after(BUSY_REPAINT);
        }

        let p = theme::palette(&ctx);
        let bare = |top: i8, bottom: i8| {
            egui::Frame::new().fill(p.bg).inner_margin(egui::Margin {
                left: GUTTER,
                right: GUTTER,
                top,
                bottom,
            })
        };
        egui::Panel::bottom("footer")
            .frame(bare(4, 8))
            .show_separator_line(false)
            .show(ui_root, |ui| self.footer(ui, p));
        egui::Panel::bottom("controls")
            .frame(bare(12, 4))
            .show_separator_line(false)
            .show(ui_root, |ui| {
                self.codec_help(ui, p);
                self.options_card(ui, p);
                ui.add_space(12.0);
                self.action_area(ui, p);
            });
        egui::Panel::top("toolbar")
            .frame(bare(16, 12))
            .show_separator_line(false)
            .show(ui_root, |ui| self.toolbar(ui, p));
        egui::CentralPanel::default()
            .frame(bare(0, 0))
            .show(ui_root, |ui| self.list(ui, p));
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

    #[test]
    fn meta_line_grows_as_more_is_known() {
        let info = Info {
            seconds: Some(222.0),
            rate: 44_100,
            channels: 2,
        };
        assert_eq!(meta_line(Probe::Pending, None, None), "…");
        assert_eq!(meta_line(Probe::Unknown, None, None), "");
        assert_eq!(meta_line(Probe::Unknown, Some(2048), None), "2.0 KB");
        assert_eq!(
            meta_line(Probe::Known(info), Some(50_436_505), None),
            "3:42 · 48.1 MB"
        );
        assert_eq!(
            meta_line(Probe::Known(info), Some(50_436_505), Some(5_347_737)),
            "3:42 · 48.1 MB → 5.1 MB"
        );
    }

    #[test]
    fn summary_counts_the_run() {
        let done = Status::Done(PathBuf::from("a.mp3"));
        let failed = Status::Failed(ConvertError::NoAudio);
        let s = summarise(
            [
                (&done, Some(100 * 1024 * 1024), Some(9 * 1024 * 1024)),
                (&done, Some(112 * 1024 * 1024), Some(9 * 1024 * 1024)),
                (&failed, Some(5), None),
                (&Status::Waiting, Some(7), None),
            ]
            .into_iter(),
        );
        assert_eq!(
            s,
            Summary {
                converted: 2,
                failed: 1,
                in_bytes: 212 * 1024 * 1024,
                out_bytes: 18 * 1024 * 1024
            }
        );
        let t = Lang::En.strings();
        assert_eq!(summary_text(t, &s), "2 converted · 212 MB → 18.0 MB · 1 failed");
        let none_failed = Summary { failed: 0, ..s };
        assert_eq!(summary_text(t, &none_failed), "2 converted · 212 MB → 18.0 MB");
        let only_failed = Summary { converted: 0, ..s };
        assert_eq!(summary_text(t, &only_failed), "1 failed");
    }

    #[test]
    fn rows_reserve_room_for_their_buttons() {
        let done = Status::Done(PathBuf::from("a.mp3"));
        let failed = Status::Failed(ConvertError::NoAudio);
        assert_eq!(action_count(&Status::Waiting, true), 1);
        assert_eq!(action_count(&Status::Waiting, false), 0);
        assert_eq!(action_count(&Status::Converting, false), 0);
        assert_eq!(action_count(&done, true), 3);
        assert_eq!(action_count(&done, false), 2);
        assert_eq!(action_count(&failed, true), 2);
        assert_eq!(action_count(&failed, false), 0);
    }

    #[test]
    fn overall_progress_weights_each_file() {
        assert_eq!(overall(&[]), 0.0);
        assert_eq!(overall(&[1.0, 0.5, 0.0, 0.5]), 0.5);
        assert_eq!(overall(&[2.0]), 1.0);
    }
}
