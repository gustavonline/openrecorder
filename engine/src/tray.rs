//! The status icon and its menu.
//!
//! Everything a person sees here is computed by pure functions from one `TrayState`, which the native
//! app fills from its single capture state machine. A platform port only draws a finished `View`, so
//! the rules that matter (one icon, a timer that exists only while recording, labels that are gone
//! the moment a take ends) are tested without any menu bar, tray host or window.
//!
//! * macOS and Windows: `tray-icon` (a native status item / notification icon).
//! * Linux: `ksni`, a StatusNotifierItem over the D-Bus connection the app already links. No GTK.
//! * Anywhere else, or when no tray host exists: `Tray::new` returns an error and the app falls back
//!   to Home, which carries the same controls.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

// ---------------------------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------------------------

/// Where the one capture state machine stands. Terminal outcomes (saved, failed, cancelled,
/// interrupted) are all `Idle` here: they leave a notice or a Recent entry, never a status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Countdown(u32),
    /// Inputs are being checked and opened; nothing is captured yet.
    Starting,
    /// Whole seconds since recording began, computed by the caller as now - started_at.
    Recording(u64),
    Stopping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Off,
    /// Chosen to start with Record; nothing has been opened.
    Armed,
    /// Being opened for a take that is starting.
    Starting,
    On,
    NotConnected,
    AccessOff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub uid: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub mark: Mark,
    pub devices: Vec<Device>,
    /// The chosen device's name, when the system has told us it. Never a made-up one.
    pub name: Option<String>,
}
impl Input {
    pub fn off() -> Self {
        Self { mark: Mark::Off, devices: Vec::new(), name: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    pub id: u32,
    pub label: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    /// The take's files exist: the one next step is to open the recording.
    pub kept: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayState {
    pub phase: Phase,
    pub camera: Input,
    pub microphone: Input,
    pub source_label: String,
    pub displays: Vec<Display>,
    pub window_selected: bool,
    pub capability_line: String,
    pub shape: String,
    pub size: u32,
    pub mirror: bool,
    /// Whether the camera bubble is drawn, independent of the camera being on.
    pub show_camera: bool,
    /// Up to three; (take id, "label · duration").
    pub recent: Vec<(String, String)>,
    pub notice: Option<Notice>,
}
impl TrayState {
    pub fn idle() -> Self {
        Self {
            phase: Phase::Idle,
            camera: Input::off(),
            microphone: Input::off(),
            source_label: "Display".into(),
            displays: Vec::new(),
            window_selected: false,
            capability_line: String::new(),
            shape: "circle".into(),
            size: 168,
            mirror: true,
            show_camera: true,
            recent: Vec::new(),
            notice: None,
        }
    }
}

/// Everything a menu choice can ask for. Nothing here opens an input by itself: only `Record` and an
/// explicit camera or microphone toggle do, and those are decided by the app, not by this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Record,
    Stop,
    CancelCountdown,
    CameraOn(bool),
    CameraDevice(Option<String>),
    MicOn(bool),
    MicDevice(Option<String>),
    MicDefault,
    Display(u32),
    ChooseWindow,
    Shape(&'static str),
    Size(u32),
    Mirror(bool),
    ShowCamera(bool),
    OpenRecent(String),
    /// The newest recording that was stopped early, in Home.
    OpenLastRecording,
    OpenHome,
    /// Home's Settings route (there is no separate native Settings window).
    Settings,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Action(Action),
    Submenu(Vec<Item>),
    Separator,
    /// A line of text with no action: a subtitle, a section name, a capability line.
    Note,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub kind: Kind,
}
impl Item {
    fn act(label: impl Into<String>, action: Action) -> Self {
        Self { label: label.into(), enabled: true, checked: None, kind: Kind::Action(action) }
    }
    fn note(label: impl Into<String>) -> Self {
        Self { label: label.into(), enabled: false, checked: None, kind: Kind::Note }
    }
    fn sep() -> Self {
        Self { label: String::new(), enabled: true, checked: None, kind: Kind::Separator }
    }
    fn sub(label: impl Into<String>, enabled: bool, items: Vec<Item>) -> Self {
        Self { label: label.into(), enabled, checked: None, kind: Kind::Submenu(items) }
    }
    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    fn check(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconKind {
    Disc,
    Square,
}

/// A finished picture of the tray: the only thing a port is asked to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub icon: IconKind,
    /// Text beside the icon on hosts that show it. Empty means none, explicitly.
    pub title: String,
    pub tooltip: String,
    pub spoken: String,
    pub items: Vec<Item>,
}

pub fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn title_for(phase: Phase) -> String {
    match phase {
        Phase::Idle => String::new(),
        Phase::Countdown(n) => n.to_string(),
        Phase::Starting => String::new(),
        Phase::Recording(s) => clock(s),
        Phase::Stopping => "Saving…".into(),
    }
}

/// The one status vocabulary: Off, On (chosen, not live), Starting…, Live, and the reasons it needs attention.
fn status_word(mark: Mark) -> &'static str {
    match mark {
        Mark::Off => "Off",
        Mark::Armed => "On",
        Mark::Starting => "Starting…",
        Mark::On => "Live",
        Mark::NotConnected => "Not connected",
        Mark::AccessOff => "Access off",
    }
}
/// "FaceTime HD Camera · On", or just the word when no name is known.
fn input_value(input: &Input) -> String {
    match &input.name {
        Some(name) if input.mark != Mark::AccessOff => format!("{name} · {}", status_word(input.mark)),
        _ => status_word(input.mark).to_owned(),
    }
}

/// Names only the inputs that Record would start.
pub fn starts_label(camera: bool, microphone: bool) -> Option<&'static str> {
    match (camera, microphone) {
        (true, true) => Some("Turns on Camera and Microphone first"),
        (true, false) => Some("Turns on Camera first"),
        (false, true) => Some("Turns on Microphone first"),
        (false, false) => None,
    }
}

const SIZES: [(&str, u32); 3] = [("Small", 120), ("Medium", 168), ("Large", 224)];
const SHAPES: [(&str, &str); 4] = [("Circle", "circle"), ("Rounded Square", "rounded"), ("Wide 16:9", "wide"), ("Portrait 9:16", "portrait")];

fn size_selected(current: u32, option: u32) -> bool {
    // The older size set maps position for position onto the current one.
    let all = [120, 168, 224, 140, 200, 280];
    all.iter().position(|s| *s == current).map(|i| i % 3) == all.iter().position(|s| *s == option).map(|i| i % 3)
}

fn camera_menu(s: &TrayState) -> Item {
    let idle = s.phase == Phase::Idle;
    let recording = matches!(s.phase, Phase::Recording(_));
    // Shape, size and mirror stay live while recording; opening or closing the device does not.
    let live = idle || recording;
    let device_edit = idle;
    let mut items = Vec::new();
    let on = matches!(s.camera.mark, Mark::On | Mark::Armed | Mark::Starting);
    let toggle = match s.camera.mark {
        Mark::AccessOff => Item::note("Camera access is off in system settings"),
        _ if on => Item::act("Turn Off Camera", Action::CameraOn(false)).enabled(device_edit),
        _ => Item::act("Turn On Camera", Action::CameraOn(true)).enabled(device_edit && s.camera.mark != Mark::NotConnected),
    };
    items.push(toggle);
    if recording {
        items.push(Item::note("Unavailable while recording"));
    }
    items.push(Item::sep());
    items.push(Item::note("Device"));
    let none_selected = !s.camera.devices.iter().any(|d| d.selected);
    items.push(Item::act("No Camera", Action::CameraDevice(None)).check(none_selected).enabled(device_edit));
    for d in &s.camera.devices {
        items.push(Item::act(d.label.clone(), Action::CameraDevice(Some(d.uid.clone()))).check(d.selected).enabled(device_edit));
    }
    items.push(Item::sep());
    items.push(Item::act("Show Camera", Action::ShowCamera(!s.show_camera)).check(s.show_camera).enabled(live));
    items.push(Item::sep());
    items.push(Item::note("Shape"));
    for (label, value) in SHAPES {
        items.push(Item::act(label, Action::Shape(value)).check(s.shape == value).enabled(live));
    }
    items.push(Item::sep());
    items.push(Item::note("Size"));
    for (label, value) in SIZES {
        items.push(Item::act(label, Action::Size(value)).check(size_selected(s.size, value)).enabled(live));
    }
    items.push(Item::sep());
    items.push(Item::act("Mirror Preview", Action::Mirror(!s.mirror)).check(s.mirror).enabled(live));
    let label = format!("Camera: {}", input_value(&s.camera));
    Item::sub(label, matches!(s.phase, Phase::Idle | Phase::Recording(_)), items)
}

fn microphone_menu(s: &TrayState) -> Item {
    let idle = s.phase == Phase::Idle;
    let mut items = Vec::new();
    let on = matches!(s.microphone.mark, Mark::On | Mark::Armed | Mark::Starting);
    items.push(match s.microphone.mark {
        Mark::AccessOff => Item::note("Microphone access is off in system settings"),
        _ if on => Item::act("Turn Off Microphone", Action::MicOn(false)),
        _ => Item::act("Turn On Microphone", Action::MicOn(true)).enabled(s.microphone.mark != Mark::NotConnected),
    });
    items.push(Item::sep());
    let none_selected = !s.microphone.devices.iter().any(|d| d.selected);
    items.push(Item::act("No Microphone", Action::MicDevice(None)).check(false));
    items.push(Item::act("System Default", Action::MicDefault).check(none_selected));
    for d in &s.microphone.devices {
        items.push(Item::act(d.label.clone(), Action::MicDevice(Some(d.uid.clone()))).check(d.selected));
    }
    if !idle {
        for item in &mut items {
            item.enabled = false;
        }
    }
    Item::sub(format!("Microphone: {}", input_value(&s.microphone)), idle, items)
}

fn source_menu(s: &TrayState) -> Item {
    let idle = s.phase == Phase::Idle;
    let mut items: Vec<Item> = s
        .displays
        .iter()
        .map(|d| Item::act(d.label.clone(), Action::Display(d.id)).check(d.selected && !s.window_selected))
        .collect();
    if !items.is_empty() {
        items.push(Item::sep());
    }
    items.push(Item::act("Choose Window…", Action::ChooseWindow).check(s.window_selected));
    if !s.capability_line.is_empty() {
        items.push(Item::sep());
        items.push(Item::note(s.capability_line.clone()));
    }
    if !idle {
        for item in &mut items {
            if item.kind != Kind::Separator {
                item.enabled = false;
            }
        }
    }
    Item::sub(format!("Source: {}", s.source_label), idle, items)
}

/// The whole menu for a state. Nothing here keeps a timer, a "stopped" line or any other status of
/// its own: when the phase is idle, none of it can be produced.
pub fn menu(s: &TrayState) -> Vec<Item> {
    let mut out = Vec::new();
    match s.phase {
        Phase::Idle => {
            out.push(Item::act("Record", Action::Record));
            let (cam, mic) = (matches!(s.camera.mark, Mark::Armed), matches!(s.microphone.mark, Mark::Armed));
            if let Some(line) = starts_label(cam, mic) {
                out.push(Item::note(line));
            }
        }
        Phase::Countdown(n) => out.push(Item::act(format!("Cancel Countdown ({n})"), Action::CancelCountdown)),
        Phase::Starting => out.push(Item::act("Cancel Start", Action::CancelCountdown)),
        Phase::Recording(secs) => out.push(Item::act(format!("Stop Recording ({})", clock(secs)), Action::Stop)),
        Phase::Stopping => out.push(Item::note("Saving…")),
    }
    out.push(Item::sep());
    out.push(source_menu(s));
    out.push(camera_menu(s));
    out.push(microphone_menu(s));
    out.push(Item::sep());
    if s.phase == Phase::Idle {
        if let Some(notice) = &s.notice {
            if notice.kept {
                out.push(Item::note("Recording stopped early"));
                out.push(Item::note("Available files were kept."));
                out.push(Item::act("Open Last Recording", Action::OpenLastRecording));
            } else {
                out.push(Item::note(notice.text.clone()));
            }
            out.push(Item::sep());
        } else if !s.recent.is_empty() {
            out.push(Item::note("Recent"));
            for (id, label) in s.recent.iter().take(3) {
                out.push(Item::act(label.clone(), Action::OpenRecent(id.clone())));
            }
            out.push(Item::sep());
        }
    }
    out.push(Item::act("Open Home", Action::OpenHome));
    out.push(Item::act("Settings…", Action::Settings));
    out.push(Item::sep());
    let quit = if s.phase == Phase::Idle { "Quit OpenRecorder" } else { "Quit OpenRecorder…" };
    out.push(Item::act(quit, Action::Quit));
    out
}

/// What the tray shows for a state. A recording is the only state with a square and a timer.
pub fn view(s: &TrayState) -> View {
    let icon = if matches!(s.phase, Phase::Recording(_)) { IconKind::Square } else { IconKind::Disc };
    let title = title_for(s.phase);
    let (tooltip, spoken) = match s.phase {
        Phase::Idle => ("OpenRecorder".to_owned(), "OpenRecorder, idle".to_owned()),
        Phase::Countdown(n) => (format!("OpenRecorder — Starting in {n}"), format!("OpenRecorder, starting in {n}")),
        Phase::Starting => ("OpenRecorder — Starting".to_owned(), "OpenRecorder, starting".to_owned()),
        Phase::Recording(secs) => (format!("OpenRecorder — Recording {}", clock(secs)), format!("OpenRecorder, recording, {secs} seconds")),
        Phase::Stopping => ("OpenRecorder — Saving".to_owned(), "OpenRecorder, saving".to_owned()),
    };
    View { icon, title, tooltip, spoken, items: menu(s) }
}

/// Alpha mask for the idle icon: a solid disc, no ring and no centre mark.
pub fn disc_alpha(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let centre = (size as f32 - 1.) / 2.;
    let radius = size as f32 * 0.31;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
            if d <= radius {
                rgba[(y * size + x) * 4 + 3] = 255;
            }
        }
    }
    rgba
}
/// Alpha mask for the recording icon: a solid square with softly rounded corners.
pub fn square_alpha(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let half = size as f32 * 0.27;
    let corner = size as f32 * 0.06;
    let centre = (size as f32 - 1.) / 2.;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - centre).abs();
            let dy = (y as f32 - centre).abs();
            let qx = (dx - (half - corner)).max(0.);
            let qy = (dy - (half - corner)).max(0.);
            if qx * qx + qy * qy <= corner * corner {
                rgba[(y * size + x) * 4 + 3] = 255;
            }
        }
    }
    rgba
}
pub fn icon_alpha(kind: IconKind, size: usize) -> Vec<u8> {
    match kind {
        IconKind::Disc => disc_alpha(size),
        IconKind::Square => square_alpha(size),
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing: a port draws views; `Tray` decides when
// ---------------------------------------------------------------------------------------------

pub trait Port {
    fn draw(&mut self, view: &View);
}

/// A fact that has been asked about and may not have been answered yet.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Seen {
    #[default]
    Unknown,
    Yes,
    No,
}
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Seen {
    pub fn from(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::Yes,
            Some(false) => Self::No,
            None => Self::Unknown,
        }
    }
}

