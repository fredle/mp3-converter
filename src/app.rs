//! The window: a queue of video and audio files, the output options, and the
//! worker threads that convert them. The auto-update toggle, the language
//! and the version live in the Settings flyout behind the gear in the
//! header. Colours, type and painted pieces come from `theme.rs`; this file
//! only lays out.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, FontId, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, pos2, vec2,
};

use crate::convert::{Codec, ConvertError, Format, Options, format_duration, format_size};
use crate::edits::{FileEdit, Problem};
use crate::i18n::{Lang, Strings, fill};
use crate::license;
use crate::mf::{AudioTrack, Edits, Info, Progress};
use crate::theme::{self, ButtonKind, Icon, Palette, Segment, StatusKind};

const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 680.0;
const MIN_SIZE: [f32; 2] = [640.0, 620.0];
const APP_NAME: &str = "MP4 to MP3 & Audio Converter";
/// The app's Microsoft Store ID (12 characters, such as `9NXXXXXXXXXX`),
/// for "Rate this app". The link stays hidden while this is empty; fill it
/// in once the Store listing has its ID (Partner Center > Product identity).
const STORE_ID: &str = "XP99D661MJWKBX";
/// Marks a decoder crash; shown to the user as the translated "couldn't read".
const PANIC_MESSAGE: &str = "decoder panicked";
/// Space between the window edge and its contents.
const GUTTER: i8 = 16;
const ROW_HEIGHT: f32 = 52.0;
/// What an expanded row adds under its name: two lines of controls.
const STRIP_HEIGHT: f32 = 2.0 * theme::CONTROL_HEIGHT + STRIP_GAP + 14.0;
const STRIP_GAP: f32 = 8.0;
/// Width of one row icon button with its gap.
const ACTION_SLOT: f32 = 32.0;
/// Width of a start or end time field.
const TIME_FIELD: f32 = 84.0;
/// How often the window redraws while converting.
const BUSY_REPAINT: Duration = Duration::from_millis(100);
const SETTINGS_WIDTH: f32 = 300.0;

pub fn run(args: Vec<String>) -> eframe::Result<()> {
    let size = match Shots::from_env() {
        Some(shots) => [shots.size[0] * shots.scale, shots.size[1] * shots.scale],
        None => [WIDTH, HEIGHT],
    };
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

/// A file's audio tracks, read on their own thread when its row is first
/// expanded.
#[derive(Clone, Debug, PartialEq)]
enum Tracks {
    NotAsked,
    Loading,
    Known(Vec<AudioTrack>),
    /// Windows could not list them; converting will say why.
    Unknown,
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
    /// Screenshot mode only: progress to show instead of `progress`.
    staged: Option<f32>,
    /// This file's own trim, track and volume.
    edit: FileEdit,
    tracks: Tracks,
}

impl Item {
    fn new(id: u64, path: PathBuf) -> Self {
        Self {
            id,
            path,
            status: Status::Waiting,
            probe: Probe::Pending,
            size: None,
            out_size: None,
            progress: Arc::default(),
            staged: None,
            edit: FileEdit::default(),
            tracks: Tracks::NotAsked,
        }
    }

    fn seconds(&self) -> Option<f64> {
        match self.probe {
            Probe::Known(info) => info.seconds,
            _ => None,
        }
    }

    /// Why this file's trim cannot be used, if it cannot.
    fn problem(&self) -> Option<Problem> {
        self.edit.check(self.seconds()).err()
    }

    /// Waiting, with choices a conversion can use.
    fn ready(&self) -> bool {
        self.status == Status::Waiting && self.problem().is_none()
    }

    /// Whether its choices can still change: not while it converts, and not
    /// once it is done.
    fn editable(&self) -> bool {
        matches!(self.status, Status::Waiting | Status::Failed(_))
    }

    /// Expected output size with the file's trim, when it can be known.
    fn estimate(&self, format: Format, kbps: u32, mono: bool) -> Option<u64> {
        let Probe::Known(info) = self.probe else { return None };
        let trim = self.edit.trim().ok()?;
        crate::convert::estimate_bytes(format, kbps, mono, &info.trimmed(&trim))
    }

    fn height(&self) -> f32 {
        if self.edit.expanded {
            ROW_HEIGHT + STRIP_HEIGHT
        } else {
            ROW_HEIGHT
        }
    }

    /// Whether converting has begun, and how far it is (when known).
    fn progress(&self) -> (bool, Option<f32>) {
        match self.staged {
            Some(f) => (true, Some(f)),
            None => (self.progress.started(), self.progress.fraction()),
        }
    }
}

struct Job {
    id: u64,
    path: PathBuf,
    edits: Edits,
    progress: Arc<Progress>,
}

type JobResult = (u64, Result<(PathBuf, u64), ConvertError>);
/// (item, file, audio track) to probe.
type ProbeRequest = (u64, PathBuf, usize);
type ProbeResult = (u64, Option<Info>, Option<u64>);
type TrackResult = (u64, Option<Vec<AudioTrack>>);

/// One press of Convert, until its last file finishes.
struct Run {
    /// Files no worker has started yet; emptied by Cancel.
    queue: Arc<Mutex<VecDeque<Job>>>,
    results: Receiver<JobResult>,
    ids: HashSet<u64>,
    in_flight: usize,
}

/// What the last run did, for the result bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Summary {
    converted: usize,
    failed: usize,
    in_bytes: u64,
    out_bytes: u64,
}

/// One-click starting points that set format, quality, mono and "Make
/// louder" together. The detailed controls stay and show what was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preset {
    /// Speech: MP3, mono, the Standard step (96 kbps mono), made louder
    /// (voice recordings are often quiet).
    Voice,
    /// MP3, stereo, the High step (256 kbps).
    Music,
    /// M4A (AAC), stereo, the Smaller step (96 kbps): about a third of a
    /// standard MP3 and still fine for music.
    Smallest,
}

impl Preset {
    const ALL: [Preset; 3] = [Preset::Voice, Preset::Music, Preset::Smallest];

    /// The options it sets. The stored kbps is a stereo rate; mono uses the
    /// same step of the mono list (see `Format::effective_kbps`).
    fn settings(self) -> Choice {
        let (format, kbps, mono, louder) = match self {
            Preset::Voice => (Format::Mp3, 192, true, true),
            Preset::Music => (Format::Mp3, 256, false, false),
            Preset::Smallest => (Format::M4a, 96, false, false),
        };
        Choice { format, kbps, mono, louder }
    }

    fn label(self, t: &Strings) -> &'static str {
        match self {
            Preset::Voice => t.preset_voice,
            Preset::Music => t.preset_music,
            Preset::Smallest => t.preset_smallest,
        }
    }

    /// What it sets, for the tooltip: "MP3 · 96 kbps · Mono (voice) ·
    /// Make louder".
    fn describe(self, t: &Strings) -> String {
        let c = self.settings();
        let mut parts = vec![c.format.label().to_owned()];
        let rate = c.format.effective_kbps(c.kbps, c.mono);
        if rate > 0 {
            parts.push(format!("{rate} kbps"));
        }
        if c.mono {
            parts.push(t.mono.to_owned());
        }
        if c.louder {
            parts.push(t.louder.to_owned());
        }
        parts.join(" · ")
    }
}

/// The options a preset sets, and that the options card shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Choice {
    format: Format,
    kbps: u32,
    mono: bool,
    /// "Make louder" for every file without a choice of its own.
    louder: bool,
}

impl Choice {
    fn of(settings: &crate::settings::Settings) -> Self {
        Self {
            format: settings.format,
            kbps: settings.kbps,
            mono: settings.mono,
            louder: settings.louder,
        }
    }

    fn apply(self, settings: &mut crate::settings::Settings) {
        settings.format = self.format;
        settings.kbps = self.kbps;
        settings.mono = self.mono;
        settings.louder = self.louder;
    }
}

/// The preset the current options match, if any: every option it sets,
/// the quality as a step.
fn preset_of(choice: Choice) -> Option<Preset> {
    Preset::ALL.into_iter().find(|preset| {
        let c = preset.settings();
        c.format == choice.format
            && c.mono == choice.mono
            && c.louder == choice.louder
            && (c.format.bitrates().is_empty() || c.format.tier(c.kbps) == choice.format.tier(choice.kbps))
    })
}

/// Files forwarded from another launch join the queue now, or wait in
/// `pending` while a run is going (the queue takes no files then); none
/// is ever dropped.
fn take_forwarded(pending: &mut Vec<PathBuf>, incoming: Vec<PathBuf>, busy: bool) -> Vec<PathBuf> {
    pending.extend(incoming);
    if busy { Vec::new() } else { std::mem::take(pending) }
}

/// The rows that show in a scroll view from `top` to `bottom`, given each
/// row's height in order.
fn visible_rows(heights: &[f32], top: f32, bottom: f32) -> std::ops::Range<usize> {
    let mut y = 0.0;
    let mut first = heights.len();
    let mut end = heights.len();
    for (i, h) in heights.iter().enumerate() {
        if first == heights.len() && y + h > top {
            first = i;
        }
        if y >= bottom {
            end = i;
            break;
        }
        y += h;
    }
    first.min(end)..end
}

/// The Store's "write a review" page for `store_id`, when it looks like a
/// real Store ID.
fn review_url(store_id: &str) -> Option<String> {
    let valid = matches!(store_id.len(), 12 | 14) && store_id.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then(|| format!("ms-windows-store://review/?ProductId={store_id}"))
}

/// "Rate this app" only after a run where everything converted.
fn show_rate_link(summary: &Summary, store_id: &str) -> bool {
    summary.failed == 0 && summary.converted > 0 && review_url(store_id).is_some()
}

/// The main button's words for `waiting` files of which a run may take
/// `allowance`.
fn primary_label(t: &Strings, waiting: usize, allowance: usize) -> String {
    if allowance < waiting {
        fill(t.convert_next, &[("n", &allowance.to_string())])
    } else if waiting == 0 {
        t.convert.to_owned()
    } else if waiting == 1 {
        t.convert_one.to_owned()
    } else {
        fill(t.convert_n, &[("n", &waiting.to_string())])
    }
}

/// Store screenshots mode: with `MAC_SHOTS=<folder>` set, the app shows
/// made-up scenes, writes each as `<n>_<w>x<h>.rgba` into the folder and
/// closes. The first three (a queue ready to convert, the same queue
/// converted, the empty window) are the Store's; `MAC_SHOTS_ALL=1` adds
/// design-review scenes (drop highlight, the free limit, converting,
/// Settings, failures, keyboard focus, a row expanded with its own trim,
/// track and volume). `MAC_SHOTS_LANG=<locale>` picks the language,
/// `MAC_SHOTS_SCALE` the pixels per point (on a display at 100%) and
/// `MAC_SHOTS_MIN=1` the smallest window. Nothing is converted and no
/// setting is saved. app-factory's `tools/compose-screenshot.ps1` turns the
/// frames into the Store's images.
struct Shots {
    dir: PathBuf,
    /// 0-based scene being shown.
    scene: usize,
    scenes: usize,
    /// Frames drawn of this scene, so fonts and layout settle before capture.
    frames: u32,
    requested: bool,
    scale: f32,
    size: [f32; 2],
    /// The scene pretends files are being dragged over the window.
    dragging: bool,
}

const SHOT_SIZE: [f32; 2] = [680.0, 640.0];
const STORE_SCENES: usize = 3;
const ALL_SCENES: usize = 10;
/// (file name, seconds, bytes) of the made-up queue.
const SHOT_FILES: [(&str, f64, u64); 8] = [
    ("Guitar lesson.mov", 768.0, 432_013_312),
    ("Wedding speech.mp4", 365.0, 195_035_136),
    ("Live set.flac", 3500.0, 357_564_416),
    ("Podcast episode 12.mp4", 2710.0, 612_368_384),
    ("Voice memo.m4a", 94.0, 1_530_880),
    ("Lecture week 3.mkv", 3180.0, 881_852_416),
    ("Birthday.mov", 142.0, 96_468_992),
    ("Rehearsal.wav", 1260.0, 222_265_344),
];

impl Shots {
    fn from_env() -> Option<Self> {
        let dir = std::env::var_os("MAC_SHOTS").filter(|d| !d.is_empty())?;
        let flag = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
        let scale = std::env::var("MAC_SHOTS_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|s| (0.5..=4.0).contains(s))
            .unwrap_or(1.0);
        Some(Self {
            dir: PathBuf::from(dir),
            scene: 0,
            scenes: if flag("MAC_SHOTS_ALL") { ALL_SCENES } else { STORE_SCENES },
            frames: 0,
            requested: false,
            scale,
            size: if flag("MAC_SHOTS_MIN") { MIN_SIZE } else { SHOT_SIZE },
            dragging: false,
        })
    }
}

enum RowAction {
    Remove,
    Retry,
    Play,
    Reveal,
    /// The expander was pressed (the row has already changed).
    Toggle,
    /// Another audio track was chosen: read its length again.
    TrackChanged,
}

/// What every row draws with.
struct RowLook<'a> {
    p: Palette,
    t: &'a Strings,
    idle: bool,
    time: f64,
    /// Room for the most buttons any row has, so the badges line up.
    actions_width: f32,
    /// The options' "Make louder", for rows without a choice of their own.
    louder: bool,
    format: Format,
    kbps: u32,
    mono: bool,
}