/// Whether an icon would actually be shown, from three separate facts. On Linux a StatusNotifierWatcher owning its
/// bus name is **not** a tray: a watcher can run with no host displaying anything, and a host can disappear while
/// the watcher stays. Only a registered item, an owned watcher name and a registered host together count.
/// macOS and Windows have no such split and start out present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Availability {
    /// Our item is registered with the watcher (set by the tray service when it registers or re-registers).
    pub registered: bool,
    /// The watcher owns `org.kde.StatusNotifierWatcher`.
    pub watcher: Seen,
    /// The watcher reports a registered host (`IsStatusNotifierHostRegistered`).
    pub host: Seen,
}
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Availability {
    pub const PRESENT: Self = Self { registered: true, watcher: Seen::Yes, host: Seen::Yes };
    /// Linux before anything has been confirmed: nothing is claimed.
    pub const UNCONFIRMED: Self = Self { registered: true, watcher: Seen::Unknown, host: Seen::Unknown };
    /// A fresh reading of the bus. A missing watcher says nothing about hosts, so that fact is forgotten with it.
    pub fn read(&mut self, watcher: Seen, host: Seen) {
        self.watcher = watcher;
        self.host = if watcher == Seen::Yes { host } else { Seen::Unknown };
    }
    pub fn lost(&self) -> bool {
        !(self.registered && self.watcher == Seen::Yes && self.host == Seen::Yes)
    }
    /// Plain words for why, for Home. Unknown is not "fine": it is said as unconfirmed.
    pub fn reason(&self) -> Option<&'static str> {
        if !self.lost() {
            return None;
        }
        Some(match (self.watcher, self.host, self.registered) {
            (Seen::No, ..) => "The desktop's tray service isn't running",
            (Seen::Yes, Seen::No, _) => "The desktop's tray service is running, but nothing is showing tray icons",
            (Seen::Yes, Seen::Yes, false) => "OpenRecorder's icon isn't registered with the desktop's tray yet",
            _ => "The desktop's tray hasn't been confirmed yet",
        })
    }
}
/// Shared with the tray service threads, which update it as the bus changes.
pub type Shared = Arc<Mutex<Availability>>;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn change(shared: &Shared, ctx: &eframe::egui::Context, update: impl FnOnce(&mut Availability)) {
    let mut held = shared.lock().unwrap_or_else(|e| e.into_inner());
    let before = *held;
    update(&mut held);
    if *held != before {
        ctx.request_repaint();
    }
}