pub struct App {
    settings: crate::settings::Settings,
    version: String,
    /// "What would make this better?" (src/feedback.rs).
    feedback: crate::feedback::Feedback,
    items: Vec<Item>,
    next_id: u64,
    probe_requests: Sender<ProbeRequest>,
    probe_results: Receiver<ProbeResult>,
    track_requests: Sender<(u64, PathBuf)>,
    track_results: Receiver<TrackResult>,
    /// Files forwarded by later launches while a run was going.
    forwarded: Vec<PathBuf>,
    /// A row just expanded, to scroll into view once it has its height.
    reveal_row: Option<u64>,
    run: Option<Run>,
    summary: Option<Summary>,
    /// Folder of the most recent successful conversion, for "Open folder".
    last_output_dir: Option<PathBuf>,
    licensed_to: Option<String>,
    unlock_open: bool,
    key_entry: String,
    key_message: Option<String>,
    /// The Settings flyout, and where its gear button is.
    settings_open: bool,
    gear_rect: Rect,
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
        // Files forwarded by later launches wake this window.
        crate::shell::attach(ctx);
        theme::install_fonts(ctx, &[lang, Lang::system()]);
        // For checking both looks without changing Windows' setting.
        match std::env::var("HWC_THEME").as_deref() {
            Ok("dark") => ctx.set_theme(egui::Theme::Dark),
            Ok("light") => ctx.set_theme(egui::Theme::Light),
            _ => {}
        }
        let (probe_requests, probe_results) = spawn_prober(ctx.clone());
        let (track_requests, track_results) = spawn_track_reader(ctx.clone());
        let mut app = Self {
            lang,
            t: lang.strings(),
            settings,
            version: crate::update::current_version(),
            feedback: {
                let mut f = crate::feedback::Feedback::default();
                f.set_texts(lang.feedback_texts());
                f
            },
            items: Vec::new(),
            next_id: 0,
            probe_requests,
            probe_results,
            track_requests,
            track_results,
            forwarded: Vec::new(),
            reveal_row: None,
            run: None,
            summary: None,
            last_output_dir: None,
            licensed_to,
            unlock_open: false,
            key_entry: String::new(),
            key_message: None,
            settings_open: false,
            gear_rect: Rect::NOTHING,
            shots,
        };
        if let Some(shots) = &app.shots {
            ctx.set_pixels_per_point(shots.scale);
            // The version being released, not this build's "(dev)".
            app.version = std::env::var("MAC_SHOTS_VERSION")
                .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned());
            return app;
        }
        // Files passed on the command line ("Open with", Explorer's verb, or
        // dropped on the exe); flags such as --auto-update are not files.
        let paths: Vec<PathBuf> = crate::shell::file_args(&args)
            .into_iter()
            .map(PathBuf::from)
            .collect();
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
        self.feedback.set_texts(lang.feedback_texts());
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
            let _ = self.probe_requests.send((id, path.clone(), 0));
            self.items.push(Item::new(id, path));
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

    fn pick_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
            self.add_paths(&[folder]);
        }
    }

    fn look<'a>(&self, p: Palette, t: &'a Strings, time: f64, actions_width: f32) -> RowLook<'a> {
        RowLook {
            p,
            t,
            idle: !self.busy(),
            time,
            actions_width,
            louder: self.settings.louder,
            format: self.settings.format,
            kbps: self.settings.kbps,
            mono: self.settings.mono,
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

    /// Files the next run can take: waiting, with choices that fit the file.
    /// A file whose trim does not fit waits, with its row saying why.
    fn waiting(&self) -> usize {
        self.items.iter().filter(|i| i.ready()).count()
    }

    fn allowance(&self) -> usize {
        license::run_allowance(self.waiting(), self.licensed_to.is_some())
    }

    /// Converts the waiting files a run may take (all of them, or the first
    /// `FREE_BATCH` without a licence); the rest stay waiting for the next
    /// press.
    fn start(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let allowance = self.allowance();
        let louder = self.settings.louder;
        let mut jobs = VecDeque::new();
        for item in &mut self.items {
            if jobs.len() == allowance {
                break;
            }
            if item.status != Status::Waiting {
                continue;
            }
            let Ok(edits) = item.edit.edits(item.seconds(), louder) else {
                continue;
            };
            item.status = Status::Converting;
            item.progress = Arc::default();
            jobs.push_back(Job {
                id: item.id,
                path: item.path.clone(),
                edits,
                progress: item.progress.clone(),
            });
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

    /// Sets up scene `scene` of the screenshots: 0 ready, 1 converted,
    /// 2 empty, then (design review) 3 files dragged over the empty window,
    /// 4 more files than a free run takes, 5 converting, 6 Settings open,
    /// 7 a run with failures, 8 keyboard focus on a preset, 9 a row expanded
    /// (a trim, two audio tracks, Make louder) above one with its own trim.
    fn show_scene(&mut self, scene: usize) {
        self.items.clear();
        self.summary = None;
        self.last_output_dir = None;
        self.run = None;
        self.settings_open = scene == 6;
        // The focus scene also shows a preset chosen.
        let choice = match scene {
            8 => Preset::Voice.settings(),
            _ => Choice::of(&crate::settings::Settings::default()),
        };
        choice.apply(&mut self.settings);
        if let Some(shots) = self.shots.as_mut() {
            shots.dragging = scene == 3;
        }
        if matches!(scene, 2 | 3) {
            return;
        }
        let folder = Path::new("C:\\Media");
        let options = self.options();
        let count = if scene == 4 { 8 } else { 3 };
        for (id, (name, seconds, bytes)) in SHOT_FILES.into_iter().take(count).enumerate() {
            let path = folder.join(name);
            let info = Info {
                seconds: Some(seconds),
                rate: 48_000,
                channels: 2,
            };
            let converted = || {
                let out = crate::convert::output_path(&path, &options, |_| false);
                let size = crate::convert::estimate_bytes(options.format, options.kbps, false, &info);
                (Status::Done(out), size)
            };
            let (status, out_size, staged) = match (scene, id) {
                (1, _) | (7, 0) => {
                    let (status, size) = converted();
                    (status, size, None)
                }
                (5, 0) => (Status::Converting, None, Some(0.64)),
                (5, 1) => (Status::Converting, None, Some(0.31)),
                (5, _) => (Status::Converting, None, None),
                (7, 1) => (Status::Failed(ConvertError::NoAudio), None, None),
                (7, _) => (Status::Failed(ConvertError::MissingCodec(Codec::WebMedia)), None, None),
                _ => (Status::Waiting, None, None),
            };
            let mut item = Item::new(id as u64, path);
            item.status = status;
            item.probe = Probe::Known(info);
            item.size = Some(bytes);
            item.out_size = out_size;
            item.staged = staged;
            self.items.push(item);
        }
        if scene == 9 {
            let track = |index: usize, language: &str, name: Option<&str>| AudioTrack {
                index,
                language: Some(language.to_owned()),
                name: name.map(str::to_owned),
                codec: Some("AAC"),
                channels: 2,
                rate: 48_000,
                decodable: true,
            };
            let first = &mut self.items[0];
            first.edit = FileEdit {
                expanded: true,
                start: "0:30".to_owned(),
                end: "2:15".to_owned(),
                ..FileEdit::default()
            };
            first.edit.set_louder(true, self.settings.louder);
            first.tracks = Tracks::Known(vec![track(0, "en", None), track(1, "en", Some("Commentary"))]);
            let second = &mut self.items[1];
            second.edit.start = "1:10".to_owned();
            second.edit.track = 1;
        }
        if matches!(scene, 1 | 7) {
            self.summary = Some(summarise(
                self.items.iter().map(|i| (&i.status, i.size, i.out_size)),
            ));
            self.last_output_dir = Some(folder.to_path_buf());
        }
        if scene == 5 {
            // A run with no workers: its results never come.
            let ids: HashSet<u64> = self.items.iter().map(|i| i.id).collect();
            self.run = Some(Run {
                queue: Arc::default(),
                results: channel().1,
                in_flight: ids.len(),
                ids,
            });
        }
    }

    /// Steps through the screenshots: show a scene, let it settle, capture
    /// it, write it, move on; close after the last.
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
        if scene >= shots.scenes {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if frames == 0 {
            self.show_scene(scene);
            // Each scene starts without the last one's keyboard focus.
            if let Some(focused) = ctx.memory(|m| m.focused()) {
                ctx.memory_mut(|m| m.surrender_focus(focused));
            }
        }
        if scene == 8 {
            // As if Tab had reached the "Music" preset.
            let id = egui::Id::new(("preset", Preset::Music as u8));
            ctx.memory_mut(|m| m.request_focus(id));
        }
        if frames >= 5 && !requested {
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

    fn collect_tracks(&mut self) {
        while let Ok((id, tracks)) = self.track_results.try_recv() {
            if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
                item.tracks = tracks.map_or(Tracks::Unknown, Tracks::Known);
            }
        }
    }

    /// Asks for the track list of every expanded row that has none yet.
    fn request_tracks(&mut self) {
        if self.shots.is_some() {
            return;
        }
        for item in &mut self.items {
            if item.edit.expanded && item.tracks == Tracks::NotAsked {
                item.tracks = Tracks::Loading;
                let _ = self.track_requests.send((item.id, item.path.clone()));
            }
        }
    }

    /// Files forwarded by later launches (Explorer's verb on many files):
    /// into the queue, or held until the run ends.
    fn collect_forwarded(&mut self) {
        if self.shots.is_some() {
            return;
        }
        let incoming = crate::shell::take_forwarded();
        let busy = self.busy();
        let now = take_forwarded(&mut self.forwarded, incoming, busy);
        if !now.is_empty() {
            self.add_paths(&now);
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
                Status::Converting => i.progress().1.unwrap_or(0.0),
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
        for item in self.items.iter().filter(|i| i.ready()).take(self.allowance()) {
            match item.probe {
                Probe::Pending => return None,
                Probe::Unknown => {}
                Probe::Known(_) => {
                    // Only the kept part of a trimmed file.
                    total += item.estimate(format, kbps, mono)?;
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
            if self.settings_open {
                // A language list open inside the flyout closes first.
                if !ctx.any_popup_open() {
                    self.settings_open = false;
                }
            } else if self.unlock_open {
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
            self.settings_open = false;
            self.pick_files();
        } else if enter
            && !self.settings_open
            && self.waiting() > 0
            && ctx.memory(|m| m.focused().is_none())
            && !ctx.any_popup_open()
        {
            self.start(ctx);
        }
    }

    /// The header: adding files on the left; the licence state and the
    /// Settings gear on the right.
    fn header(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let idle = !self.busy();
            ui.add_enabled_ui(idle, |ui| {
                if theme::button(ui, ButtonKind::Standard, t.add_files, 0.0, theme::CONTROL_HEIGHT)
                    .on_hover_text("Ctrl+O")
                    .clicked()
                {
                    self.settings_open = false;
                    self.pick_files();
                }
                if theme::button(ui, ButtonKind::Standard, t.add_folder, 0.0, theme::CONTROL_HEIGHT).clicked() {
                    self.settings_open = false;
                    self.pick_folder();
                }
                if !self.items.is_empty()
                    && theme::button(ui, ButtonKind::Subtle, t.clear, 0.0, theme::CONTROL_HEIGHT).clicked()
                {
                    self.items.clear();
                    self.summary = None;
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (gear, _) = ui.allocate_exact_size(vec2(theme::CONTROL_HEIGHT, theme::CONTROL_HEIGHT), Sense::hover());
                self.gear_rect = gear;
                if self.settings_open {
                    ui.painter().rect_filled(gear, theme::CONTROL_RADIUS, p.control_pressed);
                }
                if theme::icon_button(ui, gear, egui::Id::new("settings-gear"), Icon::Gear, p)
                    .on_hover_text(t.settings)
                    .clicked()
                {
                    self.settings_open = !self.settings_open;
                }
                if let Some(email) = &self.licensed_to {
                    theme::tag(ui, Icon::Check, &fill(t.unlocked_for, &[("email", email)]), p.success, p.success_soft);
                } else if license::paid_tier_enabled() {
                    let limit = fill(t.free_limit, &[("n", &license::FREE_BATCH.to_string())]);
                    if theme::button(ui, ButtonKind::Link, t.unlock, 0.0, theme::CONTROL_HEIGHT)
                        .on_hover_text(&limit)
                        .clicked()
                    {
                        self.settings_open = false;
                        self.unlock_open = true;
                    }
                }
            });
        });
    }

    /// The Settings flyout under the gear: language, automatic updates, the
    /// feedback link and the version.
    fn settings_flyout(&mut self, ctx: &egui::Context, p: Palette) {
        if !self.settings_open {
            return;
        }
        let t = self.t;
        let anchor = self.gear_rect;
        let area = egui::Area::new(egui::Id::new("settings-flyout"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(anchor.right(), anchor.bottom() + 6.0))
            .pivot(Align2::RIGHT_TOP)
            .constrain(true)
            .show(ctx, |ui| {
                theme::card(p)
                    .inner_margin(egui::Margin::same(16))
                    .shadow(ctx.global_style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        let width = SETTINGS_WIDTH.min(ctx.content_rect().width() - 48.0);
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.label(RichText::new(t.settings).font(FontId::new(16.0, theme::semibold())).color(p.text));
                        ui.add_space(6.0);

                        ui.label(RichText::new(t.language).font(theme::small()).color(p.muted));
                        let mut chosen = self.lang;
                        let combo = egui::ComboBox::from_id_salt("language")
                            .selected_text(RichText::new(self.lang.name()).color(p.text))
                            .width(width)
                            .height(320.0)
                            .icon(|ui, rect, _, _| {
                                let p = theme::palette(ui.ctx());
                                theme::paint_icon(ui.painter(), rect.center(), Icon::ChevronDown, p.muted);
                            })
                            .show_ui(ui, |ui| {
                                for lang in Lang::ALL {
                                    ui.selectable_value(&mut chosen, lang, lang.name());
                                }
                            });
                        theme::focus_ring(ui, &combo.response, theme::CONTROL_RADIUS);
                        if chosen != self.lang {
                            self.set_language(chosen, ctx);
                        }
                        ui.add_space(8.0);

                        let mut auto_update = self.settings.auto_update;
                        if theme::checkbox(ui, &mut auto_update, t.auto_update, theme::body()).changed() {
                            self.settings.auto_update = auto_update;
                            self.save();
                        }
                        ui.add(
                            egui::Label::new(RichText::new(t.auto_update_hint).font(theme::small()).color(p.muted))
                                .wrap(),
                        );
                        ui.add_space(6.0);
                        let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
                        ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, p.border));
                        ui.add_space(4.0);
                        // The feedback link (factory-owned); the flyout
                        // closes when it opens the feedback box.
                        let link = ui.scope(|ui| self.feedback.link(ui)).response.rect;
                        let clicked = ui.input(|i| {
                            i.pointer.primary_clicked()
                                && i.pointer.interact_pos().is_some_and(|pos| link.contains(pos))
                        });
                        if clicked {
                            self.settings_open = false;
                        }
                        ui.label(
                            RichText::new(fill(t.version, &[("v", &self.version)]))
                                .font(theme::small())
                                .color(p.muted),
                        );
                    });
            });
        // A click anywhere else closes it, as Windows' flyouts do; the
        // language list is its own popup, so clicks in it do not count.
        let clicked_outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer.interact_pos().is_some_and(|pos| {
                    !area.response.rect.contains(pos) && !anchor.contains(pos)
                })
        });
        if clicked_outside && !ctx.any_popup_open() {
            self.settings_open = false;
        }
    }

    /// The empty state: one big target that says what the app wants.
    fn drop_zone(&mut self, ui: &mut egui::Ui, p: Palette, hovering: bool) {
        let t = self.t;
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, theme::CARD_RADIUS, p.surface);
        if hovering {
            painter.rect_stroke(
                rect,
                theme::CARD_RADIUS,
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
        } else {
            theme::dashed_rect(
                &painter,
                rect.shrink(0.5),
                theme::CARD_RADIUS as f32,
                Stroke::new(1.0, p.strong_border),
            );
        }

        let max_text = (rect.width() - 48.0).max(120.0);
        let headline = theme::truncated(
            &painter,
            t.drop_here,
            FontId::new(theme::SUBTITLE_SIZE, theme::semibold()),
            p.text,
            max_text,
        );
        let formats = fill(
            t.formats_and_more,
            &[("list", &crate::convert::featured_formats().join(", "))],
        );
        let formats_galley = painter.layout(
            formats,
            theme::small(),
            p.muted,
            max_text.min(420.0),
        );
        let disc = 56.0;
        let link_h = 28.0;
        let total = disc + 16.0 + headline.size().y + 4.0 + link_h + 12.0 + formats_galley.size().y;
        let mut y = rect.center().y - total / 2.0;
        let cx = rect.center().x;

        let disc_rect = Rect::from_center_size(pos2(cx, y + disc / 2.0), vec2(disc, disc));
        painter.circle_filled(disc_rect.center(), disc / 2.0, if hovering { p.accent_soft } else { p.bg });
        theme::paint_icon_sized(
            &painter,
            disc_rect.center(),
            Icon::Arrow,
            if hovering { p.accent_text } else { p.muted },
            1.5,
        );
        y += disc + 16.0;

        painter.galley(pos2(cx - headline.size().x / 2.0, y), headline.clone(), p.text);
        y += headline.size().y + 4.0;

        let link_w = theme::text_width(ui, t.or_choose, theme::body()) + 4.0;
        let link_rect = Rect::from_min_size(pos2(cx - link_w / 2.0, y), vec2(link_w, link_h));
        let clicked = ui
            .scope_builder(egui::UiBuilder::new().max_rect(link_rect), |ui| {
                theme::button(ui, ButtonKind::Link, t.or_choose, link_w, link_h)
                    .on_hover_text("Ctrl+O")
                    .clicked()
            })
            .inner;
        if clicked {
            self.pick_files();
        }
        y += link_h + 12.0;
        painter.galley(pos2(cx - formats_galley.size().x / 2.0, y), formats_galley, p.muted);
    }

    fn list(&mut self, ui: &mut egui::Ui, p: Palette) {
        let staged_drag = self.shots.as_ref().is_some_and(|s| s.dragging);
        let hovering = (staged_drag || ui.ctx().input(|i| !i.raw.hovered_files.is_empty())) && !self.busy();
        if self.items.is_empty() {
            self.drop_zone(ui, p, hovering);
            return;
        }
        let idle = !self.busy();
        let time = ui.ctx().input(|i| i.time);
        let mut actions: Vec<(u64, RowAction)> = Vec::new();
        let slots = self
            .items
            .iter()
            .map(|i| action_count(&i.status, idle))
            .max()
            .unwrap_or(0);
        let look = self.look(p, self.t, time, slots as f32 * ACTION_SLOT);
        let scroll_to = self.reveal_row.take();
        let mut frame = theme::card(p).inner_margin(egui::Margin::symmetric(0, 4));
        if hovering {
            frame = frame.stroke(Stroke::new(2.0, p.accent));
        }
        frame.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Rows differ in height (an expanded one shows its controls), so
            // only the rows in view are laid out, each at its own offset.
            let heights: Vec<f32> = self.items.iter().map(Item::height).collect();
            let offsets: Vec<f32> = heights
                .iter()
                .scan(0.0, |y, h| {
                    let top = *y;
                    *y += h;
                    Some(top)
                })
                .collect();
            let count = self.items.len();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_viewport(ui, |ui, viewport| {
                    ui.set_height(heights.iter().sum::<f32>());
                    let top = ui.max_rect().top();
                    let x = ui.max_rect().x_range();
                    for index in visible_rows(&heights, viewport.min.y, viewport.max.y) {
                        let y = top + offsets[index];
                        let rect = Rect::from_x_y_ranges(x, y..=y + heights[index]);
                        let item = &mut self.items[index];
                        if scroll_to == Some(item.id) {
                            ui.scroll_to_rect(rect, None);
                        }
                        let last = index + 1 == count;
                        let builder = egui::UiBuilder::new().max_rect(rect).id_salt(("row", item.id));
                        if let Some(action) = ui.scope_builder(builder, |ui| row(ui, item, &look, last)).inner {
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
                RowAction::Toggle => {
                    if self.items[position].edit.expanded {
                        self.reveal_row = Some(id);
                    }
                }
                RowAction::TrackChanged => {
                    // Tracks of one file can differ in length, rate and
                    // channels: read the chosen one, for its size estimate.
                    let item = &mut self.items[position];
                    item.probe = Probe::Pending;
                    let _ = self.probe_requests.send((id, item.path.clone(), item.edit.track));
                }
            }
        }
        self.request_tracks();
    }

    fn codec_help(&mut self, ui: &mut egui::Ui, p: Palette) {
        let codecs = self.missing_codecs();
        if codecs.is_empty() {
            return;
        }
        let t = self.t;
        // A Fluent InfoBar: a soft caution fill, an icon and the fix.
        theme::card(p)
            .fill(p.warn_soft)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for codec in codecs {
                    ui.horizontal(|ui| {
                        let (icon, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                        theme::paint_icon(ui.painter(), icon.center(), Icon::Warning, p.warn);
                        ui.add(
                            egui::Label::new(RichText::new(match codec {
                                Codec::WebMedia => t.need_webmedia,
                            }).color(p.text))
                            .wrap(),
                        );
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (name, product, free) in codec.store_products() {
                            let label =
                                fill(if *free { t.get_free } else { t.get }, &[("name", name)]);
                            if theme::button(ui, ButtonKind::Standard, &label, 0.0, theme::CONTROL_HEIGHT).clicked() {
                                open(&format!("ms-windows-store://pdp/?ProductId={product}"));
                            }
                        }
                        ui.label(RichText::new(t.install_then).color(p.muted));
                        if ui
                            .add_enabled_ui(!self.busy(), |ui| {
                                theme::button(ui, ButtonKind::Standard, t.try_again, 0.0, theme::CONTROL_HEIGHT)
                            })
                            .inner
                            .clicked()
                        {
                            self.retry_failed();
                        }
                    });
                }
            });
        ui.add_space(8.0);
    }

    fn options_card(&mut self, ui: &mut egui::Ui, p: Palette) {
        let before = Choice::of(&self.settings);
        let t = self.t;
        let idle = !self.busy();
        let estimate = self.estimate();
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            let label_w = label_width(ui, &[t.use_for, t.convert_to, t.quality, t.save_to]);
            ui.add_enabled_ui(idle, |ui| {
                option_row(ui, label_w, t.use_for, 30.0, p, |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let current = preset_of(Choice::of(&self.settings));
                    for preset in Preset::ALL {
                        let id = egui::Id::new(("preset", preset as u8));
                        if theme::chip(ui, id, preset.label(t), current == Some(preset))
                            .on_hover_text(preset.describe(t))
                            .clicked()
                        {
                            preset.settings().apply(&mut self.settings);
                        }
                    }
                });
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
                let height = if lossy { theme::SEGMENTED_TALL } else { theme::CONTROL_HEIGHT };
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
                option_row(ui, label_w, "", 28.0, p, |ui| {
                    let mut mono = self.settings.mono;
                    if theme::checkbox(ui, &mut mono, t.mono, theme::body())
                        .on_hover_text(t.mono_hint)
                        .changed()
                    {
                        self.settings.mono = mono;
                    }
                    ui.add_space(8.0);
                    // For every file without a choice of its own.
                    let mut louder = self.settings.louder;
                    if theme::checkbox(ui, &mut louder, t.louder, theme::body())
                        .on_hover_text(t.louder_hint)
                        .changed()
                    {
                        self.settings.louder = louder;
                    }
                    if let (Some(bytes), true) = (estimate, idle) {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let text = fill(t.estimate, &[("size", &format_size(bytes))]);
                            ui.add(egui::Label::new(RichText::new(text).color(p.muted)).truncate());
                        });
                    }
                });
                option_row(ui, label_w, t.save_to, theme::CONTROL_HEIGHT, p, |ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::button(ui, ButtonKind::Standard, t.change_folder, 0.0, theme::CONTROL_HEIGHT).clicked()
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
        if before != Choice::of(&self.settings) {
            self.save();
        }
    }

    /// After a run: what happened, Open folder, Clear finished, and a quiet
    /// "Rate this app" when everything converted.
    fn result_bar(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        let Some(summary) = self.summary else { return };
        if self.busy() {
            return;
        }
        let done = self.items.iter().any(|i| matches!(i.status, Status::Done(_)));
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if done && theme::button(ui, ButtonKind::Standard, t.clear_finished, 0.0, theme::CONTROL_HEIGHT).clicked() {
                    self.items.retain(|i| !matches!(i.status, Status::Done(_)));
                }
                if let Some(dir) = &self.last_output_dir
                    && theme::button(ui, ButtonKind::Standard, t.open_folder, 0.0, theme::CONTROL_HEIGHT).clicked()
                {
                    open(&dir.display().to_string());
                }
                if show_rate_link(&summary, STORE_ID)
                    && let Some(url) = review_url(STORE_ID)
                    && theme::button(ui, ButtonKind::Link, t.rate, 0.0, theme::CONTROL_HEIGHT).clicked()
                {
                    open(&url);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (icon, colour) = if summary.failed == 0 {
                        (Icon::Check, p.success)
                    } else {
                        (Icon::Alert, p.error)
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                    theme::paint_icon(ui.painter(), r.center(), icon, colour);
                    ui.add(
                        egui::Label::new(RichText::new(summary_text(t, &summary)).color(p.text))
                            .truncate(),
                    );
                });
            });
        });
        ui.add_space(6.0);
    }

    /// A calm line when more files wait than a free run converts: the next
    /// ones simply wait for the next click.
    fn limit_note(&mut self, ui: &mut egui::Ui, p: Palette) {
        if !limit_note_visible(self.waiting(), self.allowance(), self.busy()) {
            return;
        }
        let t = self.t;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
            theme::paint_icon(ui.painter(), r.center(), Icon::Info, p.muted);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(fill(t.limit_note, &[("n", &license::FREE_BATCH.to_string())]))
                            .font(theme::small())
                            .color(p.muted),
                    )
                    .wrap(),
                );
                if theme::link(ui, t.unlock_link, theme::small()).clicked() {
                    self.unlock_open = true;
                }
            });
        });
        ui.add_space(6.0);
    }

    /// The main button, or while converting the run's progress and Cancel.
    fn action_area(&mut self, ui: &mut egui::Ui, p: Palette) {
        let t = self.t;
        let width = ui.available_width();
        if let Some(run) = &self.run {
            let fraction = self.run_progress();
            let in_flight = run.in_flight;
            ui.horizontal(|ui| {
                let cancel_w = (theme::text_width(ui, t.cancel, theme::body()) + 32.0).max(96.0);
                let (rect, _) = ui.allocate_exact_size(
                    vec2((width - cancel_w - 16.0).max(80.0), theme::PRIMARY_HEIGHT),
                    Sense::hover(),
                );
                let painter = ui.painter();
                let status = fill(t.converting_left, &[("n", &in_flight.to_string())]);
                let percent = format!("{}%", (fraction * 100.0).floor() as u32);
                let percent_galley = painter.layout_no_wrap(percent, theme::strong(), p.text);
                let status_galley = theme::truncated(
                    painter,
                    &status,
                    theme::body(),
                    p.text,
                    rect.width() - percent_galley.size().x - 12.0,
                );
                let text_y = rect.top() + 4.0;
                painter.galley(pos2(rect.left(), text_y), status_galley, p.text);
                painter.galley(
                    pos2(rect.right() - percent_galley.size().x, text_y),
                    percent_galley,
                    p.text,
                );
                let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 10.0), pos2(rect.right(), rect.bottom() - 6.0));
                theme::progress_bar(painter, bar, Some(fraction), 0.0, p);
                ui.add_space(8.0);
                if theme::button(ui, ButtonKind::Standard, t.cancel, cancel_w, theme::PRIMARY_HEIGHT)
                    .on_hover_text("Esc")
                    .clicked()
                {
                    self.cancel();
                }
            });
            return;
        }
        let waiting = self.waiting();
        let label = primary_label(t, waiting, self.allowance());
        let clicked = ui
            .add_enabled_ui(waiting > 0, |ui| {
                theme::button(ui, ButtonKind::Primary, &label, width, theme::PRIMARY_HEIGHT)
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
        egui::Window::new(RichText::new(t.unlock_title).font(FontId::new(16.0, theme::semibold())))
            .id(egui::Id::new("unlock"))
            .open(&mut open_flag)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0_f32.min(ctx.content_rect().width() - 80.0));
                ui.label(RichText::new(fill(
                    t.unlock_body,
                    &[("n", &license::FREE_BATCH.to_string())],
                )).color(p.text));
                ui.add_space(8.0);
                if theme::button(ui, ButtonKind::Primary, t.buy, 0.0, theme::PRIMARY_HEIGHT).clicked() {
                    open(license::BUY_URL);
                }
                ui.add_space(12.0);
                ui.label(RichText::new(t.paste_key).color(p.muted));
                ui.add(
                    egui::TextEdit::singleline(&mut self.key_entry)
                        .hint_text("MAC1.…")
                        .margin(vec2(8.0, 6.0))
                        .desired_width(f32::INFINITY),
                );
                if theme::button(ui, ButtonKind::Standard, t.activate, 0.0, theme::CONTROL_HEIGHT).clicked() {
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
}

/// Whether to show the free-limit note: more files wait than the next run
/// takes.
fn limit_note_visible(waiting: usize, allowance: usize, busy: bool) -> bool {
    !busy && allowance < waiting
}

/// Width for the option labels: the longest, within limits, so the
/// controls line up whatever the language.
fn label_width(ui: &egui::Ui, labels: &[&str]) -> f32 {
    labels
        .iter()
        .map(|l| theme::text_width(ui, l, theme::body()))
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

/// One file in the list: its badge, name, details, status and actions, and
/// when expanded its own trim, audio track and volume underneath.
fn row(ui: &mut egui::Ui, item: &mut Item, look: &RowLook, last: bool) -> Option<RowAction> {
    let (p, t, idle) = (look.p, look.t, look.idle);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, item.height()), Sense::hover());
    let head = Rect::from_min_size(rect.min, vec2(width, ROW_HEIGHT));
    let id = egui::Id::new(("row", item.id));
    let painter = ui.painter().clone();
    if ui.rect_contains_pointer(head) {
        painter.rect_filled(head.shrink2(vec2(4.0, 2.0)), theme::CONTROL_RADIUS, p.hover);
    }
    if !last {
        painter.hline(
            (rect.left() + 16.0)..=(rect.right() - 16.0),
            rect.bottom() - 0.5,
            Stroke::new(1.0, p.border),
        );
    }
    let inner = head.shrink2(vec2(16.0, 0.0));
    let cy = head.center().y;

    let extension = item
        .path
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    let badge = Rect::from_min_size(pos2(inner.left(), cy - 18.0), vec2(36.0, 36.0));
    theme::file_badge(&painter, badge, crate::convert::is_video(&item.path), &extension, p);

    // Icon buttons from the right edge in; the expander is the left-most.
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
    let chevron = if item.edit.expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    button(ui, chevron, t.file_options, RowAction::Toggle);
    if matches!(action, Some(RowAction::Toggle)) {
        item.edit.expanded = !item.edit.expanded;
    }

    // The status badge, lined up across rows.
    let (started, fraction) = item.progress();
    let (badge_text, kind) = match &item.status {
        Status::Waiting => (t.waiting.to_owned(), StatusKind::Waiting),
        Status::Converting if !started => (t.waiting.to_owned(), StatusKind::Waiting),
        Status::Converting => match fraction {
            Some(f) => (format!("{}%", (f * 100.0).floor() as u32), StatusKind::Busy),
            None => (t.converting.to_owned(), StatusKind::Busy),
        },
        Status::Done(_) => (t.done.to_owned(), StatusKind::Done),
        Status::Failed(ConvertError::MissingCodec(_)) => (t.failed.to_owned(), StatusKind::Warning),
        Status::Failed(_) => (t.failed.to_owned(), StatusKind::Failed),
    };
    // Room for the most buttons any row has, so the badges line up.
    let badge_right = pos2(inner.right() - look.actions_width - 4.0, cy);
    let status = theme::status_badge(&painter, badge_right, &badge_text, kind, p, 150.0);

    // Name and details.
    let text_left = badge.right() + 12.0;
    let text_w = (status.left() - 12.0 - text_left).max(20.0);
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
    // A collapsed row notes its own choices, or why they cannot be used.
    let note = match (&item.status, item.edit.expanded, item.problem()) {
        (Status::Failed(_), _, _) | (_, true, _) => None,
        (Status::Waiting, false, Some(problem)) => {
            Some((crate::edits::problem_text(problem, t).to_owned(), p.error))
        }
        _ => crate::edits::indicator(&item.edit, look.louder, t).map(|text| (text, p.accent_text)),
    };
    let mut parts: Vec<(&str, egui::Color32)> = vec![(&meta, meta_colour)];
    if let Some((text, colour)) = &note {
        if !meta.is_empty() {
            parts.push((" · ", p.muted));
        }
        parts.push((text, *colour));
    }
    let meta_galley = theme::truncated_parts(&painter, &parts, theme::small(), text_w);
    let converting = item.status == Status::Converting && started;
    let gap = if converting { 1.0 } else { 2.0 };
    let block_h = name_galley.size().y + gap + meta_galley.size().y + if converting { 8.0 } else { 0.0 };
    let name_pos = pos2(text_left, cy - block_h / 2.0);
    let meta_pos = pos2(text_left, name_pos.y + name_galley.size().y + gap);
    let text_rect = Rect::from_min_size(
        name_pos,
        vec2(text_w, name_galley.size().y + gap + meta_galley.size().y),
    );
    let meta_bottom = meta_pos.y + meta_galley.size().y;
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

    if converting {
        let bar = Rect::from_min_size(pos2(text_left, meta_bottom + 3.0), vec2(text_w, 4.0));
        theme::progress_bar(&painter, bar, fraction, look.time, p);
    }

    if item.edit.expanded {
        // Under the name, from its left edge to the buttons' right edge.
        let line = Rect::from_min_max(
            pos2(text_left, head.bottom()),
            pos2(inner.right(), head.bottom() + theme::CONTROL_HEIGHT),
        );
        if strip(ui, item, look, line, id) && action.is_none() {
            action = Some(RowAction::TrackChanged);
        }
    }
    action
}

/// The expanded row's controls, on two lines from `line` down: the trim
/// times (and what is wrong with them), then the audio track, "Make louder"
/// and the file's expected size. Returns whether the track changed.
fn strip(ui: &mut egui::Ui, item: &mut Item, look: &RowLook, line: Rect, id: egui::Id) -> bool {
    let (p, t) = (look.p, look.t);
    let enabled = item.editable();
    let builder = |rect: Rect| {
        let b = egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center));
        if enabled { b } else { b.disabled() }
    };
    let label = |ui: &mut egui::Ui, text: &str| {
        let colour = if enabled { p.muted } else { p.disabled };
        ui.add(egui::Label::new(RichText::new(text).color(colour)).selectable(false));
    };

    // Line one: Start and End, then the problem with them, if any.
    ui.scope_builder(builder(line), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let before = (item.edit.start.clone(), item.edit.end.clone());
        let problem = item.problem();
        let bad_start = crate::edits::parse_time(&item.edit.start).is_err()
            || problem == Some(Problem::Trim(crate::mf::TrimError::StartPastEnd));
        let bad_end = crate::edits::parse_time(&item.edit.end).is_err()
            || problem == Some(Problem::Trim(crate::mf::TrimError::EndBeforeStart));
        label(ui, t.trim_start);
        theme::text_field(ui, id.with("start"), &mut item.edit.start, "0:00", TIME_FIELD, bad_start);
        ui.add_space(8.0);
        label(ui, t.trim_end);
        // The file's length: the end used when none is given.
        let end_hint = item.seconds().map(crate::edits::format_time).unwrap_or_default();
        theme::text_field(ui, id.with("end"), &mut item.edit.end, &end_hint, TIME_FIELD, bad_end);
        if (&item.edit.start, &item.edit.end) != (&before.0, &before.1) {
            // The outlines above were drawn for the old text.
            ui.ctx().request_repaint();
        }
        if let Some(problem) = item.problem() {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
            theme::paint_icon(ui.painter(), r.center(), Icon::Alert, p.error);
            ui.add(
                egui::Label::new(
                    RichText::new(crate::edits::problem_text(problem, t))
                        .font(theme::small())
                        .color(p.error),
                )
                .truncate(),
            );
        }
    });

    // Line two: the audio track (only when there is a choice), Make louder,
    // and the size.
    let line = line.translate(vec2(0.0, theme::CONTROL_HEIGHT + STRIP_GAP));
    let mut track_changed = false;
    ui.scope_builder(builder(line), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if let Tracks::Known(tracks) = &item.tracks
            && tracks.len() > 1
        {
            let font = theme::body();
            let labels: Vec<(String, bool)> = tracks
                .iter()
                .map(|track| {
                    let drawable = |s: &str| ui.fonts_mut(|f| f.has_glyphs(&font, s));
                    (crate::edits::track_label(track, t, drawable), track.decodable)
                })
                .collect();
            let current = item.edit.track.min(labels.len() - 1);
            let mut chosen = current;
            let width = (ui.available_width() * 0.5).clamp(150.0, 300.0);
            let fg = if enabled { p.text } else { p.disabled };
            let combo = egui::ComboBox::from_id_salt(id.with("track"))
                .selected_text(RichText::new(&labels[current].0).color(fg))
                .width(width)
                .icon(|ui, rect, _, _| {
                    let p = theme::palette(ui.ctx());
                    let colour = if ui.is_enabled() { p.muted } else { p.disabled };
                    theme::paint_icon(ui.painter(), rect.center(), Icon::ChevronDown, colour);
                })
                .show_ui(ui, |ui| {
                    for (i, (text, decodable)) in labels.iter().enumerate() {
                        let option = egui::Button::selectable(chosen == i, text.as_str());
                        if ui.add_enabled(*decodable, option).clicked() {
                            chosen = i;
                        }
                    }
                });
            theme::focus_ring(ui, &combo.response, theme::CONTROL_RADIUS);
            if chosen != current {
                item.edit.track = chosen;
                track_changed = true;
            }
            ui.add_space(8.0);
        }
        let mut louder = item.edit.louder(look.louder);
        if theme::checkbox(ui, &mut louder, t.louder, theme::body())
            .on_hover_text(t.louder_hint)
            .changed()
        {
            item.edit.set_louder(louder, look.louder);
        }
        // Only for a file the next run can take as it stands.
        if item.ready()
            && let Some(bytes) = item.estimate(look.format, look.kbps, look.mono)
        {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = fill(t.estimate, &[("size", &format_size(bytes))]);
                ui.add(egui::Label::new(RichText::new(text).color(p.muted)).truncate());
            });
        }
    });
    track_changed
}