/// Actions chosen from the menu, waiting for the app. Filled from the host's own thread.
#[derive(Clone, Default)]
pub struct Queue(Arc<Mutex<VecDeque<Action>>>);
impl Queue {
    pub fn push(&self, action: Action) {
        let mut held = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if held.len() < 16 {
            held.push_back(action);
        }
    }
    pub fn drain(&self) -> Vec<Action> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }
}

pub struct Tray {
    port: Box<dyn Port>,
    last: Option<View>,
    queue: Queue,
    availability: Shared,
}
impl Tray {
    /// Starts the platform tray. An error means there is no tray here and the app must say so and use Home.
    pub fn new(ctx: eframe::egui::Context) -> Result<Self, String> {
        let queue = Queue::default();
        let availability: Shared = Arc::new(Mutex::new(if cfg!(target_os = "linux") { Availability::UNCONFIRMED } else { Availability::PRESENT }));
        let port = open(ctx, queue.clone(), availability.clone())?;
        Ok(Self { port, last: None, queue, availability })
    }
    #[cfg(test)]
    fn with_port(port: Box<dyn Port>) -> Self {
        Self { port, last: None, queue: Queue::default(), availability: Arc::new(Mutex::new(Availability::PRESENT)) }
    }
    /// Draws the state if, and only if, what a person would see has changed.
    pub fn show(&mut self, state: &TrayState) {
        let next = view(state);
        if self.last.as_ref() != Some(&next) {
            self.port.draw(&next);
            self.last = Some(next);
        }
    }
    pub fn actions(&self) -> Vec<Action> {
        self.queue.drain()
    }
    /// The desktop's tray host went away after we registered (Linux). The app shows Home's controls
    /// until it returns; nothing is lost because Home carries the same controls.
    pub fn host_lost(&self) -> bool {
        self.availability.lock().unwrap_or_else(|e| e.into_inner()).lost()
    }
    /// Why the tray can't be used right now, when it can't.
    pub fn lost_reason(&self) -> Option<&'static str> {
        self.availability.lock().unwrap_or_else(|e| e.into_inner()).reason()
    }
    #[cfg(test)]
    pub fn availability(&self) -> Shared {
        self.availability.clone()
    }
}

// ---------------------------------------------------------------------------------------------
// macOS and Windows: tray-icon
// ---------------------------------------------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn open(ctx: eframe::egui::Context, queue: Queue, _availability: Shared) -> Result<Box<dyn Port>, String> {
    native::Native::open(ctx, queue).map(|n| Box::new(n) as Box<dyn Port>)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native {
    use super::*;
    use std::collections::HashMap;
    use tray_icon::{
        menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu},
        Icon, TrayIcon, TrayIconBuilder,
    };

    const SIZE: usize = 32;

    pub struct Native {
        icon: TrayIcon,
        shown: Option<IconKind>,
        title_set: bool,
        /// Shape of the menu that is installed, to decide between editing it and rebuilding it.
        structure: Option<Vec<Shape>>,
        handles: Vec<Handle>,
        actions: Arc<Mutex<HashMap<MenuId, Action>>>,
    }
    #[derive(Clone, PartialEq, Eq)]
    enum Shape {
        Leaf { checkable: bool, note: bool },
        Sub(Vec<Shape>),
        Sep,
    }
    enum Handle {
        Plain(MenuItem),
        Check(CheckMenuItem),
        Sub(Submenu),
        None,
    }
    fn shape(items: &[Item]) -> Vec<Shape> {
        items
            .iter()
            .map(|i| match &i.kind {
                Kind::Separator => Shape::Sep,
                Kind::Submenu(inner) => Shape::Sub(shape(inner)),
                Kind::Action(_) | Kind::Note => Shape::Leaf { checkable: i.checked.is_some(), note: i.kind == Kind::Note },
            })
            .collect()
    }
    fn icon(kind: IconKind) -> Result<Icon, String> {
        // The mask is drawn as a template image: the system tints it for light, dark and the menu bar.
        Icon::from_rgba(icon_alpha(kind, SIZE), SIZE as u32, SIZE as u32).map_err(|e| e.to_string())
    }
    impl Native {
        pub fn open(ctx: eframe::egui::Context, queue: Queue) -> Result<Self, String> {
            let actions: Arc<Mutex<HashMap<MenuId, Action>>> = Arc::default();
            let table = actions.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let action = table.lock().unwrap_or_else(|e| e.into_inner()).get(&event.id).cloned();
                if let Some(action) = action {
                    queue.push(action);
                    ctx.request_repaint();
                }
            }));
            let tray = TrayIconBuilder::new()
                .with_tooltip("OpenRecorder")
                .with_icon_as_template(true)
                .with_icon(icon(IconKind::Disc)?)
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Self { icon: tray, shown: Some(IconKind::Disc), title_set: false, structure: None, handles: Vec::new(), actions })
        }
        fn install(&mut self, items: &[Item]) -> Result<(), String> {
            let menu = Menu::new();
            let mut table = HashMap::new();
            let mut handles = Vec::new();
            fn add(items: &[Item], parent: &dyn Fn(&dyn IsMenuItem) -> Result<(), String>, table: &mut HashMap<MenuId, Action>, handles: &mut Vec<Handle>) -> Result<(), String> {
                for item in items {
                    match &item.kind {
                        Kind::Separator => {
                            parent(&PredefinedMenuItem::separator())?;
                            handles.push(Handle::None);
                        }
                        Kind::Submenu(inner) => {
                            let sub = Submenu::new(&item.label, item.enabled);
                            parent(&sub)?;
                            handles.push(Handle::Sub(sub.clone()));
                            let target = sub.clone();
                            add(inner, &move |i| target.append(i).map_err(|e| e.to_string()), table, handles)?;
                        }
                        Kind::Action(_) | Kind::Note => match item.checked {
                            Some(checked) => {
                                let m = CheckMenuItem::new(&item.label, item.enabled, checked, None);
                                parent(&m)?;
                                if let Kind::Action(a) = &item.kind {
                                    table.insert(m.id().clone(), a.clone());
                                }
                                handles.push(Handle::Check(m));
                            }
                            None => {
                                let m = MenuItem::new(&item.label, item.enabled, None);
                                parent(&m)?;
                                if let Kind::Action(a) = &item.kind {
                                    table.insert(m.id().clone(), a.clone());
                                }
                                handles.push(Handle::Plain(m));
                            }
                        },
                    }
                }
                Ok(())
            }
            let root = menu.clone();
            add(items, &move |i| root.append(i).map_err(|e| e.to_string()), &mut table, &mut handles)?;
            *self.actions.lock().unwrap_or_else(|e| e.into_inner()) = table;
            self.icon.set_menu(Some(Box::new(menu)));
            self.structure = Some(shape(items));
            self.handles = handles;
            Ok(())
        }
        /// Edit the installed items in place, in the same order the tree was built.
        fn edit(&mut self, items: &[Item]) {
            fn walk<'a>(items: &'a [Item], out: &mut Vec<&'a Item>) {
                for i in items {
                    out.push(i);
                    if let Kind::Submenu(inner) = &i.kind {
                        walk(inner, out);
                    }
                }
            }
            let mut flat = Vec::new();
            walk(items, &mut flat);
            let mut table = HashMap::new();
            for (item, handle) in flat.iter().zip(&self.handles) {
                match handle {
                    Handle::Plain(m) => {
                        m.set_text(&item.label);
                        m.set_enabled(item.enabled);
                        if let Kind::Action(a) = &item.kind {
                            table.insert(m.id().clone(), a.clone());
                        }
                    }
                    Handle::Check(m) => {
                        m.set_text(&item.label);
                        m.set_enabled(item.enabled);
                        m.set_checked(item.checked.unwrap_or(false));
                        if let Kind::Action(a) = &item.kind {
                            table.insert(m.id().clone(), a.clone());
                        }
                    }
                    Handle::Sub(s) => {
                        s.set_text(&item.label);
                        s.set_enabled(item.enabled);
                    }
                    Handle::None => {}
                }
            }
            *self.actions.lock().unwrap_or_else(|e| e.into_inner()) = table;
        }
    }
    impl Port for Native {
        fn draw(&mut self, view: &View) {
            if self.shown != Some(view.icon) {
                if let Ok(i) = icon(view.icon) {
                    // Swapping the image is what changes the shape; the title below is text only.
                    self.icon.set_icon_with_as_template(Some(i), true).ok();
                    self.shown = Some(view.icon);
                }
            }
            // `set_title(None)` is a no-op in tray-icon on macOS, so a cleared title is set to the empty
            // string, which is acted on. A stale timer therefore cannot survive the end of a take.
            if !view.title.is_empty() || self.title_set {
                self.icon.set_title(Some(view.title.as_str()));
                self.title_set = !view.title.is_empty();
            }
            self.icon.set_tooltip(Some(view.tooltip.as_str())).ok();
            let wanted = shape(&view.items);
            if self.structure.as_ref() == Some(&wanted) {
                self.edit(&view.items);
            } else {
                let _ = self.install(&view.items);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Linux: ksni (StatusNotifierItem), no GTK
// ---------------------------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn open(ctx: eframe::egui::Context, queue: Queue, availability: Shared) -> Result<Box<dyn Port>, String> {
    linux::Linux::open(ctx, queue, availability).map(|n| Box::new(n) as Box<dyn Port>)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use ksni::{blocking::TrayMethods, menu, MenuItem};

    struct Sni {
        view: View,
        queue: Queue,
        ctx: eframe::egui::Context,
        availability: Shared,
    }
    impl Sni {
        fn send(&self, action: Action) {
            self.queue.push(action);
            self.ctx.request_repaint();
        }
        fn convert(&self, items: &[Item]) -> Vec<MenuItem<Sni>> {
            items
                .iter()
                .map(|item| match &item.kind {
                    Kind::Separator => MenuItem::Separator,
                    Kind::Submenu(inner) => menu::SubMenu {
                        label: item.label.clone(),
                        enabled: item.enabled,
                        submenu: self.convert(inner),
                        ..Default::default()
                    }
                    .into(),
                    Kind::Note => menu::StandardItem { label: item.label.clone(), enabled: false, ..Default::default() }.into(),
                    Kind::Action(action) => {
                        let action = action.clone();
                        let activate: Box<dyn Fn(&mut Sni) + Send> = Box::new(move |t: &mut Sni| t.send(action.clone()));
                        match item.checked {
                            Some(checked) => menu::CheckmarkItem { label: item.label.clone(), enabled: item.enabled, checked, activate, ..Default::default() }.into(),
                            None => menu::StandardItem { label: item.label.clone(), enabled: item.enabled, activate, ..Default::default() }.into(),
                        }
                    }
                })
                .collect()
        }
    }
    /// ARGB32 in network byte order, white: hosts draw it as a symbolic icon.
    fn pixmap(kind: IconKind, size: usize) -> ksni::Icon {
        let mask = icon_alpha(kind, size);
        let mut data = Vec::with_capacity(size * size * 4);
        for px in mask.chunks_exact(4) {
            data.extend_from_slice(&[px[3], 255, 255, 255]);
        }
        ksni::Icon { width: size as i32, height: size as i32, data }
    }
    impl ksni::Tray for Sni {
        fn id(&self) -> String {
            "online.openrecorder.desktop".into()
        }
        fn title(&self) -> String {
            // A host that shows the title beside the icon gets the timer; others see it in the tooltip.
            if self.view.title.is_empty() { "OpenRecorder".into() } else { format!("OpenRecorder {}", self.view.title) }
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![pixmap(self.view.icon, 22), pixmap(self.view.icon, 32)]
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: self.view.tooltip.clone(), ..Default::default() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            // A host that only sends Activate (no menu) opens Home.
            self.send(Action::OpenHome);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            self.convert(&self.view.items)
        }
        fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
            // Keep the service: it re-registers when a watcher returns. Until then Home shows the controls.
            change(&self.availability, &self.ctx, |a| a.registered = false);
            true
        }
        fn watcher_online(&self) {
            // ksni registers the item again right after this. Whether anything is *showing* it is a separate
            // question that the monitor below answers; being registered is not enough.
            change(&self.availability, &self.ctx, |a| a.registered = true);
        }
    }
    pub struct Linux {
        handle: ksni::blocking::Handle<Sni>,
    }
    impl Linux {
        pub fn open(ctx: eframe::egui::Context, queue: Queue, availability: Shared) -> Result<Self, String> {
            monitor::start(ctx.clone(), availability.clone());
            let tray = Sni { view: view(&TrayState::idle()), queue, ctx, availability };
            // Without a StatusNotifierWatcher (or any host to show it) this returns an error, and the
            // caller uses Home. Nothing is polled: a watcher that appears later is reported by the service.
            // `assume_sni_available(true)` routes "no watcher yet" to `watcher_offline` instead of failing here, so a
            // tray host that starts later is picked up from the bus's NameOwnerChanged with no polling.
            let handle = tray.assume_sni_available(true).spawn().map_err(|e| e.to_string())?;
            Ok(Self { handle })
        }
    }
    impl Port for Linux {
        fn draw(&mut self, view: &View) {
            let next = view.clone();
            let _ = self.handle.update(move |t| t.view = next);
        }
    }
}

/// Watches the StatusNotifierWatcher and its hosts on the session bus, driven only by the bus's own signals (no
/// polling): the watcher's name appearing or disappearing, and the watcher announcing that a host registered or
/// unregistered. Each signal makes one bounded re-read (3 s method timeout) of the same two facts, so a host that
/// vanishes while the watcher stays is noticed, which the tray service's own callbacks (watcher ownership only)
/// cannot do. Uses the `zbus` already behind `ksni`; nothing here speaks the protocol by hand beyond one property
/// and three signals declared through zbus's own proxy macro.
#[cfg(target_os = "linux")]
mod monitor {
    use super::*;
    use std::time::Duration;

    const WATCHER: &str = "org.kde.StatusNotifierWatcher";

    #[zbus::proxy(
        interface = "org.kde.StatusNotifierWatcher",
        default_service = "org.kde.StatusNotifierWatcher",
        default_path = "/StatusNotifierWatcher",
        gen_async = false,
        blocking_name = "WatcherProxy"
    )]
    trait Watcher {
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> zbus::Result<bool>;
        #[zbus(signal)]
        fn status_notifier_host_registered(&self) -> zbus::Result<()>;
        #[zbus(signal)]
        fn status_notifier_host_unregistered(&self) -> zbus::Result<()>;
    }

    /// One reading: does the watcher own its name, and does it report a registered host.
    fn read(conn: &zbus::blocking::Connection) -> (Seen, Seen) {
        let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(conn) else { return (Seen::Unknown, Seen::Unknown) };
        let Ok(name) = zbus::names::BusName::try_from(WATCHER) else { return (Seen::Unknown, Seen::Unknown) };
        match bus.name_has_owner(name) {
            Ok(false) => (Seen::No, Seen::Unknown),
            Err(_) => (Seen::Unknown, Seen::Unknown),
            Ok(true) => {
                let host = WatcherProxy::builder(conn)
                    .cache_properties(zbus::proxy::CacheProperties::No)
                    .build()
                    .and_then(|proxy| proxy.is_status_notifier_host_registered())
                    .ok();
                (Seen::Yes, Seen::from(host))
            }
        }
    }
    fn refresh(conn: &zbus::blocking::Connection, shared: &Shared, ctx: &eframe::egui::Context, one_at_a_time: &Mutex<()>) {
        let _turn = one_at_a_time.lock().unwrap_or_else(|e| e.into_inner());
        let (watcher, host) = read(conn);
        change(shared, ctx, |a| a.read(watcher, host));
    }

    pub fn start(ctx: eframe::egui::Context, shared: Shared) {
        let _ = std::thread::Builder::new().name("tray-presence".into()).spawn(move || {
            let Ok(builder) = zbus::blocking::connection::Builder::session() else { return };
            let Ok(conn) = builder.method_timeout(Duration::from_secs(3)).build() else { return };
            let turn = Arc::new(Mutex::new(()));
            // Subscribe before the first reading so nothing that happens in between is missed.
            let mut sources: Vec<Box<dyn FnOnce() + Send>> = Vec::new();
            if let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(&conn) {
                if let Ok(owner) = bus.receive_name_owner_changed_with_args(&[(0, WATCHER)]) {
                    let (conn, shared, ctx, turn) = (conn.clone(), shared.clone(), ctx.clone(), turn.clone());
                    sources.push(Box::new(move || {
                        for _ in owner {
                            refresh(&conn, &shared, &ctx, &turn);
                        }
                    }));
                }
            }
            if let Ok(proxy) = WatcherProxy::builder(&conn).cache_properties(zbus::proxy::CacheProperties::No).build() {
                if let Ok(registered) = proxy.receive_status_notifier_host_registered() {
                    let (conn, shared, ctx, turn) = (conn.clone(), shared.clone(), ctx.clone(), turn.clone());
                    sources.push(Box::new(move || {
                        for _ in registered {
                            refresh(&conn, &shared, &ctx, &turn);
                        }
                    }));
                }
                if let Ok(unregistered) = proxy.receive_status_notifier_host_unregistered() {
                    let (conn, shared, ctx, turn) = (conn.clone(), shared.clone(), ctx.clone(), turn.clone());
                    sources.push(Box::new(move || {
                        for _ in unregistered {
                            refresh(&conn, &shared, &ctx, &turn);
                        }
                    }));
                }
            }
            for source in sources {
                let _ = std::thread::Builder::new().name("tray-presence-signal".into()).spawn(source);
            }
            refresh(&conn, &shared, &ctx, &turn);
        });
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn open(_ctx: eframe::egui::Context, _queue: Queue, _availability: Shared) -> Result<Box<dyn Port>, String> {
    Err("This system has no supported status icon".into())
}

/// A tray drawn into memory, for tests of the code that drives it.
#[cfg(test)]
pub mod testing {
    use super::*;
    pub struct Memory(pub Arc<Mutex<Vec<View>>>);
    impl Port for Memory {
        fn draw(&mut self, view: &View) {
            self.0.lock().unwrap().push(view.clone());
        }
    }
    pub fn tray() -> (Tray, Arc<Mutex<Vec<View>>>) {
        let drawn = Arc::new(Mutex::new(Vec::new()));
        (Tray::with_port(Box::new(Memory(drawn.clone()))), drawn)
    }
    /// What a person choosing from the menu would queue.
    pub fn choose(tray: &Tray, action: Action) {
        tray.queue.push(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording(secs: u64) -> TrayState {
        TrayState { phase: Phase::Recording(secs), ..TrayState::idle() }
    }
    fn labels(items: &[Item]) -> Vec<String> {
        fn walk(items: &[Item], out: &mut Vec<String>) {
            for i in items {
                out.push(i.label.clone());
                if let Kind::Submenu(inner) = &i.kind {
                    walk(inner, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(items, &mut out);
        out
    }
    fn has_action(items: &[Item], wanted: &Action) -> bool {
        items.iter().any(|i| match &i.kind {
            Kind::Action(a) => a == wanted,
            Kind::Submenu(inner) => has_action(inner, wanted),
            _ => false,
        })
    }

    #[test]
    fn the_clock_has_no_leading_zero_minutes_and_gains_hours_after_one() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(9), "0:09");
        assert_eq!(clock(42), "0:42");
        assert_eq!(clock(600), "10:00");
        assert_eq!(clock(3599), "59:59");
        assert_eq!(clock(3725), "1:02:05");
    }

    #[test]
    fn there_is_one_shape_and_the_timer_is_separate_text_only_while_recording() {
        for (phase, icon, title) in [
            (Phase::Idle, IconKind::Disc, ""),
            (Phase::Countdown(3), IconKind::Disc, "3"),
            (Phase::Recording(42), IconKind::Square, "0:42"),
            (Phase::Stopping, IconKind::Disc, "Saving…"),
        ] {
            let v = view(&TrayState { phase, ..TrayState::idle() });
            assert_eq!((v.icon, v.title.as_str()), (icon, title), "{phase:?}");
            for glyph in ['■', '●', '◼', '⏺', '⏹'] {
                assert!(!v.title.contains(glyph) && !v.tooltip.contains(glyph), "no shape character beside the icon");
            }
        }
        assert_eq!(view(&recording(3725)).title, "1:02:05");
        assert_eq!(view(&recording(42)).tooltip, "OpenRecorder — Recording 0:42");
        assert_eq!(view(&recording(42)).spoken, "OpenRecorder, recording, 42 seconds");
        assert_eq!(view(&TrayState::idle()).spoken, "OpenRecorder, idle");
    }

    #[test]
    fn leaving_recording_removes_every_trace_of_the_take_in_the_same_update() {
        let during = view(&recording(46));
        assert!(labels(&during.items).iter().any(|l| l == "Stop Recording (0:46)"));
        assert_eq!(during.icon, IconKind::Square);
        // The same state with only the phase changed: stop, save, failure, cancel all end here.
        let after = view(&TrayState::idle());
        let text = format!("{:?} {} {}", labels(&after.items), after.title, after.tooltip);
        for stale in ["0:46", "46", "Stop Recording", "Recording", "Saving", "Stopped", "Recording Stopped"] {
            assert!(!text.contains(stale), "{stale} survived: {text}");
        }
        assert_eq!((after.icon, after.title.as_str()), (IconKind::Disc, ""));
        assert_eq!(labels(&after.items)[0], "Record");
        assert!(has_action(&after.items, &Action::Record) && !has_action(&after.items, &Action::Stop));
        // Stopping is already the disc, with no timer and nothing to press.
        let saving = view(&TrayState { phase: Phase::Stopping, ..TrayState::idle() });
        assert_eq!(saving.icon, IconKind::Disc);
        assert!(!has_action(&saving.items, &Action::Stop) && !has_action(&saving.items, &Action::Record));
        assert_eq!(saving.items[0].label, "Saving…");
        assert!(!saving.items[0].enabled);
    }

    #[test]
    fn phases_lock_exactly_what_the_contract_says() {
        let find = |items: &[Item], prefix: &str| items.iter().find(|i| i.label.starts_with(prefix)).cloned().unwrap();
        let idle = menu(&TrayState::idle());
        for name in ["Source", "Camera", "Microphone"] {
            assert!(find(&idle, name).enabled, "{name}");
        }
        let counting = menu(&TrayState { phase: Phase::Countdown(2), ..TrayState::idle() });
        assert_eq!(counting[0].label, "Cancel Countdown (2)");
        assert!(has_action(&counting, &Action::CancelCountdown));
        for name in ["Source", "Camera", "Microphone"] {
            assert!(!find(&counting, name).enabled, "{name} during countdown");
        }
        assert_eq!(find(&counting, "Quit").label, "Quit OpenRecorder…");
        let rec = menu(&recording(5));
        assert!(!find(&rec, "Source").enabled && !find(&rec, "Microphone").enabled);
        let camera = find(&rec, "Camera");
        assert!(camera.enabled, "camera stays reachable while recording");
        let Kind::Submenu(inner) = &camera.kind else { panic!() };
        let by = |l: &str| inner.iter().find(|i| i.label == l).unwrap();
        assert!(!by("Turn On Camera").enabled && !by("No Camera").enabled, "device changes are locked");
        assert!(by("Circle").enabled && by("Wide 16:9").enabled && by("Portrait 9:16").enabled && by("Medium").enabled && by("Mirror Preview").enabled, "shape, size and mirror are live");
        assert!(inner.iter().any(|i| i.label == "Unavailable while recording"));
        assert_eq!(find(&rec, "Quit").label, "Quit OpenRecorder…");
    }

    #[test]
    fn record_names_only_the_inputs_it_will_start() {
        let mut s = TrayState::idle();
        assert!(!labels(&menu(&s)).iter().any(|l| l.contains("Turns on")));
        s.camera.mark = Mark::Armed;
        assert!(labels(&menu(&s)).contains(&"Turns on Camera first".to_string()));
        s.microphone.mark = Mark::Armed;
        assert!(labels(&menu(&s)).contains(&"Turns on Camera and Microphone first".to_string()));
        s.camera.mark = Mark::On; // already open: Record has nothing to turn on for it
        assert!(labels(&menu(&s)).contains(&"Turns on Microphone first".to_string()));
        assert_eq!(starts_label(false, false), None);
    }

    #[test]
    fn choosing_a_device_shape_or_size_only_asks_for_that_and_never_for_an_input() {
        let mut s = TrayState::idle();
        s.camera.devices = vec![Device { uid: "A".into(), label: "FaceTime HD Camera".into(), selected: true }];
        s.shape = "portrait".into();
        let m = menu(&s);
        assert!(has_action(&m, &Action::CameraDevice(Some("A".into()))));
        assert!(has_action(&m, &Action::Shape("portrait")) && has_action(&m, &Action::Shape("wide")) && has_action(&m, &Action::Shape("circle")));
        let cam = m.iter().find(|i| i.label.starts_with("Camera")).unwrap();
        let Kind::Submenu(inner) = &cam.kind else { panic!() };
        assert_eq!(inner.iter().find(|i| i.label == "Portrait 9:16").unwrap().checked, Some(true));
        assert_eq!(inner.iter().find(|i| i.label == "Circle").unwrap().checked, Some(false));
        assert_eq!(inner.iter().find(|i| i.label == "FaceTime HD Camera").unwrap().checked, Some(true));
        assert_eq!(inner.iter().find(|i| i.label == "No Camera").unwrap().checked, Some(false));
    }

    #[test]
    fn existing_square_and_older_sizes_stay_selectable_and_shown_as_chosen() {
        let mut s = TrayState::idle();
        s.shape = "square".into();
        s.size = 200; // the older medium
        let cam = menu(&s).into_iter().find(|i| i.label.starts_with("Camera")).unwrap();
        let Kind::Submenu(inner) = cam.kind else { panic!() };
        let medium = inner.iter().find(|i| i.label == "Medium").unwrap();
        assert_eq!(medium.checked, Some(true), "an older stored size reads as its current equivalent");
        assert_eq!(inner.iter().find(|i| i.label == "Small").unwrap().checked, Some(false));
        // A stored square has no entry that would silently rewrite it, and nothing else is checked.
        assert!(!SHAPES.iter().any(|(_, v)| *v == "square"));
        assert!(inner.iter().filter(|i| i.checked == Some(true)).all(|i| ["Medium", "Mirror Preview", "No Camera", "Show Camera"].contains(&i.label.as_str())));
    }

    #[test]
    fn a_stopped_take_leaves_one_calm_notice_with_one_next_step_and_a_saved_one_leaves_recent() {
        let mut s = TrayState::idle();
        s.notice = Some(Notice { text: "Couldn't keep up at 60 fps, 2560 × 1440. Originals are kept.".into(), kept: true });
        s.recent = vec![("t1".into(), "Today 10:58 · 0:46".into())];
        let m = menu(&s);
        let l = labels(&m);
        assert!(l.contains(&"Recording stopped early".into()) && l.contains(&"Available files were kept.".into()) && l.contains(&"Open Last Recording".into()));
        assert!(has_action(&m, &Action::OpenLastRecording));
        assert!(!l.iter().any(|x| x == "Use 30 fps" || x == "Dismiss" || x == "Show in Home" || x.contains("keep up")), "no frame-rate advice, no raw reason: {l:?}");
        assert!(!l.contains(&"Recent".into()), "a notice takes the place of Recent");
        // Nothing was kept: the plain sentence only, and nothing to open.
        s.notice = Some(Notice { text: "Couldn't start recording. Nothing was recorded.".into(), kept: false });
        let m = menu(&s);
        assert!(labels(&m).contains(&"Couldn't start recording. Nothing was recorded.".into()) && !has_action(&m, &Action::OpenLastRecording));
        s.notice = None;
        let m = menu(&s);
        assert!(labels(&m).contains(&"Recent".into()) && has_action(&m, &Action::OpenRecent("t1".into())));
        assert!(!labels(&m).iter().any(|l| l.contains("Saved") || l.contains("Stopped")), "no saved status line");
        // Neither is ever shown during a take.
        let mut busy = recording(7);
        busy.notice = Some(Notice { text: "x".into(), kept: true });
        busy.recent = s.recent.clone();
        assert!(!labels(&menu(&busy)).iter().any(|l| l == "Recent" || l == "Recording stopped early"));
    }
    #[test]
    fn devices_are_named_with_one_status_vocabulary_and_show_camera_is_its_own_choice() {
        let mut s = TrayState::idle();
        s.camera = Input { mark: Mark::Armed, devices: Vec::new(), name: Some("FaceTime HD Camera".into()) };
        s.microphone = Input { mark: Mark::On, devices: Vec::new(), name: Some("MacBook Pro Microphone".into()) };
        let m = menu(&s);
        let l = labels(&m);
        assert!(l.contains(&"Camera: FaceTime HD Camera · On".into()) && l.contains(&"Microphone: MacBook Pro Microphone · Live".into()), "{l:?}");
        assert!(!l.iter().any(|x| x.contains("Remembered")), "armed is On, never Remembered: {l:?}");
        for (mark, word) in [(Mark::Off, "Off"), (Mark::Starting, "Starting…"), (Mark::NotConnected, "Not connected"), (Mark::AccessOff, "Access off")] {
            s.camera = Input { mark, devices: Vec::new(), name: Some("Studio Camera".into()) };
            let want = if mark == Mark::AccessOff { "Camera: Access off".to_owned() } else { format!("Camera: Studio Camera · {word}") };
            assert!(labels(&menu(&s)).contains(&want), "{word}");
        }
        s.camera = Input { mark: Mark::Off, devices: Vec::new(), name: None };
        assert!(labels(&menu(&s)).contains(&"Camera: Off".into()), "no name known, no name shown");
        // Show Camera sits in the camera menu, live while recording, and turning the camera on or off does not touch it.
        let camera = |s: &TrayState| menu(s).into_iter().find(|i| i.label.starts_with("Camera")).unwrap();
        let Kind::Submenu(items) = camera(&s).kind else { panic!("camera submenu") };
        let show = items.iter().find(|i| i.label == "Show Camera").unwrap();
        assert_eq!((show.checked, show.enabled, &show.kind), (Some(true), true, &Kind::Action(Action::ShowCamera(false))));
        s.show_camera = false;
        s.phase = Phase::Recording(5);
        let Kind::Submenu(items) = camera(&s).kind else { panic!("camera submenu") };
        let show = items.iter().find(|i| i.label == "Show Camera").unwrap();
        assert_eq!((show.checked, show.enabled, &show.kind), (Some(false), true, &Kind::Action(Action::ShowCamera(true))));
        // The four forms, in the design's order.
        assert_eq!(SHAPES.map(|(_, v)| v), ["circle", "rounded", "wide", "portrait"]);
    }

    #[test]
    fn icons_are_a_solid_disc_and_a_solid_square_never_both() {
        let filled = |p: &[u8]| p.chunks(4).filter(|px| px[3] == 255).count();
        let alpha = |p: &[u8], x: usize, y: usize| p[(y * 32 + x) * 4 + 3];
        let disc = disc_alpha(32);
        assert_eq!((alpha(&disc, 16, 16), alpha(&disc, 15, 15), alpha(&disc, 0, 0)), (255, 255, 0));
        assert!((250..400).contains(&filled(&disc)), "{}", filled(&disc));
        let square = square_alpha(32);
        assert_eq!((alpha(&square, 16, 16), alpha(&square, 0, 0)), (255, 0));
        // A disc has transparent corners inside its bounding box; a square fills them.
        let (lo, hi) = (7usize, 24usize);
        assert_eq!(alpha(&square, lo + 1, lo + 1), 255);
        assert_eq!(alpha(&disc, lo + 1, lo + 1), 0);
        assert!(filled(&square) > filled(&disc) / 2 && filled(&square) < 600, "{}", filled(&square));
        assert_ne!(disc, square);
        let _ = hi;
        assert_eq!(icon_alpha(IconKind::Disc, 32), disc);
        assert_eq!(icon_alpha(IconKind::Square, 32), square);
    }

    #[derive(Default)]
    struct Fake {
        drawn: Arc<Mutex<Vec<View>>>,
    }
    impl Port for Fake {
        fn draw(&mut self, view: &View) {
            self.drawn.lock().unwrap().push(view.clone());
        }
    }
    #[test]
    fn the_port_is_drawn_only_when_the_picture_changes_and_a_terminal_state_always_redraws() {
        let fake = Fake::default();
        let drawn = fake.drawn.clone();
        let mut tray = Tray::with_port(Box::new(fake));
        tray.show(&TrayState::idle());
        tray.show(&TrayState::idle());
        assert_eq!(drawn.lock().unwrap().len(), 1, "an unchanged state costs nothing");
        for secs in [0, 1, 2, 46] {
            tray.show(&recording(secs));
        }
        assert_eq!(drawn.lock().unwrap().len(), 5, "the timer moves each second");
        tray.show(&TrayState { phase: Phase::Stopping, ..TrayState::idle() });
        tray.show(&TrayState::idle());
        let last = drawn.lock().unwrap().last().unwrap().clone();
        assert_eq!((last.icon, last.title.as_str()), (IconKind::Disc, ""));
        assert!(!format!("{:?}", last).contains("0:46"));
        // The same recording seconds after a terminal state is a new take, drawn again from scratch.
        tray.show(&recording(46));
        assert_eq!(drawn.lock().unwrap().last().unwrap().title, "0:46");
        tray.show(&TrayState::idle());
        assert_eq!(drawn.lock().unwrap().last().unwrap().title, "");
    }

    #[test]
    fn menu_choices_queue_in_order_and_are_bounded() {
        let q = Queue::default();
        q.push(Action::Record);
        q.push(Action::Quit);
        assert_eq!(q.drain(), vec![Action::Record, Action::Quit]);
        assert!(q.drain().is_empty());
        for _ in 0..40 {
            q.push(Action::OpenHome);
        }
        assert_eq!(q.drain().len(), 16);
    }

    #[test]
    fn the_source_menu_lists_displays_and_choose_window_and_ends_with_the_capability_line() {
        let mut s = TrayState::idle();
        s.displays = vec![
            Display { id: 1, label: "Built-in Display".into(), selected: true },
            Display { id: 2, label: "LG UltraFine".into(), selected: false },
        ];
        s.source_label = "Built-in Display".into();
        s.capability_line = "The camera bubble can appear in a display recording. Recording a window keeps it out.".into();
        let source = menu(&s).into_iter().find(|i| i.label.starts_with("Source")).unwrap();
        assert_eq!(source.label, "Source: Built-in Display");
        let Kind::Submenu(inner) = source.kind else { panic!() };
        assert_eq!(inner[0].checked, Some(true));
        assert_eq!(inner[1].checked, Some(false));
        assert!(inner.iter().any(|i| i.label == "Choose Window…"));
        assert_eq!(inner.last().unwrap().label, s.capability_line);
        assert!(!inner.last().unwrap().enabled, "the capability line is text");
        s.window_selected = true;
        let Kind::Submenu(inner) = menu(&s).into_iter().find(|i| i.label.starts_with("Source")).unwrap().kind else { panic!() };
        assert_eq!((inner[0].checked, inner.iter().find(|i| i.label == "Choose Window…").unwrap().checked), (Some(false), Some(true)));
    }

    #[test]
    fn a_watcher_alone_is_not_a_tray_and_each_fact_that_can_change_is_tracked_separately() {
        let mut a = Availability::UNCONFIRMED;
        assert!(a.lost() && a.reason().unwrap().contains("hasn't been confirmed"), "nothing is claimed before it is read");
        // The watcher owns its name but no host is registered: not a tray (the gap a watcher-only check missed).
        a.read(Seen::Yes, Seen::No);
        assert!(a.lost());
        assert_eq!(a.reason(), Some("The desktop's tray service is running, but nothing is showing tray icons"));
        // A host registers: now it counts.
        a.read(Seen::Yes, Seen::Yes);
        assert!(!a.lost() && a.reason().is_none());
        // The host disappears while the watcher stays.
        a.read(Seen::Yes, Seen::No);
        assert!(a.lost(), "host loss with the watcher still there");
        a.read(Seen::Yes, Seen::Yes);
        // The watcher itself goes away: the host fact is forgotten with it, not carried over.
        a.read(Seen::No, Seen::Yes);
        assert!(a.lost() && a.host == Seen::Unknown && a.reason() == Some("The desktop's tray service isn't running"));
        // It comes back: until the host is read again nothing is claimed, even though the name is owned.
        a.read(Seen::Yes, Seen::Unknown);
        assert!(a.lost() && a.reason().unwrap().contains("hasn't been confirmed"));
        a.read(Seen::Yes, Seen::Yes);
        assert!(!a.lost());
        // A failed reading is not "fine".
        a.read(Seen::Unknown, Seen::Unknown);
        assert!(a.lost());
        // Our own item must be registered too: the service sets this when it (re)registers.
        a.read(Seen::Yes, Seen::Yes);
        a.registered = false;
        assert!(a.lost() && a.reason().unwrap().contains("isn't registered"));
        a.registered = true;
        assert!(!a.lost());
        assert!(!Availability::PRESENT.lost(), "macOS and Windows start present");
    }
    #[test]
    fn the_tray_reports_loss_and_its_reason_from_the_shared_availability() {
        let (tray, _) = testing::tray();
        assert!(!tray.host_lost() && tray.lost_reason().is_none());
        tray.availability().lock().unwrap().read(Seen::Yes, Seen::No);
        assert!(tray.host_lost());
        assert!(tray.lost_reason().unwrap().contains("nothing is showing"));
    }
    #[test]
    fn the_linux_monitor_is_event_driven_bounded_and_uses_no_polling_or_hand_written_protocol() {
        let source = include_str!("tray.rs").split("/// A tray drawn into memory, for tests").next().unwrap();
        let monitor = source.split("mod monitor {").nth(1).unwrap().split("#[cfg(not(any(").next().unwrap();
        // Three bus signals, one property, one bounded method timeout; no timer loop.
        for needle in ["receive_name_owner_changed_with_args", "receive_status_notifier_host_registered", "receive_status_notifier_host_unregistered", "method_timeout(Duration::from_secs(3))", "is_status_notifier_host_registered"] {
            assert!(monitor.contains(needle), "{needle}");
        }
        for polling in ["loop {", "sleep(", "interval", "while true"] {
            assert!(!monitor.contains(polling), "no polling: {polling}");
        }
        assert!(monitor.matches("#[zbus(signal)]").count() == 2 && monitor.matches("#[zbus(property)]").count() == 1, "the whole declared protocol surface");
        assert!(!source.contains("gtk"));
        // ksni's assume-available mode only skips its own first host check; the monitor is what answers it.
        assert!(source.contains("assume_sni_available(true)") && source.contains("monitor::start("));
    }
    #[test]
    fn linux_uses_ksni_without_gtk_and_the_platform_trays_stay_where_they_were() {
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("[target.'cfg(target_os = \"linux\")'.dependencies.ksni]") && manifest.contains("version = \"=0.3.7\""));
        assert!(!manifest.contains("gtk") && !manifest.contains("libappindicator"));
        assert!(manifest.contains("[target.'cfg(any(target_os = \"macos\", target_os = \"windows\"))'.dependencies.tray-icon]"));
        let source = include_str!("tray.rs").split("/// A tray drawn into memory, for tests").next().unwrap();
        assert!(!source.contains("gtk::") && !source.contains("gtk_main"));
    }
}