/// How many icon buttons a row in this state shows; must match `row`.
fn action_count(status: &Status, idle: bool) -> usize {
    let remove = usize::from(idle && *status != Status::Converting);
    // The expander is always there.
    1 + remove
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

/// One background thread that reads each added file's length and size (of
/// its first audio track, or of the one chosen later).
fn spawn_prober(ctx: egui::Context) -> (Sender<ProbeRequest>, Receiver<ProbeResult>) {
    let (request_tx, request_rx) = channel::<ProbeRequest>();
    let (result_tx, result_rx) = channel();
    std::thread::spawn(move || {
        while let Ok((id, path, track)) = request_rx.recv() {
            let size = std::fs::metadata(&path).ok().map(|m| m.len());
            let info = std::panic::catch_unwind(|| crate::mf::probe_track(&path, track))
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

/// One background thread that lists a file's audio tracks when its row is
/// first expanded, so opening a large file never stalls the window. Apart
/// from the prober, so a folder still being probed does not hold it up.
fn spawn_track_reader(ctx: egui::Context) -> (Sender<(u64, PathBuf)>, Receiver<TrackResult>) {
    let (request_tx, request_rx) = channel::<(u64, PathBuf)>();
    let (result_tx, result_rx) = channel();
    std::thread::spawn(move || {
        while let Ok((id, path)) = request_rx.recv() {
            let tracks = std::panic::catch_unwind(|| crate::mf::audio_tracks(&path))
                .ok()
                .and_then(Result::ok);
            if result_tx.send((id, tracks)).is_err() {
                break;
            }
            ctx.request_repaint();
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
                    crate::convert::convert_file_with(&job.path, &options, &job.edits, &job.progress)
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
        self.collect_tracks();
        self.collect_forwarded();
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
        egui::Panel::bottom("controls")
            .frame(bare(12, 16))
            .show_separator_line(false)
            .show(ui_root, |ui| {
                self.codec_help(ui, p);
                self.options_card(ui, p);
                ui.add_space(12.0);
                self.result_bar(ui, p);
                self.limit_note(ui, p);
                self.action_area(ui, p);
            });
        egui::Panel::top("header")
            .frame(bare(12, 12))
            .show_separator_line(false)
            .show(ui_root, |ui| self.header(ui, p));
        egui::CentralPanel::default()
            .frame(bare(0, 0))
            .show(ui_root, |ui| self.list(ui, p));
        self.settings_flyout(&ctx, p);
        self.unlock_window(&ctx, p);
        self.feedback.show(&ctx, &self.version, self.shots.is_some());
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
        // Every row has its expander.
        assert_eq!(action_count(&Status::Waiting, true), 2);
        assert_eq!(action_count(&Status::Waiting, false), 1);
        assert_eq!(action_count(&Status::Converting, false), 1);
        assert_eq!(action_count(&done, true), 4);
        assert_eq!(action_count(&done, false), 3);
        assert_eq!(action_count(&failed, true), 3);
        assert_eq!(action_count(&failed, false), 1);
    }

    #[test]
    fn forwarded_files_wait_for_the_run_and_are_never_dropped() {
        let a = PathBuf::from("C:/a.mp4");
        let b = PathBuf::from("C:/b.mp3");
        let mut pending = Vec::new();
        assert!(take_forwarded(&mut pending, vec![a.clone()], true).is_empty());
        assert!(take_forwarded(&mut pending, vec![b.clone()], true).is_empty());
        assert!(take_forwarded(&mut pending, Vec::new(), true).is_empty());
        assert_eq!(pending, vec![a.clone(), b.clone()]);
        // Once the run is over, all of them, in the order they came.
        assert_eq!(take_forwarded(&mut pending, Vec::new(), false), vec![a.clone(), b.clone()]);
        assert!(pending.is_empty());
        assert_eq!(take_forwarded(&mut pending, vec![a.clone()], false), vec![a]);
        assert!(take_forwarded(&mut pending, Vec::new(), false).is_empty());
    }

    #[test]
    fn only_rows_in_view_are_laid_out() {
        let heights = [52.0, 136.0, 52.0, 52.0, 52.0];
        // Offsets 0, 52, 188, 240, 292; 344 in all.
        assert_eq!(visible_rows(&heights, 0.0, 100.0), 0..2);
        assert_eq!(visible_rows(&heights, 0.0, 52.0), 0..1);
        assert_eq!(visible_rows(&heights, 60.0, 200.0), 1..3);
        assert_eq!(visible_rows(&heights, 188.0, 240.0), 2..3);
        assert_eq!(visible_rows(&heights, 0.0, 1000.0), 0..5);
        assert_eq!(visible_rows(&heights, 300.0, 400.0), 4..5);
        assert_eq!(visible_rows(&heights, 400.0, 500.0), 5..5);
        assert_eq!(visible_rows(&[], 0.0, 100.0), 0..0);
    }

    #[test]
    fn a_rows_choices_shape_its_estimate_and_whether_it_converts() {
        let mut item = Item::new(1, PathBuf::from("C:/Media/talk.mp4"));
        item.probe = Probe::Known(Info { seconds: Some(300.0), rate: 48_000, channels: 2 });
        // 192 kbps for 5 minutes, then for the 1:45 kept.
        assert_eq!(item.estimate(Format::Mp3, 192, false), Some(7_200_000));
        item.edit.start = "0:30".to_owned();
        item.edit.end = "2:15".to_owned();
        assert_eq!(item.estimate(Format::Mp3, 192, false), Some(2_520_000));
        assert!(item.ready());
        // A trim that does not fit the file holds the file back.
        item.edit.start = "6:00".to_owned();
        assert!(!item.ready());
        assert_eq!(item.problem(), Some(Problem::Trim(crate::mf::TrimError::StartPastEnd)));
        item.edit.start = "0:30".to_owned();
        assert!(item.ready() && item.editable());
        item.status = Status::Converting;
        assert!(!item.ready() && !item.editable());
        // An expanded row is taller by its controls.
        assert_eq!(item.height(), ROW_HEIGHT);
        item.edit.expanded = true;
        assert_eq!(item.height(), ROW_HEIGHT + STRIP_HEIGHT);
    }

    /// The edits a row builds, through the real converter: a two-track file,
    /// the second track, 0:01 to 0:03, made louder.
    #[test]
    fn a_rows_choices_convert_for_real() {
        let dir = std::env::temp_dir().join(format!("mac-app-edits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("two tracks.mp4");
        // Track 2 is quiet, so Normalise has something to do.
        crate::mf::tests::write_aac_tracks(&src, None, &[0.5, 0.05], 5).unwrap();
        let mut item = Item::new(1, src.clone());
        item.probe = Probe::Known(crate::mf::probe(&src).unwrap());
        let tracks = crate::mf::audio_tracks(&src).unwrap();
        assert_eq!(tracks.len(), 2);
        item.edit = FileEdit {
            start: "0:01".to_owned(),
            end: "3".to_owned(),
            track: 1,
            ..FileEdit::default()
        };
        item.edit.set_louder(true, false);
        let edits = item.edit.edits(item.seconds(), false).unwrap();
        let options = Options { format: Format::Wav, kbps: 192, mono: false, out_dir: None };
        let progress = Progress::default();
        let out = crate::convert::convert_file_with(&src, &options, &edits, &progress).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as f64;
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as f64;
        let seconds = data / (rate * 4.0);
        assert!((seconds - 2.0).abs() < 0.01, "{seconds}");
        let peak = bytes[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
            .max()
            .unwrap();
        // Brought from about -26 dBFS up to -1 dBFS.
        assert!(peak > 28_000, "{peak}");
        assert_eq!(progress.fraction(), Some(1.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overall_progress_weights_each_file() {
        assert_eq!(overall(&[]), 0.0);
        assert_eq!(overall(&[1.0, 0.5, 0.0, 0.5]), 0.5);
        assert_eq!(overall(&[2.0]), 1.0);
    }

    #[test]
    fn presets_set_existing_options_and_are_recognised() {
        for preset in Preset::ALL {
            let c = preset.settings();
            assert!(c.format.bitrates().contains(&c.kbps), "{preset:?}");
            assert_eq!(preset_of(c), Some(preset));
        }
        assert_eq!(Format::Mp3.effective_kbps(192, true), 96);
        assert_eq!(Format::Mp3.effective_kbps(256, false), 256);
        assert_eq!(Format::M4a.effective_kbps(96, false), 96);
        let choice = |format, kbps, mono, louder| Choice { format, kbps, mono, louder };
        // Voice makes speech louder; the others leave the volume alone.
        assert!(Preset::Voice.settings().louder);
        assert!(!Preset::Music.settings().louder && !Preset::Smallest.settings().louder);
        // Changing a detail leaves the preset, "Make louder" included.
        assert_eq!(preset_of(choice(Format::Mp3, 320, false, false)), None);
        assert_eq!(preset_of(choice(Format::Mp3, 192, false, false)), None);
        assert_eq!(preset_of(choice(Format::Wav, 192, false, false)), None);
        assert_eq!(preset_of(choice(Format::Mp3, 192, true, false)), None);
        assert_eq!(preset_of(choice(Format::Mp3, 256, false, true)), None);
        assert_eq!(preset_of(choice(Format::Mp3, 192, true, true)), Some(Preset::Voice));
        let t = Lang::En.strings();
        assert_eq!(Preset::Voice.describe(t), "MP3 · 96 kbps · Mono (voice) · Make louder");
        assert_eq!(Preset::Smallest.describe(t), "M4A · 96 kbps");
        // Applying a preset sets every option it names, and is then shown.
        let mut settings = crate::settings::Settings::default();
        Preset::Voice.settings().apply(&mut settings);
        assert!(settings.louder && settings.mono);
        assert_eq!(preset_of(Choice::of(&settings)), Some(Preset::Voice));
    }

    #[test]
    fn the_free_limit_reads_as_next_batch() {
        let t = Lang::En.strings();
        assert_eq!(primary_label(t, 8, 3), "Convert next 3");
        assert_eq!(primary_label(t, 3, 3), "Convert 3 files");
        assert_eq!(primary_label(t, 1, 1), "Convert 1 file");
        assert_eq!(primary_label(t, 0, 0), "Convert");
        assert!(limit_note_visible(8, 3, false));
        assert!(!limit_note_visible(8, 3, true));
        assert!(!limit_note_visible(3, 3, false));
        assert!(!limit_note_visible(8, 8, false));
    }

    #[test]
    fn rate_link_needs_a_store_id_and_a_clean_run() {
        assert_eq!(review_url(""), None);
        assert_eq!(review_url("9NBLGGH4NNS"), None);
        assert_eq!(review_url("9N-LGGH4NNS1"), None);
        assert!(review_url("XP99D661MJWKBX").is_some());
        assert_eq!(
            review_url("9NBLGGH4NNS1").as_deref(),
            Some("ms-windows-store://review/?ProductId=9NBLGGH4NNS1")
        );
        let clean = Summary { converted: 3, failed: 0, in_bytes: 1, out_bytes: 1 };
        let failed = Summary { failed: 1, ..clean };
        let nothing = Summary { converted: 0, ..clean };
        assert!(show_rate_link(&clean, "9NBLGGH4NNS1"));
        assert!(!show_rate_link(&failed, "9NBLGGH4NNS1"));
        assert!(!show_rate_link(&nothing, "9NBLGGH4NNS1"));
        assert!(!show_rate_link(&clean, STORE_ID) || review_url(STORE_ID).is_some());
    }
}
