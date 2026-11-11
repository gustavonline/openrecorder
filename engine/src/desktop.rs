use super::controls::Stage;
use super::permissions::{self, Grant, GrantSnapshot};
use eframe::egui;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicI8, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// Child viewports. Every window other than ROOT is named here, and every close
// goes through close_present_viewport: closing an absent viewport made egui
// manufacture one and spin at 100 % CPU.
// ---------------------------------------------------------------------------
const VP_SETTINGS: &str = "settings";
const VP_CAMERA: &str = "camera";
const VP_TOAST: &str = "notice";
const VP_COUNTDOWN: &str = "countdown";
const VP_SOURCE: &str = "source-panel";
const VP_DEVICE: &str = "device-panel";
const VP_SETUP: &str = "setup";
const VP_QUIT: &str = "quit-confirmation";
const CHILD_VIEWPORTS: [&str; 8] = [
    VP_SETTINGS,
    VP_CAMERA,
    VP_TOAST,
    VP_COUNTDOWN,
    VP_SOURCE,
    VP_DEVICE,
    VP_SETUP,
    VP_QUIT,
];
fn vp(name: &'static str) -> egui::ViewportId {
    debug_assert!(CHILD_VIEWPORTS.contains(&name), "unlisted viewport {name}");
    egui::ViewportId::from_hash_of(name)
}
/// Closes every child except the ones named in `keep`.
fn close_children_except(ctx: &egui::Context, keep: &[&str]) {
    for name in CHILD_VIEWPORTS {
        if !keep.contains(&name) {
            close_present_viewport(ctx, vp(name));
        }
    }
}

// ---------------------------------------------------------------------------
// Tokens (DESIGN §3). Light and Dark share every metric; only colour differs.
// ---------------------------------------------------------------------------
use egui::Color32;
const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
#[derive(Clone, Copy)]
struct Tokens {
    surface: Color32,
    sidebar: Color32,
    control: Color32,
    selected: Color32,
    separator: Color32,
    hairline: Color32,
    text: Color32,
    secondary: Color32,
    tertiary: Color32,
    accent: Color32,
    primary: Color32,
    on_primary: Color32,
    live: Color32,
    record: Color32,
    toolbar: Color32,
    toolbar_edge: Color32,
    toast: Color32,
}
fn tokens(dark: bool) -> Tokens {
    if dark {
        Tokens {
            surface: rgb(0x1E1E21),
            sidebar: rgb(0x1A1A1D),
            control: rgb(0x2A2A2E),
            selected: rgb(0x1F3556),
            separator: rgb(0x2F2F34),
            hairline: rgb(0x3A3A40),
            text: rgb(0xF2F2F4),
            secondary: rgb(0xA1A1A8),
            tertiary: rgb(0x6E6E76),
            accent: rgb(0x4C9AFF),
            primary: rgb(0xF2F2F4),
            on_primary: rgb(0x111113),
            live: rgb(0x3DD68C),
            record: rgb(0xE5484D),
            toolbar: rgb(0x242427),
            toolbar_edge: rgb(0x3A3A40),
            toast: rgb(0x3A3A3E),
        }
    } else {
        Tokens {
            surface: rgb(0xFFFFFF),
            sidebar: rgb(0xF7F7F9),
            control: rgb(0xF0F0F2),
            selected: rgb(0xE5EEFD),
            separator: rgb(0xE5E5EA),
            hairline: rgb(0xD8D8DE),
            text: rgb(0x1D1D1F),
            secondary: rgb(0x6E6E73),
            tertiary: rgb(0x9A9AA0),
            accent: rgb(0x0A6CDF),
            primary: rgb(0x1D1D1F),
            on_primary: rgb(0xFFFFFF),
            live: rgb(0x1F8A4C),
            record: rgb(0xE0393E),
            toolbar: rgb(0xFBFBFD),
            toolbar_edge: rgb(0xD5D5DB),
            toast: rgb(0x2A2A2D),
        }
    }
}
fn tokens_for(ctx: &egui::Context) -> Tokens {
    tokens(ctx.style().visuals.dark_mode)
}
fn shade(color: Color32, factor: f32) -> Color32 {
    let scale = |c: u8| (c as f32 * factor).clamp(0., 255.) as u8;
    Color32::from_rgba_premultiplied(scale(color.r()), scale(color.g()), scale(color.b()), color.a())
}

const SETTINGS_PANES: [(&str, Icon); 6] = [
    ("General", Icon::Sliders),
    ("Recording", Icon::Record),
    ("Camera & Microphone", Icon::Camera),
    ("Sharing", Icon::Share),
    ("Permissions", Icon::Shield),
    ("About", Icon::Info),
];

/// egui stores independent light/dark Styles. Install shared metrics in both
/// before selecting a theme so ROOT and every child inherit identical geometry.
fn configure_native_styles(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        // Keep the approved body/control metrics without flattening headings
        // and small text through override_font_id.
        style.override_font_id = None;
        for (text_style, size) in [
            (egui::TextStyle::Body, 13.),
            (egui::TextStyle::Button, 13.),
            (egui::TextStyle::Small, 12.),
            (egui::TextStyle::Heading, 20.),
        ] {
            style
                .text_styles
                .insert(text_style, egui::FontId::proportional(size));
        }
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.));
        style.spacing.button_padding = egui::vec2(10., 9.);
        style.spacing.item_spacing = egui::vec2(9., 8.);
    });
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let dark = theme == egui::Theme::Dark;
        let t = tokens(dark);
        ctx.style_mut_of(theme, |style| {
            style.visuals = if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            style.visuals.override_text_color = Some(t.text);
            style.visuals.panel_fill = t.surface;
            style.visuals.window_fill = t.surface;
            style.visuals.extreme_bg_color = t.control;
            style.visuals.hyperlink_color = t.accent;
            style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1., t.separator);
            style.visuals.widgets.inactive.bg_fill = t.control;
            style.visuals.widgets.inactive.weak_bg_fill = t.control;
            style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            style.visuals.widgets.hovered.weak_bg_fill = shade(t.control, if dark { 1.18 } else { 0.96 });
            style.visuals.selection.bg_fill = t.selected;
            style.visuals.selection.stroke = egui::Stroke::new(1., t.accent);
        });
    }
}

// ---------------------------------------------------------------------------
// Fixed toolbar geometry (DESIGN §4): 427 × 44 pt, exact slots.
// ---------------------------------------------------------------------------
pub(crate) const TOOLBAR_W: f32 = 427.;
pub(crate) const TOOLBAR_H: f32 = 44.;
const SLOT_Y: f32 = 8.;
const SLOT_H: f32 = 28.;
const PANEL_GAP: f32 = 8.;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Grip,
    Library,
    Source,
    Camera,
    Mic,
    Divider,
    Record,
    Settings,
    Hide,
}
const SLOTS: [(Slot, f32, f32); 9] = [
    (Slot::Grip, 6., 10.),
    (Slot::Library, 18., 28.),
    (Slot::Source, 50., 148.),
    (Slot::Camera, 202., 28.),
    (Slot::Mic, 234., 28.),
    (Slot::Divider, 270., 1.),
    (Slot::Record, 279., 76.),
    (Slot::Settings, 359., 28.),
    (Slot::Hide, 391., 28.),
];
fn slot_rect(origin: egui::Pos2, slot: Slot) -> egui::Rect {
    let (_, x, w) = SLOTS.iter().copied().find(|(s, ..)| *s == slot).unwrap();
    egui::Rect::from_min_size(origin + egui::vec2(x, SLOT_Y), egui::vec2(w, SLOT_H))
}
/// Rectangles that own a press. Everything else on the toolbar drags the window.
fn control_rects(origin: egui::Pos2) -> Vec<egui::Rect> {
    SLOTS
        .iter()
        .filter(|(slot, ..)| !matches!(slot, Slot::Grip | Slot::Divider))
        .map(|(slot, ..)| slot_rect(origin, *slot))
        .collect()
}
pub(crate) fn default_toolbar_position(monitor: egui::Vec2) -> egui::Pos2 {
    egui::pos2(((monitor.x - TOOLBAR_W) / 2.).max(0.), (monitor.y - 68.).max(0.))
}
/// Places a panel 8 pt above the slot (below it near the top edge) so it never
/// overlaps the toolbar, clamped to the *monitor rectangle in global
/// coordinates* (origin included). With no known monitor nothing is clamped:
/// the panel is placed relative to the toolbar only and the OS keeps it on screen.
fn anchor_panel(
    toolbar: egui::Rect,
    slot: egui::Rect,
    panel: egui::Vec2,
    monitor: Option<egui::Rect>,
    prefer_below: bool,
) -> egui::Pos2 {
    let above = toolbar.min.y - PANEL_GAP - panel.y;
    let below = toolbar.max.y + PANEL_GAP;
    let Some(monitor) = monitor else {
        return egui::pos2(slot.min.x, if prefer_below { below } else { above });
    };
    let fits_above = above >= monitor.min.y;
    let fits_below = below + panel.y <= monitor.max.y;
    let y = match (prefer_below, fits_above, fits_below) {
        (true, _, true) => below,
        (_, true, _) => above,
        (_, false, true) => below,
        // Neither side has room: take the side with more space.
        _ if toolbar.min.y - monitor.min.y >= monitor.max.y - toolbar.max.y => above.max(monitor.min.y),
        _ => below.min((monitor.max.y - panel.y).max(monitor.min.y)),
    };
    let min_x = monitor.min.x + PANEL_GAP;
    let max_x = (monitor.max.x - panel.x - PANEL_GAP).max(min_x);
    egui::pos2(slot.min.x.clamp(min_x, max_x), y)
}

// ---------------------------------------------------------------------------
// Global coordinates. egui reports window rectangles in global logical points
// (origin = the primary display's top-left, secondary displays may sit at
// negative or large offsets) but a monitor's *size* only. Every conversion
// therefore carries the monitor or capture rectangle including its origin.
// ---------------------------------------------------------------------------
/// What the recording shows and where each monitor sits, in global points.
#[derive(Clone, Debug, Default, PartialEq)]
struct BubbleSpace {
    monitors: Vec<egui::Rect>,
    /// The selected display's rectangle. None for a window, or when unknown.
    capture: Option<egui::Rect>,
}
/// The monitor holding most of `rect`. Without a monitor list (platforms that
/// don't report one) the reported size is trusted only when the rectangle lies
/// wholly inside [0, size]; anything else is unknown rather than guessed.
fn monitor_for(rect: egui::Rect, monitors: &[egui::Rect], reported: Option<egui::Vec2>) -> Option<egui::Rect> {
    if let Some(best) = monitors
        .iter()
        .filter(|m| m.intersects(rect))
        .max_by(|a, b| a.intersect(rect).area().total_cmp(&b.intersect(rect).area()))
    {
        return Some(*best);
    }
    if !monitors.is_empty() {
        return None;
    }
    let assumed = egui::Rect::from_min_size(egui::Pos2::ZERO, reported?);
    assumed.contains_rect(rect).then_some(assumed)
}
fn layout_region(bubble: egui::Rect, space: &BubbleSpace, reported: Option<egui::Vec2>) -> Option<egui::Rect> {
    space.capture.or_else(|| monitor_for(bubble, &space.monitors, reported))
}
/// `Layout` fractions (x, y, base size) for a bubble: relative to the region
/// the recording shows, origin included. The size has no floor beyond sanity
/// bounds, so a 120 pt bubble stays 120 pt on a 2560 pt display.
fn bubble_layout(bubble: egui::Rect, shape: u8, region: egui::Rect) -> Option<(f32, f32, f32)> {
    if region.width() <= 0. || region.height() <= 0. {
        return None;
    }
    let x = (bubble.min.x - region.min.x) / region.width();
    let y = (bubble.min.y - region.min.y) / region.height();
    let size = bubble_base(bubble.width(), shape) / region.width();
    Some((x.clamp(0., 1.), y.clamp(0., 1.), size.clamp(0.01, 0.5)))
}
/// Active displays as (CGDirectDisplayID, global rectangle in points) and the
/// main display id. Passive Quartz calls: no permission, prompt or capture.
#[cfg(target_os = "macos")]
fn system_displays() -> (Vec<(u32, egui::Rect)>, Option<u32>) {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point { x: f64, y: f64 }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size { width: f64, height: f64 }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect { origin: Point, size: Size }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGMainDisplayID() -> u32;
    }
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    // SAFETY: the buffer holds 16 ids and CoreGraphics writes at most `max`.
    let status = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
    if status != 0 {
        return (Vec::new(), None);
    }
    let displays = ids[..count.min(16) as usize]
        .iter()
        .map(|id| {
            // SAFETY: plain value query for an id CoreGraphics just returned.
            let r = unsafe { CGDisplayBounds(*id) };
            (*id, egui::Rect::from_min_size(egui::pos2(r.origin.x as f32, r.origin.y as f32), egui::vec2(r.size.width as f32, r.size.height as f32)))
        })
        .collect();
    // SAFETY: no arguments.
    (displays, Some(unsafe { CGMainDisplayID() }))
}
/// No platform API here reports display origins: positions stay unknown.
#[cfg(not(target_os = "macos"))]
fn system_displays() -> (Vec<(u32, egui::Rect)>, Option<u32>) {
    (Vec::new(), None)
}
/// Global rectangle, in points, of ONE selected window (a CGWindowID the person
/// picked). Passive Quartz query: bounds only, never a title, never another
/// window, no capture, no permission, no prompt. None when the window is gone.
#[cfg(target_os = "macos")]
fn system_window_bounds(window: u32) -> Option<egui::Rect> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point { x: f64, y: f64 }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Size { width: f64, height: f64 }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect { origin: Point, size: Size }
    #[link(name = "CoreGraphics", kind = "framework")]
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCGWindowBounds: *const c_void;
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
        fn CGRectMakeWithDictionaryRepresentation(dictionary: *const c_void, rect: *mut CGRect) -> bool;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
        fn CFRelease(object: *const c_void);
    }
    // kCGWindowListOptionIncludingWindow: the list holds just the window named below.
    const INCLUDING_WINDOW: u32 = 1 << 3;
    let mut rect = CGRect::default();
    // SAFETY: every pointer comes from CoreGraphics/CoreFoundation in this call, is checked for null
    // before use, and the returned (copied) array is released exactly once on every path.
    let found = unsafe {
        let list = CGWindowListCopyWindowInfo(INCLUDING_WINDOW, window);
        if list.is_null() {
            return None;
        }
        // Exactly one entry or nothing: anything else is not "this window".
        let ok = CFArrayGetCount(list) == 1 && {
            let info = CFArrayGetValueAtIndex(list, 0);
            let bounds = if info.is_null() { std::ptr::null() } else { CFDictionaryGetValue(info, kCGWindowBounds) };
            !bounds.is_null() && CGRectMakeWithDictionaryRepresentation(bounds, &mut rect)
        };
        CFRelease(list);
        ok
    };
    let (x, y, w, h) = (rect.origin.x, rect.origin.y, rect.size.width, rect.size.height);
    (found && [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0. && h > 0.)
        .then(|| egui::Rect::from_min_size(egui::pos2(x as f32, y as f32), egui::vec2(w as f32, h as f32)))
}
/// No other platform reports a window's global rectangle through an API used here:
/// Windows would need per-window DPI conversion that is not qualified, and a Linux
/// portal deliberately hides the chosen window. Unknown stays unknown.
#[cfg(not(target_os = "macos"))]
fn system_window_bounds(_window: u32) -> Option<egui::Rect> {
    None
}
/// Region the recording shows. A selected window is only known where `window` is.
fn space_for(
    displays: &[(u32, egui::Rect)],
    main: Option<u32>,
    selection: Option<(bool, u32)>,
    window: Option<egui::Rect>,
) -> BubbleSpace {
    let capture = match selection {
        Some((false, id)) => displays.iter().find(|(d, _)| *d == id).map(|(_, r)| *r),
        Some((true, _)) => window,
        None => main.and_then(|id| displays.iter().find(|(d, _)| *d == id).map(|(_, r)| *r)),
    };
    BubbleSpace { monitors: displays.iter().map(|(_, r)| *r).collect(), capture }
}

// ---------------------------------------------------------------------------
// Camera bubble geometry (DESIGN §8). The shape table lives in pipeline.rs, so
// the preview window and the composite cannot disagree; only rounding to whole
// points happens here.
// ---------------------------------------------------------------------------
const SHAPE_CIRCLE: u8 = 0;
const SHAPE_ROUNDED: u8 = 1;
const SHAPE_SQUARE: u8 = 2;
const SHAPE_WIDE: u8 = 3;
const SHAPE_PORTRAIT: u8 = 4;
const SIZES: [(u32, &str); 3] = [(120, "S"), (168, "M"), (224, "L")];
const LEGACY_SIZES: [u32; 3] = [140, 200, 280];
pub(crate) fn bubble_dimensions(base: f32, shape: u8) -> (f32, f32) {
    let (width, height) = super::pipeline::camera_dimensions(base, shape);
    (width.round(), height.round())
}
/// Inverse of the width: what `Layout::size` stores is the base, not the width.
pub(crate) fn bubble_base(width: f32, shape: u8) -> f32 {
    match shape {
        SHAPE_WIDE => width * 3. / 4.,
        SHAPE_PORTRAIT => width * 4. / 3.,
        _ => width,
    }
}
fn bubble_radius(shape: u8, width: f32, height: f32) -> f32 {
    match shape {
        SHAPE_CIRCLE => width.min(height) / 2.,
        SHAPE_ROUNDED => width.min(height) * 0.22,
        SHAPE_WIDE | SHAPE_PORTRAIT => width.min(height) * 0.20,
        _ => 0.,
    }
}
/// Size S/M/L for any stored value. Legacy 140/200/280 stay what they are until
/// the person picks one, but read as S/M/L in the UI.
fn size_class(size: u32) -> usize {
    match size {
        120 | 140 => 0,
        168 | 200 => 1,
        224 | 280 => 2,
        _ => 1,
    }
}
fn valid_size(size: u64) -> bool {
    SIZES.iter().any(|(s, _)| *s as u64 == size) || LEGACY_SIZES.contains(&(size as u32))
}
/// Source rectangle (x, y, w, h) that fills the destination aspect with a
/// centred, proportional crop: never stretched, never letterboxed.
fn cover_rect(sw: usize, sh: usize, dw: usize, dh: usize) -> (usize, usize, usize, usize) {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return (0, 0, 0, 0);
    }
    if sw * dh > sh * dw {
        let w = (sh * dw / dh).clamp(1, sw);
        ((sw - w) / 2, 0, w, sh)
    } else {
        let h = (sw * dh / dw).clamp(1, sh);
        (0, (sh - h) / 2, sw, h)
    }
}
fn preview_dimensions(shape: u8) -> (usize, usize) {
    match shape {
        SHAPE_WIDE => (480, 270),
        SHAPE_PORTRAIT => (270, 480),
        _ => (360, 360),
    }
}

// ---------------------------------------------------------------------------
// Record slot, input marks and contextual problems.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Face {
    Ready,
    Starting,
    Countdown(u64),
    Recording(u64),
    Saving,
    /// Capture is over and the take is on disk, but the library hasn't confirmed it; press retries.
    RetrySave,
    Saved,
    Unavailable,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Glyph {
    Dot,
    Square,
    Check,
    None,
}
impl Face {
    fn parts(self) -> (Glyph, String) {
        match self {
            Face::Ready => (Glyph::Dot, "Record".into()),
            Face::Starting => (Glyph::None, "Starting…".into()),
            Face::Countdown(n) => (Glyph::None, format!("{n}  Cancel")),
            Face::Recording(s) => (Glyph::Square, format_elapsed(s)),
            Face::Saving => (Glyph::None, "Saving…".into()),
            Face::RetrySave => (Glyph::None, "Retry save".into()),
            Face::Saved => (Glyph::Check, "Saved".into()),
            Face::Unavailable => (Glyph::None, "Record".into()),
        }
    }
    #[cfg(test)]
    fn label(self) -> String {
        let (glyph, text) = self.parts();
        match glyph {
            Glyph::Dot => format!("● {text}"),
            Glyph::Square => format!("■ {text}"),
            Glyph::Check => format!("✓ {text}"),
            Glyph::None => text,
        }
    }
    fn accessible_name(self) -> String {
        match self {
            Face::Ready => "Record".into(),
            Face::Starting => "Starting, press to cancel".into(),
            Face::Countdown(n) => format!("Countdown {n}, press to cancel"),
            Face::Recording(s) => format!("Stop recording, {}", format_elapsed(s)),
            Face::Saving => "Saving".into(),
            Face::RetrySave => "Retry saving the last recording".into(),
            Face::Saved => "Saved".into(),
            Face::Unavailable => "Record, unavailable".into(),
        }
    }
    /// Source, Camera and Mic lock for these faces.
    fn locks_inputs(self) -> bool {
        matches!(
            self,
            Face::Starting | Face::Countdown(_) | Face::Recording(_) | Face::Saving | Face::RetrySave
        )
    }
}
fn format_elapsed(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
fn format_free(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}
const MIN_FREE_BYTES: u64 = 2_000_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mark {
    None,
    Armed,
    Live,
    Attention,
}
/// Off shows nothing, remembered intent a hollow ring, only an actually open
/// source a green dot, and a refused or missing input a dark dot.
fn input_mark(live: bool, opening: bool, attention: bool, intent: bool) -> Mark {
    if live {
        Mark::Live
    } else if attention {
        Mark::Attention
    } else if opening || intent {
        Mark::Armed
    } else {
        Mark::None
    }
}
fn mark_word(mark: Mark) -> &'static str {
    match mark {
        Mark::None => "off",
        Mark::Armed => "armed, not opened",
        Mark::Live => "on",
        Mark::Attention => "needs attention",
    }
}

#[derive(Clone, Debug, PartialEq)]
enum InputProblem {
    AccessOff { camera: bool },
    Restricted { camera: bool },
    NeedsAccess { camera: bool },
    Missing { camera: bool, name: Option<String> },
    Duplicate { name: String },
    Failed { camera: bool, detail: String },
}
impl InputProblem {
    fn is_camera(&self) -> bool {
        match self {
            Self::AccessOff { camera }
            | Self::Restricted { camera }
            | Self::NeedsAccess { camera }
            | Self::Missing { camera, .. }
            | Self::Failed { camera, .. } => *camera,
            Self::Duplicate { .. } => false,
        }
    }
    fn message(&self) -> String {
        let noun = |camera: bool| if camera { "Camera" } else { "Mic" };
        match self {
            Self::AccessOff { camera } => format!("{} access is off", noun(*camera)),
            Self::Restricted { camera } => {
                format!("{} access is restricted on this computer", noun(*camera))
            }
            Self::NeedsAccess { camera } => {
                format!("{} access isn't allowed yet · Click {}", noun(*camera), noun(*camera))
            }
            Self::Missing { camera, name } => match name {
                Some(name) => format!("{name} isn't connected"),
                None => format!("Your {} isn't connected", noun(*camera).to_lowercase()),
            },
            Self::Duplicate { name } => format!("Several microphones are named “{name}”"),
            Self::Failed { camera, .. } => {
                format!("Couldn't start the {}", noun(*camera).to_lowercase())
            }
        }
    }
    fn action(&self) -> Option<ToastAction> {
        match self {
            Self::AccessOff { camera } => Some(if *camera {
                ToastAction::OpenCameraSettings
            } else {
                ToastAction::OpenMicSettings
            }),
            Self::Missing { camera, .. } | Self::Failed { camera, .. } => Some(if *camera {
                ToastAction::ChooseCamera
            } else {
                ToastAction::ChooseMicrophone
            }),
            Self::Duplicate { .. } => Some(ToastAction::ChooseMicrophone),
            Self::Restricted { .. } | Self::NeedsAccess { .. } => None,
        }
    }
    fn toast(&self, offer_turn_off: bool) -> Toast {
        let mut toast = Toast::error(self.message());
        toast.action = self.action();
        if let Self::Failed { detail, .. } = self {
            toast.details = Some(detail.clone());
        }
        if offer_turn_off {
            toast.secondary = Some(if self.is_camera() {
                ToastAction::TurnOffCamera
            } else {
                ToastAction::TurnOffMicrophone
            });
        }
        toast
    }
}
/// Only used to name what a missing device was; no identity is ever resolved by name.
#[derive(Clone, Debug, PartialEq)]
enum Presence {
    Unchecked,
    Listed(Vec<(String, String)>),
    ListFailed(String),
}
#[derive(Clone, Debug)]
struct ArmedInput {
    camera: bool,
    /// Armed and not already open.
    wants_open: bool,
    grant: Grant,
    remembered: Option<String>,
    remembered_name: Option<String>,
    presence: Presence,
}
/// Grants first, then devices. Returns the first reason Record cannot start the
/// armed inputs; there is no fallback device and no silent recording without them.
fn evaluate_armed(inputs: &[ArmedInput]) -> Result<(), InputProblem> {
    for input in inputs.iter().filter(|i| i.wants_open) {
        let camera = input.camera;
        match input.grant {
            Grant::Denied => return Err(InputProblem::AccessOff { camera }),
            Grant::Restricted => return Err(InputProblem::Restricted { camera }),
            Grant::NotAsked => return Err(InputProblem::NeedsAccess { camera }),
            Grant::Allowed | Grant::SystemManaged => {}
        }
    }
    for input in inputs.iter().filter(|i| i.wants_open) {
        let Some(id) = &input.remembered else { continue };
        match &input.presence {
            Presence::Unchecked => {}
            Presence::ListFailed(detail) => {
                return Err(InputProblem::Failed { camera: input.camera, detail: detail.clone() })
            }
            Presence::Listed(devices) => {
                let matches = devices.iter().filter(|(uid, _)| uid == id).count();
                if matches == 0 {
                    return Err(InputProblem::Missing {
                        camera: input.camera,
                        name: input.remembered_name.clone(),
                    });
                }
                if !input.camera && matches > 1 {
                    return Err(InputProblem::Duplicate { name: id.clone() });
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Toast pill (DESIGN §9): one at a time, info 4 s, errors 8 s, hover pauses.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ToastAction {
    OpenScreenSettings,
    OpenCameraSettings,
    OpenMicSettings,
    ChooseSource,
    ChooseCamera,
    ChooseMicrophone,
    ShowInLibrary,
    RetrySave,
    TurnOffCamera,
    TurnOffMicrophone,
}
impl ToastAction {
    fn label(self) -> &'static str {
        match self {
            Self::OpenScreenSettings | Self::OpenCameraSettings | Self::OpenMicSettings => {
                "Open Settings"
            }
            Self::ChooseSource | Self::ChooseCamera | Self::ChooseMicrophone => "Choose…",
            Self::ShowInLibrary => "Show in Library",
            Self::RetrySave => "Retry",
            Self::TurnOffCamera => "Turn Camera off",
            Self::TurnOffMicrophone => "Turn Mic off",
        }
    }
}
#[derive(Clone, Debug)]
struct Toast {
    text: String,
    action: Option<ToastAction>,
    secondary: Option<ToastAction>,
    details: Option<String>,
    error: bool,
    expires: Instant,
    last_tick: Instant,
    details_open: bool,
}
fn toast_lifetime(error: bool) -> Duration {
    Duration::from_secs(if error { 8 } else { 4 })
}
impl Toast {
    fn new(text: impl Into<String>, error: bool) -> Self {
        let now = Instant::now();
        Self {
            text: text.into(),
            action: None,
            secondary: None,
            details: None,
            error,
            expires: now + toast_lifetime(error),
            last_tick: now,
            details_open: false,
        }
    }
    fn info(text: impl Into<String>) -> Self {
        Self::new(text, false)
    }
    fn error(text: impl Into<String>) -> Self {
        Self::new(text, true)
    }
    fn with_action(mut self, action: ToastAction) -> Self {
        self.action = Some(action);
        self
    }
    fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
    /// Advances the expiry clock; time spent under the pointer does not count.
    fn tick(&mut self, now: Instant, hovered: bool) -> bool {
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        if hovered || self.details_open {
            self.expires += elapsed;
        }
        now < self.expires
    }
    fn pill_size(&self) -> egui::Vec2 {
        let extra = if self.details_open { 76. } else { 0. };
        egui::vec2(392., 40. + extra)
    }
}

// ---------------------------------------------------------------------------
// Icons, drawn at 16 pt with the painter. No font glyphs, so Light/Dark and
// every scale render the same shapes.
// ---------------------------------------------------------------------------
const GEAR_TEETH: usize = 8;
const GEAR_BODY_RADIUS: f32 = 5.2;
const GEAR_HOLE_RADIUS: f32 = 2.1;
const GEAR_BASE_RADIUS: f32 = 4.4;
const GEAR_TIP_RADIUS: f32 = 7.8;
/// Eight tapered teeth that overlap a solid body, plus a centre hole: a real
/// cog, never detached rays. Each tooth is a convex quad, base first.
fn gear_teeth(centre: egui::Pos2) -> [[egui::Pos2; 4]; GEAR_TEETH] {
    std::array::from_fn(|n| {
        let angle = n as f32 * std::f32::consts::TAU / GEAR_TEETH as f32;
        let dir = egui::vec2(angle.cos(), angle.sin());
        let side = egui::vec2(-dir.y, dir.x);
        [
            centre + dir * GEAR_BASE_RADIUS + side * 2.0,
            centre + dir * GEAR_TIP_RADIUS + side * 1.25,
            centre + dir * GEAR_TIP_RADIUS - side * 1.25,
            centre + dir * GEAR_BASE_RADIUS - side * 2.0,
        ]
    })
}
fn paint_cog(painter: &egui::Painter, centre: egui::Pos2, ink: Color32, hole: Color32) {
    for tooth in gear_teeth(centre) {
        painter.add(egui::Shape::convex_polygon(tooth.to_vec(), ink, egui::Stroke::NONE));
    }
    painter.circle_filled(centre, GEAR_BODY_RADIUS, ink);
    painter.circle_filled(centre, GEAR_HOLE_RADIUS, hole);
}
/// The brand: a plain black circle, 18 pt. Dark mode adds a 1 pt hairline.
fn library_mark(dark: bool) -> (f32, Color32, Option<egui::Stroke>) {
    (
        9.,
        Color32::BLACK,
        dark.then(|| egui::Stroke::new(1., rgb(0x4A4A50))),
    )
}
fn paint_library_mark(painter: &egui::Painter, centre: egui::Pos2, dark: bool) {
    let (radius, fill, hairline) = library_mark(dark);
    painter.circle_filled(centre, radius, fill);
    if let Some(stroke) = hairline {
        painter.circle_stroke(centre, radius + stroke.width / 2., stroke);
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Icon {
    Display,
    Window,
    Camera,
    Mic,
    Close,
    Chevron,
    Back,
    Check,
    Sliders,
    Record,
    Share,
    Shield,
    Info,
}
fn paint_icon(painter: &egui::Painter, icon: Icon, c: egui::Pos2, ink: Color32) {
    let stroke = egui::Stroke::new(1.4, ink);
    let p = |x: f32, y: f32| c + egui::vec2(x, y);
    let rect = |w: f32, h: f32| egui::Rect::from_center_size(c, egui::vec2(w, h));
    let line = |points: &[egui::Pos2]| {
        painter.add(egui::Shape::line(points.to_vec(), stroke));
    };
    match icon {
        Icon::Display => {
            painter.rect_stroke(rect(14., 9.5).translate(egui::vec2(0., -1.)), 2., stroke, egui::StrokeKind::Inside);
            line(&[p(-3., 6.), p(3., 6.)]);
        }
        Icon::Window => {
            painter.rect_stroke(rect(14., 11.), 2., stroke, egui::StrokeKind::Inside);
            line(&[p(-7., -2.), p(7., -2.)]);
        }
        Icon::Camera => {
            painter.rect_stroke(rect(10.5, 9.).translate(egui::vec2(-1.5, 0.)), 2., stroke, egui::StrokeKind::Inside);
            painter.add(egui::Shape::convex_polygon(
                vec![p(5.5, -1.5), p(8., -3.5), p(8., 3.5), p(5.5, 1.5)],
                ink,
                egui::Stroke::NONE,
            ));
        }
        Icon::Mic => {
            painter.rect_stroke(
                egui::Rect::from_center_size(p(0., -2.), egui::vec2(5., 8.5)),
                2.5,
                stroke,
                egui::StrokeKind::Inside,
            );
            let arc: Vec<egui::Pos2> = (0..=8)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 8.;
                    p(-5. * a.cos(), 1.5 + 3.5 * a.sin())
                })
                .collect();
            line(&arc);
            line(&[p(0., 5.), p(0., 7.5)]);
        }
        Icon::Close => {
            line(&[p(-4., -4.), p(4., 4.)]);
            line(&[p(-4., 4.), p(4., -4.)]);
        }
        Icon::Chevron => line(&[p(-3.5, -1.5), p(0., 2.), p(3.5, -1.5)]),
        Icon::Back => line(&[p(1.5, -4.), p(-2.5, 0.), p(1.5, 4.)]),
        Icon::Check => line(&[p(-4., 0.5), p(-1., 3.5), p(4.5, -3.)]),
        Icon::Sliders => {
            for (y, x) in [(-4.5, -2.), (0., 3.), (4.5, -1.)] {
                line(&[p(-6., y), p(6., y)]);
                painter.circle_filled(p(x, y), 2., ink);
            }
        }
        Icon::Record => {
            painter.circle_stroke(c, 6., stroke);
            painter.circle_filled(c, 2.5, ink);
        }
        Icon::Share => {
            painter.rect_stroke(rect(12., 9.).translate(egui::vec2(0., 2.)), 2., stroke, egui::StrokeKind::Inside);
            line(&[p(0., 3.), p(0., -7.)]);
            line(&[p(-3., -4.), p(0., -7.), p(3., -4.)]);
        }
        Icon::Shield => line(&[p(0., -7.), p(6., -4.5), p(5., 3.), p(0., 7.), p(-5., 3.), p(-6., -4.5), p(0., -7.)]),
        Icon::Info => {
            painter.circle_stroke(c, 6.5, stroke);
            painter.circle_filled(p(0., -2.8), 1., ink);
            line(&[p(0., -0.5), p(0., 3.5)]);
        }
    }
}
/// A text label that truncates with an ellipsis instead of growing its slot.
fn truncated_galley(
    painter: &egui::Painter,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    painter.layout_job(job)
}
struct Camera {
    opened: Instant,
    child: Option<Child>,
    pixels: Arc<Mutex<super::camera_frame::Frame>>,
    ready: Arc<AtomicBool>,
    last_frame: Option<Arc<std::sync::atomic::AtomicU64>>,
    error: Arc<Mutex<Option<String>>>,
}
impl Drop for Camera {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Camera {
    fn start(selected_uid: Option<&str>) -> Result<Self, String> {
        let ffmpeg = std::env::var("OPENRECORDER_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        let device = if selected_uid.is_some() {
            if cfg!(target_os = "macos") {
                "default".to_owned()
            } else {
                selected_uid.unwrap().to_owned()
            }
        } else {
            match std::env::var("OPENRECORDER_CAMERA") {
                Ok(value) => value,
                Err(_) => {
                    if cfg!(target_os = "linux") {
                        "/dev/video0".into()
                    } else if cfg!(target_os = "macos") {
                        "default".into()
                    } else {
                        let log = std::env::temp_dir()
                            .join(format!("openrecorder-devices-{}.log", std::process::id()));
                        let file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
                        let mut probe = Command::new(
                            std::env::var("OPENRECORDER_FFMPEG")
                                .unwrap_or_else(|_| "ffmpeg".into()),
                        )
                        .args([
                            "-hide_banner",
                            "-list_devices",
                            "true",
                            "-f",
                            "dshow",
                            "-i",
                            "dummy",
                        ])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(file)
                        .spawn()
                        .map_err(|e| e.to_string())?;
                        let deadline = Instant::now() + Duration::from_secs(3);
                        while probe.try_wait().map_err(|e| e.to_string())?.is_none() {
                            if Instant::now() > deadline {
                                let _ = probe.kill();
                                let _ = probe.wait();
                                break;
                            }
                            thread::sleep(Duration::from_millis(20));
                        }
                        let text = std::fs::read_to_string(&log).unwrap_or_default();
                        let _ = std::fs::remove_file(log);
                        text.lines()
                            .find(|line| line.contains("(video)"))
                            .and_then(|line| line.split('"').nth(1))
                            .ok_or("No camera was found")?
                            .to_string()
                    }
                }
            }
        };
        if cfg!(target_os = "macos") {
            if let Some(uid) = selected_uid {
                cmd.args(["-video_device_uid", uid]);
            }
            cmd.args([
                "-f",
                "avfoundation",
                "-framerate",
                "30",
                "-i",
                &format!("{device}:none"),
            ]);
        } else if cfg!(target_os = "linux") {
            cmd.args([
                "-f",
                "v4l2",
                "-framerate",
                "30",
                "-video_size",
                "1280x720",
                "-input_format",
                "mjpeg",
                "-i",
                &device,
            ]);
        } else {
            cmd.args(["-f", "dshow", "-i", &format!("video={device}")]);
        }
        super::event_log::event(
            "camera_process_start",
            serde_json::json!({"stableSelection":selected_uid.is_some()}),
        );
        let mut child = cmd
            .args([
                "-an",
                "-vf",
                "scale=1280:720:force_original_aspect_ratio=decrease:force_divisible_by=2",
                "-fps_mode",
                "passthrough",
                "-pix_fmt",
                "rgba",
                "-c:v",
                "pam",
                "-f",
                "image2pipe",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut stdout = std::io::BufReader::new(child.stdout.take().ok_or("Camera pipe failed")?);
        let pixels = Arc::new(Mutex::new(super::camera_frame::Frame {
            width: 0,
            height: 0,
            bgra: vec![],
            sequence: 0,
        }));
        let shared = pixels.clone();
        let ready = Arc::new(AtomicBool::new(false));
        let flag = ready.clone();
        let error = Arc::new(Mutex::new(None));
        let disconnected = error.clone();
        let opened = Instant::now();
        let last_frame = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let received = last_frame.clone();
        thread::spawn(move || {
            while let Ok(mut frame) = super::camera_frame::read_pam(&mut stdout) {
                if let Ok(mut p) = shared.lock() {
                    if p.sequence == 0 {
                        super::event_log::event(
                            "camera_first_frame",
                            serde_json::json!({"width":frame.width,"height":frame.height}),
                        );
                    }
                    frame.sequence = p.sequence.saturating_add(1);
                    *p = frame;
                    received.store(opened.elapsed().as_millis() as u64, Ordering::Relaxed);
                    flag.store(true, Ordering::Relaxed);
                } else {
                    break;
                }
            }
            if let Ok(mut value) = disconnected.lock() {
                *value = Some("Camera disconnected. Partial files are preserved.".into());
            }
        });
        Ok(Self {
            opened,
            last_frame: Some(last_frame),
            child: Some(child),
            pixels,
            ready,
            error,
        })
    }
}

// ---------------------------------------------------------------------------
// Seams. Everything that can prompt, open hardware or enumerate devices is a
// function pointer, so unit checks substitute inert fakes and never touch an OS.
// ---------------------------------------------------------------------------
enum MicSource {
    Real(super::audio::Microphone),
    #[cfg(test)]
    Fake,
}
impl MicSource {
    fn shared(&self) -> Option<Arc<super::audio::Shared>> {
        match self {
            Self::Real(mic) => Some(mic.shared.clone()),
            #[cfg(test)]
            Self::Fake => None,
        }
    }
}
#[derive(Clone, Copy)]
struct Os {
    snapshot: fn() -> GrantSnapshot,
    displays: fn() -> (Vec<(u32, egui::Rect)>, Option<u32>),
    window_bounds: fn(u32) -> Option<egui::Rect>,
    request_screen: fn() -> Arc<AtomicI8>,
    request_camera: fn() -> Arc<AtomicI8>,
    request_microphone: fn() -> Arc<AtomicI8>,
    open_camera: fn(Option<&str>) -> Result<Camera, String>,
    open_microphone: fn(Option<&str>) -> Result<MicSource, String>,
    list_cameras: fn() -> Result<Vec<super::devices::CameraDevice>, String>,
    list_microphones: fn() -> Result<Vec<super::devices::MicrophoneChoice>, String>,
}
impl Default for Os {
    fn default() -> Self {
        Self {
            snapshot: permissions::snapshot,
            displays: system_displays,
            window_bounds: system_window_bounds,
            request_screen: permissions::request_screen,
            request_camera: permissions::request_camera,
            request_microphone: permissions::request_microphone,
            open_camera: Camera::start,
            open_microphone: |selected| {
                super::audio::Microphone::start_selected(selected).map(MicSource::Real)
            },
            list_cameras: || {
                let helper =
                    std::env::var("OPENRECORDER_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
                super::devices::cameras(&helper)
            },
            list_microphones: super::devices::microphones,
        }
    }
}

// ---------------------------------------------------------------------------
// Settings values and the confirmed-write transaction.
// ---------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
struct SettingsValues {
    theme: u8,
    quality: u32,
    fps: u32,
    originals: bool,
    size: u32,
    shape: u8,
    mirror: bool,
    /// The bubble is drawn at all. Independent of the camera being on: the camera can run with the bubble hidden.
    show_camera: bool,
    remember: bool,
    countdown: u8,
    root: PathBuf,
    /// The person picked a frame rate. Only an unchosen default may be promoted to a qualified 60.
    fps_chosen: bool,
}
impl Default for SettingsValues {
    fn default() -> Self {
        Self {
            theme: 0,
            quality: 1920,
            fps: 30,
            originals: true,
            size: 168,
            shape: SHAPE_CIRCLE,
            mirror: true,
            show_camera: true,
            remember: true,
            countdown: 3,
            root: PathBuf::new(),
            fps_chosen: false,
        }
    }
}
impl SettingsValues {
    fn preferences(&self) -> serde_json::Value {
        serde_json::json!({"theme":match self.theme {1=>"light",2=>"dark",_=>"system"},
            "quality":self.quality,"fps":self.fps,"keepOriginals":self.originals,"cameraSize":self.size,
            "cameraShape":super::connection::shape_name(self.shape),"mirror":self.mirror,"showCamera":self.show_camera,
            "rememberInputs":self.remember,"countdownSeconds":self.countdown,"fpsChosen":self.fps_chosen})
    }
    /// Reads host preferences. Missing or unknown values fall back to the design
    /// defaults, but a stored legacy size or the legacy square shape is kept as is.
    fn from_host(settings: &serde_json::Value, root: PathBuf) -> Self {
        let countdown = settings["countdownSeconds"]
            .as_u64()
            .or_else(|| settings["countdown"].as_u64())
            .filter(|n| [0, 3].contains(n))
            .unwrap_or(3) as u8;
        Self {
            theme: match settings["theme"].as_str() {
                Some("light") => 1,
                Some("dark") => 2,
                _ => 0,
            },
            quality: settings["quality"]
                .as_u64()
                .filter(|n| [1280, 1920, 2560].contains(n))
                .unwrap_or(1920) as u32,
            fps: settings["fps"]
                .as_u64()
                .filter(|n| [24, 30, 60].contains(n))
                .unwrap_or(30) as u32,
            originals: settings["keepOriginals"].as_bool().unwrap_or(true),
            size: settings["cameraSize"]
                .as_u64()
                .filter(|n| valid_size(*n))
                .unwrap_or(168) as u32,
            shape: match settings["cameraShape"].as_str() {
                Some("rounded") => SHAPE_ROUNDED,
                Some("square") => SHAPE_SQUARE,
                Some("wide") => SHAPE_WIDE,
                Some("portrait") => SHAPE_PORTRAIT,
                _ => SHAPE_CIRCLE,
            },
            mirror: settings["mirror"].as_bool().unwrap_or(true),
            show_camera: settings["showCamera"].as_bool().unwrap_or(true),
            remember: settings["rememberInputs"].as_bool().unwrap_or(true),
            countdown,
            root,
            // A host that does not say is read as "chosen": nothing is raised on a guess.
            fps_chosen: settings["fpsChosen"].as_bool().unwrap_or(true),
        }
    }
}
fn pref_label(key: &str) -> &'static str {
    match key {
        "theme" => "Appearance",
        "quality" => "Resolution",
        "fps" => "Frame rate",
        "keepOriginals" => "Keep originals",
        "cameraSize" => "Bubble size",
        "cameraShape" => "Bubble shape",
        "mirror" => "Mirror preview",
        "showCamera" => "Show camera",
        "rememberInputs" => "Remember camera and mic",
        "countdownSeconds" => "Countdown",
        _ => "This setting",
    }
}
/// A draft only becomes confirmed when the host's reply carries every value that
/// was sent. A host that accepts the request but drops a key (an older Library
/// service ignores unknown preferences) is a failure, never a silent success.
fn settings_transaction(
    before: &SettingsValues,
    draft: &SettingsValues,
    mut send: impl FnMut(serde_json::Value) -> Result<serde_json::Value, String>,
) -> Result<Option<(SettingsValues, serde_json::Value)>, String> {
    let old = before.preferences();
    let new = draft.preferences();
    let changed: serde_json::Map<String, serde_json::Value> = new
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, value)| old.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut payload = serde_json::Map::new();
    if !changed.is_empty() {
        payload.insert("preferences".into(), changed.clone().into());
    }
    if before.root != draft.root {
        payload.insert("root".into(), serde_json::json!(draft.root));
    }
    if payload.is_empty() {
        return Ok(None);
    }
    let snapshot = send(payload.into())?;
    for (key, value) in &changed {
        if snapshot["settings"].get(key) != Some(value) {
            return Err(format!(
                "The Library service didn't confirm “{}”. Nothing was changed.",
                pref_label(key)
            ));
        }
    }
    if before.root != draft.root && snapshot["root"].as_str().map(PathBuf::from).as_ref() != Some(&draft.root)
    {
        return Err("The Library service didn't confirm the new folder. Nothing was changed.".into());
    }
    Ok(Some((draft.clone(), snapshot)))
}
/// A refused or unconfirmed host quit cannot release local capture or close its window.
fn confirm_host_quit(
    mut action: impl FnMut(&str) -> Result<serde_json::Value, String>,
) -> Result<(), String> {
    let state = action("state")?;
    if !state.get("active").is_some_and(serde_json::Value::is_null) {
        return Err("A recording still needs to finish saving. Open Library to review it, then try Quit again.".into());
    }
    let result = action("quit")?;
    if result["quitting"] != true {
        return Err("Quit wasn't confirmed. Try again from the toolbar.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Picker models (DESIGN §7). Pure data: drawing and OS access happen elsewhere.
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PanelKind {
    Source,
    Camera,
    Microphone,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SourceTab {
    Display,
    Window,
}
struct Panel {
    kind: PanelKind,
    tab: SourceTab,
    search: String,
    was_focused: bool,
    size: egui::Vec2,
}
#[derive(Clone, Debug, PartialEq)]
enum DeviceChoice {
    Off,
    SystemDefault,
    Device(String),
}
#[derive(Clone, Debug, PartialEq)]
struct DeviceRow {
    choice: Option<DeviceChoice>,
    title: String,
    subtitle: Option<String>,
    selected: bool,
    enabled: bool,
}
#[derive(Clone, Debug, PartialEq)]
struct DeviceModel {
    rows: Vec<DeviceRow>,
    remembered: Option<DeviceRow>,
}
/// `devices` holds (id, name, shares_its_name). For a camera the id is the stable
/// UID; for a microphone it is CPAL's name, the only identity CPAL gives.
fn device_model(
    camera: bool,
    devices: &[(String, String, bool)],
    preference: Option<&str>,
    remembered_name: Option<&str>,
    listed: bool,
    intent: bool,
    search: &str,
) -> DeviceModel {
    let needle = search.trim().to_lowercase();
    let keep = |title: &str| needle.is_empty() || title.to_lowercase().contains(&needle);
    let mut rows = vec![
        DeviceRow {
            choice: Some(DeviceChoice::Off),
            title: if camera { "No Camera" } else { "No Microphone" }.into(),
            subtitle: None,
            selected: !intent,
            enabled: true,
        },
        DeviceRow {
            choice: Some(DeviceChoice::SystemDefault),
            title: "System default".into(),
            subtitle: None,
            selected: intent && preference.is_none(),
            enabled: true,
        },
    ];
    for (id, name, shared) in devices {
        let subtitle = if camera {
            shared.then(|| format!("ID …{}", super::devices::id_suffix(id)))
        } else {
            shared.then(|| "Several microphones share this name".to_owned())
        };
        rows.push(DeviceRow {
            choice: Some(DeviceChoice::Device(id.clone())),
            title: name.clone(),
            subtitle,
            selected: intent && preference == Some(id.as_str()),
            // CPAL can't tell same-named microphones apart, so none of them can be chosen.
            enabled: camera || !shared,
        });
    }
    rows.retain(|row| keep(&row.title));
    let remembered = preference
        .filter(|id| listed && !devices.iter().any(|(uid, ..)| uid == id))
        .map(|_| {
            let name = remembered_name
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Remembered {}", if camera { "camera" } else { "microphone" }));
            DeviceRow {
                choice: None,
                subtitle: Some(format!("{name} isn't connected. Choose a replacement — nothing is picked for you.")),
                title: name,
                selected: false,
                enabled: false,
            }
        })
        .filter(|row| keep(&row.title));
    DeviceModel { rows, remembered }
}
#[derive(Clone, Debug, PartialEq)]
struct SourceCard {
    target: (bool, u32),
    title: String,
    monogram: char,
    selected: bool,
}
fn monogram(title: &str) -> char {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().next().unwrap_or(c))
        .unwrap_or('•')
}
fn source_cards(
    targets: &[(bool, u32, String)],
    windows: bool,
    selection: Option<(bool, u32)>,
    search: &str,
) -> Vec<SourceCard> {
    let needle = search.trim().to_lowercase();
    targets
        .iter()
        .filter(|(is_window, _, title)| {
            *is_window == windows && (needle.is_empty() || title.to_lowercase().contains(&needle))
        })
        .map(|(is_window, id, title)| SourceCard {
            target: (*is_window, *id),
            title: title.clone(),
            monogram: monogram(title),
            selected: selection == Some((*is_window, *id)),
        })
        .collect()
}
const DEVICE_PANEL: egui::Vec2 = egui::vec2(330., 440.);
const WINDOW_PANEL: egui::Vec2 = egui::vec2(520., 460.);
fn source_panel_size(tab: SourceTab, display_rows: usize) -> egui::Vec2 {
    match tab {
        SourceTab::Window => WINDOW_PANEL,
        SourceTab::Display => egui::vec2(330., (150. + 44. * display_rows as f32).clamp(240., 440.)),
    }
}
fn capability_line(tab: SourceTab) -> &'static str {
    match tab {
        SourceTab::Display => "Toolbar and camera can appear in a display recording on this system. Pick a window to keep them out.",
        SourceTab::Window => "The toolbar and camera aren't part of a window recording.",
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
enum BubbleCommand {
    Mirror(bool),
    Stop,
    Shape(u8),
    Size(u32),
    ShowCamera(bool),
    TurnOff,
}
/// Pre-flight and input opening for one Record press. Nothing here is a take yet.
struct StartAttempt {
    presence: Option<thread::JoinHandle<(Presence, Presence)>>,
    opened_camera: bool,
    opened_microphone: bool,
    opened: bool,
}
#[derive(Default)]
struct State {
    os: Os,
    connection: Option<super::connection::Connection>,
    library_offline: bool,
    reconnecting: Option<thread::JoinHandle<Result<Option<super::connection::Connection>, String>>>,
    reconnect_attempted: bool,
    tray: Option<super::tray::Tray>,
    /// Why there is no tray, when there is none. Shown in Home and Settings, never hidden.
    tray_error: Option<String>,
    /// The old floating toolbar, for development and measurement only (`--dev-toolbar` or OPENRECORDER_DEV_TOOLBAR=1).
    toolbar_mode: bool,
    /// Goes up by one on every accepted Record press; every relayed Stop/Cancel names the one it means.
    generation: u64,
    /// Monotonic start of the running take. Elapsed time is `now - started_at` and exists only while recording.
    started_at: Option<Instant>,
    launched_ms: u64,
    /// Names this launch of the app. Every relayed control must carry it.
    launch: String,
    consumed: super::controls::Consumed,
    fallback: super::controls::HomeFallback,
    capture_notice: Option<super::controls::Notice>,
    recent: Vec<(String, String)>,
    recent_job: Option<thread::JoinHandle<Vec<(String, String)>>>,
    recent_due: Option<Instant>,
    publisher: Option<super::connection::Publisher>,
    heartbeat: super::controls::Heartbeat,
    capabilities: super::capability::Capabilities,
    /// The source and camera the running (or last) take was started with, for what its measurements apply to.
    take_source: String,
    take_camera: Option<String>,
    /// The main display's id as the system last reported it (refreshed with the display rectangles).
    main_display: Option<u32>,
    capacity_job: Option<(thread::JoinHandle<super::capability::CheckOutcome>, Arc<AtomicBool>, u32)>,
    devices_checked: Option<Instant>,
    /// Whether the fallback to Home applies at all (not for the development toolbar or the measurement modes).
    fallback_enabled: bool,
    close_after_save: bool,
    quit_after_save: bool,
    #[cfg(target_os = "linux")]
    hyprland: super::hyprland::Desktop,
    command: Option<Arc<std::sync::atomic::AtomicU8>>,
    human: Option<super::connection::HumanInbox>,
    camera: Option<Camera>,
    camera_device_id: Option<String>,
    microphone_device_id: Option<String>,
    camera_permission: Option<Arc<AtomicI8>>,
    microphone_permission: Option<Arc<AtomicI8>>,
    microphone: Option<MicSource>,
    camera_intent: bool,
    microphone_intent: bool,
    intent_loaded: bool,
    camera_attention: Option<InputProblem>,
    microphone_attention: Option<InputProblem>,
    input_error: Option<(bool, String)>,
    options: super::capture::RecordOptions,
    targets: Vec<(bool, u32, String)>,
    targets_checked: Option<Instant>,
    positioned: bool,
    benchmark: bool,
    benchmark_failed: Arc<AtomicBool>,
    idle_deadline: Option<Instant>,
    recording: bool,
    toast: Option<Toast>,
    deferred_toast: Option<Toast>,
    settings_open: bool,
    settings_pane: usize,
    settings_status: String,
    settings_error: bool,
    settings_status_until: Option<Instant>,
    path_draft: Option<String>,
    prefs: SettingsValues,
    confirmed: SettingsValues,
    bubble_dirty: bool,
    hidden: bool,
    quitting: bool,
    quit_confirmation: bool,
    countdown: Option<Instant>,
    saved_until: Option<Instant>,
    countdown_closing: bool,
    panel: Option<Panel>,
    panel_closed: Option<(Instant, PanelKind)>,
    camera_names: Vec<super::devices::CameraDevice>,
    camera_choices: Option<Vec<super::devices::CameraDevice>>,
    camera_discovery: Option<thread::JoinHandle<Result<Vec<super::devices::CameraDevice>, String>>>,
    microphone_choices: Option<Vec<super::devices::MicrophoneChoice>>,
    microphone_discovery:
        Option<thread::JoinHandle<Result<Vec<super::devices::MicrophoneChoice>, String>>>,
    device_error: Option<String>,
    start: Option<StartAttempt>,
    bubble_commands: Arc<Mutex<Vec<BubbleCommand>>>,
    preview_mirror: Arc<AtomicBool>,
    meter_sample: Option<Instant>,
    meter_level: f32,
    last_theme_choice: Option<u8>,
    styles_configured: bool,
    screen_permission: Option<Arc<AtomicI8>>,
    screen_attempted: bool,
    screen_ready: bool,
    setup_open: bool,
    setup_error: Option<String>,
    layout: Arc<Mutex<super::pipeline::Layout>>,
    worker: Option<thread::JoinHandle<Result<(), String>>>,
    destination: PathBuf,
    grants: Option<GrantSnapshot>,
    space: Arc<Mutex<BubbleSpace>>,
    space_checked: Option<Instant>,
    grants_checked: Option<Instant>,
    toast_hovered: bool,
    toolbar_pos: Option<egui::Pos2>,
    toolbar_bounds: Vec<(Slot, egui::Rect)>,
    last_take_seconds: u64,
    /// Parked by the worker; read once when the worker has finished.
    pending_save: super::connection::PendingSlot,
    /// A finished capture whose completion the library has not confirmed.
    completion: Option<super::connection::PendingCompletion>,
    completion_next: Option<Instant>,
    completion_auto: u32,
    #[cfg(test)]
    host_override: Option<Arc<dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync>>,
}
fn friendly_worker_error(error: &str, frames: u64, seconds: u64) -> Toast {
    let text = if frames == 0 {
        "Couldn't start recording".to_owned()
    } else if error.to_lowercase().contains("camera") {
        format!("Recording stopped — camera disconnected · Up to {} kept", format_elapsed(seconds))
    } else {
        "Recording stopped".to_owned()
    };
    Toast::error(text).with_details(error.to_owned())
}

impl State {
    // ----- preferences ---------------------------------------------------
    /// Applies host preferences. Intent only arms: nothing is ever opened here.
    fn preferences(&mut self, settings: &serde_json::Value) {
        let root = self.destination.clone();
        let values = SettingsValues::from_host(settings, root);
        // Device choices and intent are imported exactly once, at startup. With "Remember camera and
        // mic" off nothing arms then. Every later refresh (a settings write, the
        // host's `preferences` command, an agent's settings call) leaves the
        // person's current-session intent alone: it only ever changes through a
        // human Camera, Mic, Record or device pick.
        if !self.intent_loaded {
            self.intent_loaded = true;
            self.camera_device_id = settings["cameraDeviceId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            self.microphone_device_id = settings["microphoneDeviceId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            self.camera_intent = values.remember && settings["camera"].as_bool().unwrap_or(false);
            self.microphone_intent = values.remember && settings["microphone"].as_bool().unwrap_or(false);
        }
        if !self.recording {
            self.prefs = values.clone();
        }
        self.confirmed = values;
        self.options.quality = self.confirmed.quality;
        self.options.fps = self.confirmed.fps;
        self.options.originals = self.confirmed.originals;
        self.sync_runtime();
    }
    /// Layout.mirror is always false: the preview mirror is cosmetic and the
    /// composited camera in the recording is never mirrored.
    fn sync_runtime(&mut self) {
        self.preview_mirror.store(self.prefs.mirror, Ordering::Relaxed);
        if let Ok(mut layout) = self.layout.lock() {
            layout.shape = self.prefs.shape;
            layout.mirror = false;
            layout.visible = self.prefs.show_camera;
        }
    }
    fn settings_values(&self) -> SettingsValues {
        self.prefs.clone()
    }
    fn commit_settings(&mut self, draft: &SettingsValues) {
        let before = self.confirmed.clone();
        if self.recording {
            // The library refuses preference writes mid-take. Bubble shape and
            // size stay live locally and are saved once the take is finished.
            let mut live = before.clone();
            live.size = draft.size;
            live.shape = draft.shape;
            live.mirror = draft.mirror;
            live.show_camera = draft.show_camera;
            self.prefs = live;
            self.bubble_dirty = true;
            self.sync_runtime();
            return;
        }
        let result = settings_transaction(&before, draft, |payload| {
            self.connection
                .as_ref()
                .ok_or("The Library is unavailable. Try again when it reconnects.")?
                .action("settings", payload)
        });
        match result {
            Ok(Some((confirmed, snapshot))) => {
                self.prefs = confirmed.clone();
                self.confirmed = confirmed;
                self.bubble_dirty = false;
                if snapshot["settings"].is_object() {
                    self.preferences(&snapshot["settings"]);
                }
                if let Some(root) = snapshot["root"].as_str() {
                    self.destination = PathBuf::from(root);
                    self.prefs.root = self.destination.clone();
                    self.confirmed.root = self.destination.clone();
                }
                self.settings_status = "Saved".into();
                self.settings_error = false;
            }
            Ok(None) => return,
            Err(error) => {
                self.prefs = before;
                self.sync_runtime();
                super::event_log::event("settings_save_failed", serde_json::json!({"error":error}));
                self.settings_status = format!("Couldn't save. {error}");
                self.settings_error = true;
            }
        }
        self.settings_status_until = Some(Instant::now() + if self.settings_error { Duration::from_secs(8) } else { Duration::from_millis(1500) });
        self.path_draft = Some(self.destination.display().to_string());
    }
    fn flush_bubble_settings(&mut self) {
        if self.bubble_dirty && !self.recording {
            let draft = self.prefs.clone();
            self.bubble_dirty = false;
            self.commit_settings(&draft);
        }
    }
    fn persist_intent(&mut self) -> bool {
        // A human change owns the session from here: a late first import must not overwrite it.
        self.intent_loaded = true;
        let Some(conn) = &self.connection else {
            self.show_toast(Toast::error("The Library is unavailable. Your choice wasn't saved."));
            return false;
        };
        let mut preferences = serde_json::json!({
            "cameraDeviceId": self.camera_device_id, "microphoneDeviceId": self.microphone_device_id,
        });
        // "Remember camera and mic" off: they always start off, so intent is not written.
        if self.prefs.remember {
            preferences["camera"] = self.camera_intent.into();
            preferences["microphone"] = self.microphone_intent.into();
        }
        // Partial preferences merge at the host, preserving unrelated settings.
        match conn.action("settings", serde_json::json!({"preferences": preferences})) {
            Ok(_) => true,
            Err(e) => {
                self.show_toast(Toast::error(e));
                false
            }
        }
    }
    fn open_library(&mut self) {
        self.open_library_route("");
    }
    /// Opens Home in the browser, at a route ("settings", "last" = the newest stopped recording) or at its start (""). No access link is shown.
    fn open_library_route(&mut self, route: &str) {
        let payload = if route.is_empty() { serde_json::json!({}) } else { serde_json::json!({"route":route}) };
        let result = self
            .connection
            .as_ref()
            .ok_or_else(|| "The Library is unavailable. Try again when it reconnects.".to_owned())
            .and_then(|conn| {
                conn.action("open-library", payload)
                    .map(|_| ())
            });
        if let Err(error) = result {
            self.show_toast(Toast::error(error));
        }
    }

    // ----- toast ---------------------------------------------------------
    fn display_capture(&self) -> bool {
        self.options.selection.is_none_or(|(window, _)| !window)
    }
    /// A display take may capture the pill itself, so during one only errors interrupt.
    fn show_toast(&mut self, toast: Toast) {
        if self.recording && !toast.error && self.display_capture() {
            self.deferred_toast = Some(toast);
        } else {
            self.toast = Some(toast);
        }
    }
    fn toast_action(&mut self, action: ToastAction) {
        match action {
            ToastAction::OpenScreenSettings => {
                let _ = permissions::screen_settings();
            }
            ToastAction::OpenCameraSettings => {
                let _ = permissions::input_settings(true);
            }
            ToastAction::OpenMicSettings => {
                let _ = permissions::input_settings(false);
            }
            ToastAction::ChooseSource => self.open_panel(PanelKind::Source),
            ToastAction::ChooseCamera => self.open_panel(PanelKind::Camera),
            ToastAction::ChooseMicrophone => self.open_panel(PanelKind::Microphone),
            ToastAction::ShowInLibrary => self.open_library(),
            ToastAction::RetrySave => self.retry_completion(true),
            ToastAction::TurnOffCamera => self.turn_off(true),
            ToastAction::TurnOffMicrophone => self.turn_off(false),
        }
    }

    // ----- state derived for the UI -------------------------------------
    fn locked(&self) -> bool {
        self.recording
            || self.countdown.is_some()
            || self.countdown_closing
            || self.start.is_some()
            || self.completion.is_some()
    }
    fn camera_live(&self) -> bool {
        self.camera.as_ref().is_some_and(|c| c.ready.load(Ordering::Relaxed))
    }
    fn camera_opening(&self) -> bool {
        self.camera_permission.is_some() || (self.camera.is_some() && !self.camera_live())
    }
    fn microphone_opening(&self) -> bool {
        self.microphone_permission.is_some()
    }
    fn mark(&self, camera: bool) -> Mark {
        if camera {
            input_mark(
                self.camera_live(),
                self.camera_opening(),
                self.camera_attention.is_some(),
                self.camera_intent,
            )
        } else {
            input_mark(
                self.microphone.is_some(),
                self.microphone_opening(),
                self.microphone_attention.is_some(),
                self.microphone_intent,
            )
        }
    }
    fn elapsed_seconds(&self) -> u64 {
        let p = &self.options.progress;
        p.frames.load(Ordering::Relaxed) / p.fps.load(Ordering::Relaxed).max(1) as u64
    }
    fn face(&self, now: Instant) -> Face {
        if self.recording {
            return match self.options.progress.phase.load(Ordering::Relaxed) {
                0 => Face::Starting,
                2 => Face::Saving,
                _ => Face::Recording(self.elapsed_seconds()),
            };
        }
        if let Some(deadline) = self.countdown {
            return Face::Countdown(countdown_number(deadline.saturating_duration_since(now)));
        }
        if self.countdown_closing {
            return Face::Countdown(1);
        }
        if self.start.is_some() {
            return Face::Starting;
        }
        if let Some(pending) = &self.completion {
            let starting = matches!(pending.kind(), super::connection::CompletionKind::Begin(_));
            return match (starting, self.completion_next.is_some()) {
                // A begin the library hasn't confirmed is "Starting", never "Saving": nothing was captured.
                (true, true) => Face::Starting,
                (_, true) => Face::Saving,
                _ => Face::RetrySave,
            };
        }
        if self.saved_until.is_some_and(|t| now < t) {
            return Face::Saved;
        }
        if self.worker.is_some() || self.quit_after_save {
            return Face::Unavailable;
        }
        Face::Ready
    }
    /// Which inputs a Record press would start: armed and not already open.
    fn armed_for_record(&self) -> (bool, bool) {
        (
            self.camera_intent && self.camera.is_none(),
            self.microphone_intent && self.microphone.is_none(),
        )
    }
    fn input_label(&self, camera: bool) -> String {
        let (noun, mark) = if camera { ("Camera", self.mark(true)) } else { ("Mic", self.mark(false)) };
        format!("{noun}, {}", mark_word(mark))
    }

    // ----- lifecycle -----------------------------------------------------
    fn release_idle(&mut self, ctx: &egui::Context) {
        self.camera = None;
        self.microphone = None;
        // A late OS completion must never reopen a device after Hide.
        self.camera_permission = None;
        self.microphone_permission = None;
        self.camera_attention = None;
        self.microphone_attention = None;
        self.camera_choices = None;
        self.microphone_choices = None;
        self.panel = None;
        self.settings_open = false;
        self.setup_open = false;
        close_children_except(ctx, &[]);
    }
    fn hide(&mut self, ctx: &egui::Context) {
        if !self.toolbar_mode {
            // There is no toolbar to hide: the root window is an invisible host for the bubble and pickers.
            return;
        }
        self.hidden = true;
        self.countdown = None;
        self.countdown_closing = false;
        self.start = None;
        self.panel = None;
        self.settings_open = false;
        self.quit_confirmation = false;
        self.toast = None;
        self.deferred_toast = None;
        // A take keeps its bubble; everything else stops with the toolbar.
        if self.recording {
            close_children_except(ctx, &[VP_CAMERA]);
        } else {
            self.release_idle(ctx);
        }
        #[cfg(target_os = "macos")]
        if self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        }
        // Taskbar restoration remains available on platforms without a tray.
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
    /// Show is passive: the toolbar returns, inputs with intent show as armed.
    fn show_toolbar(&mut self, ctx: &egui::Context) {
        if !self.toolbar_mode {
            // Home carries the controls; a remote "show" (a second launch) reopens it.
            self.open_library();
            return;
        }
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    /// Brings the development toolbar back. Without it there is nothing to bring back: notices and
    /// pickers are their own windows.
    fn reveal_root(&mut self, ctx: &egui::Context) {
        if !self.toolbar_mode {
            return;
        }
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }
    fn request_quit(&mut self, ctx: &egui::Context) {
        if self.recording {
            self.quit_confirmation = true;
            return;
        }
        if self.completion.is_some() {
            self.retry_completion(true);
            if self.completion.is_some() {
                // The library still lists the take as active: quitting would strand it.
                self.show_toast(Toast::error("Couldn't quit · The last recording isn't settled yet").with_action(ToastAction::RetrySave));
                self.reveal_root(ctx);
                ctx.request_repaint();
                return;
            }
        }
        let result = if let Some(conn) = &self.connection {
            confirm_host_quit(|op| conn.action(op, serde_json::json!({})))
        } else {
            Ok(())
        };
        if let Err(error) = result {
            super::event_log::event("quit_refused", serde_json::json!({"error":error}));
            self.show_toast(Toast::error("Couldn't quit").with_details(error));
            self.reveal_root(ctx);
            ctx.request_repaint();
            return;
        }
        self.countdown = None;
        self.countdown_closing = false;
        self.start = None;
        // Quit turns devices off; the host has already confirmed there is nothing to save.
        self.release_idle(ctx);
        self.quitting = true;
        close_recorder(ctx);
    }

    // ----- inputs --------------------------------------------------------
    fn turn_off(&mut self, camera: bool) {
        if self.locked() {
            return;
        }
        if camera {
            self.camera = None;
            self.camera_permission = None;
            self.camera_intent = false;
            self.camera_attention = None;
        } else {
            self.microphone = None;
            self.microphone_permission = None;
            self.microphone_intent = false;
            self.microphone_attention = None;
        }
        self.persist_intent();
    }
    fn attention(&mut self, problem: InputProblem) {
        let toast = problem.toast(false);
        if problem.is_camera() {
            self.camera_attention = Some(problem);
        } else {
            self.microphone_attention = Some(problem);
        }
        self.show_toast(toast);
    }
    /// A person's click on Camera or Mic. The status is read fresh; a refused
    /// input gets a pill and never another OS request.
    fn click_input(&mut self, camera: bool, grant: Grant) {
        if self.locked() {
            return;
        }
        let on = if camera {
            self.camera.is_some() || self.camera_permission.is_some()
        } else {
            self.microphone.is_some() || self.microphone_permission.is_some()
        };
        if on {
            self.turn_off(camera);
            return;
        }
        match grant {
            Grant::Denied => return self.attention(InputProblem::AccessOff { camera }),
            Grant::Restricted => return self.attention(InputProblem::Restricted { camera }),
            _ => {}
        }
        if camera {
            self.camera_intent = true;
            self.camera_attention = None;
            self.camera_permission = Some((self.os.request_camera)());
        } else {
            self.microphone_intent = true;
            self.microphone_attention = None;
            self.microphone_permission = Some((self.os.request_microphone)());
        }
        self.persist_intent();
    }
    fn open_camera_now(&mut self) -> Result<(), String> {
        let camera = (self.os.open_camera)(self.camera_device_id.as_deref())?;
        self.camera = Some(camera);
        Ok(())
    }
    fn open_microphone_now(&mut self) -> Result<(), String> {
        let microphone = (self.os.open_microphone)(self.microphone_device_id.as_deref())?;
        self.microphone = Some(microphone);
        Ok(())
    }
    fn input_failed(&mut self, camera: bool, detail: String) {
        if camera {
            self.camera = None;
        } else {
            self.microphone = None;
        }
        self.input_error = Some((camera, detail.clone()));
        if self.start.is_none() {
            self.attention(InputProblem::Failed { camera, detail });
        }
    }

    // ----- record --------------------------------------------------------
    fn record_action(&mut self, ctx: &egui::Context) {
        let grants = if self.benchmark {
            GrantSnapshot { screen: true, camera: Grant::SystemManaged, microphone: Grant::SystemManaged }
        } else {
            permissions::snapshot()
        };
        self.record_press(ctx, grants);
    }
    fn cancel_start(&mut self) {
        if let Some(start) = self.start.take() {
            if start.opened_camera {
                self.camera = None;
                self.camera_permission = None;
            }
            if start.opened_microphone {
                self.microphone = None;
                self.microphone_permission = None;
            }
        }
    }
    /// A person's Record press. `grants` was read just now, so a stale cached
    /// status can't turn into an OS request.
    fn record_press(&mut self, ctx: &egui::Context, grants: GrantSnapshot) {
        if self.recording {
            super::STOP_REQUEST.store(true, Ordering::Relaxed);
            return;
        }
        if self.countdown.take().is_some() || self.countdown_closing {
            self.countdown_closing = false;
            close_present_viewport(ctx, vp(VP_COUNTDOWN));
            self.cancel_start();
            return;
        }
        if self.start.is_some() {
            self.cancel_start();
            return;
        }
        if self.completion.is_some() {
            // The last take is not stored yet; a new one would be refused as "already active".
            self.retry_completion(true);
            return;
        }
        if self.worker.is_some() {
            self.show_toast(Toast::info("Finishing the last recording…"));
            return;
        }
        self.screen_ready = grants.screen || self.benchmark;
        if !self.screen_ready {
            if self.screen_permission.is_some() {
                return;
            }
            match permissions::decide_screen_request(false, self.screen_attempted) {
                permissions::ScreenDecision::Prompt => {
                    self.screen_attempted = true;
                    self.screen_permission = Some((self.os.request_screen)());
                    self.show_toast(Toast::info("Waiting for Screen Recording access…"));
                }
                _ => {
                    self.show_toast(
                        Toast::error("Screen Recording is off").with_action(ToastAction::OpenScreenSettings),
                    );
                }
            }
            return;
        }
        if let Some(free) = self
            .connection
            .as_ref()
            .filter(|_| !self.library_offline)
            .and_then(super::connection::Connection::free_bytes)
            .filter(|free| *free < MIN_FREE_BYTES)
        {
            self.show_toast(Toast::error(format!(
                "Not enough space to record · {} free",
                format_free(free)
            )));
            return;
        }
        #[cfg(not(target_os = "linux"))]
        if let Some(selection) = self.options.selection {
            let present = scap::get_all_targets().iter().any(|target| match target {
                scap::Target::Window(w) => selection == (true, w.id),
                scap::Target::Display(d) => selection == (false, d.id),
            });
            if !present && !self.benchmark {
                self.options.selection = None;
                self.show_toast(
                    Toast::error("The source you chose isn't available").with_action(ToastAction::ChooseSource),
                );
                return;
            }
        }
        let (want_camera, want_microphone) = self.armed_for_record();
        let armed = [
            ArmedInput {
                camera: true,
                wants_open: want_camera,
                grant: grants.camera,
                remembered: self.camera_device_id.clone(),
                remembered_name: self.camera_device_id.as_ref().and_then(|id| {
                    self.camera_names.iter().find(|d| &d.id == id).map(|d| d.name.clone())
                }),
                presence: Presence::Unchecked,
            },
            ArmedInput {
                camera: false,
                wants_open: want_microphone,
                grant: grants.microphone,
                remembered: self.microphone_device_id.clone(),
                remembered_name: self.microphone_device_id.clone(),
                presence: Presence::Unchecked,
            },
        ];
        if let Err(problem) = evaluate_armed(&armed) {
            return self.fail_start(problem);
        }
        self.saved_until = None;
        self.reveal_root(ctx);
        // Remembered devices are verified against a read-only listing first.
        let need_camera_list = want_camera && self.camera_device_id.is_some();
        let need_mic_list = want_microphone && self.microphone_device_id.is_some();
        let presence = (need_camera_list || need_mic_list).then(|| {
            let os = self.os;
            thread::spawn(move || {
                let camera = if need_camera_list {
                    match (os.list_cameras)() {
                        Ok(list) => Presence::Listed(list.into_iter().map(|d| (d.id, d.name)).collect()),
                        Err(e) if e.contains("No camera was found") => Presence::Listed(vec![]),
                        Err(e) => Presence::ListFailed(e),
                    }
                } else {
                    Presence::Unchecked
                };
                let mic = if need_mic_list {
                    match (os.list_microphones)() {
                        Ok(list) => Presence::Listed(
                            list.into_iter()
                                .flat_map(|m| {
                                    let twice = m.duplicate as usize + 1;
                                    std::iter::repeat((m.name.clone(), m.name)).take(twice)
                                })
                                .collect(),
                        ),
                        Err(e) => Presence::ListFailed(e),
                    }
                } else {
                    Presence::Unchecked
                };
                (camera, mic)
            })
        });
        self.input_error = None;
        // An accepted press is a new take: a new generation, and the last failure's notice is done.
        self.generation += 1;
        self.capture_notice = None;
        self.start = Some(StartAttempt {
            presence,
            opened_camera: false,
            opened_microphone: false,
            opened: false,
        });
        ctx.request_repaint();
    }
    /// A blocker found at Record: nothing starts, what this press opened is closed,
    /// and the pill names the one problem. Turning the input off is the explicit way on.
    fn fail_start(&mut self, problem: InputProblem) {
        self.cancel_start();
        if problem.is_camera() {
            self.camera_attention = Some(problem.clone());
        } else {
            self.microphone_attention = Some(problem.clone());
        }
        self.show_toast(problem.toast(true));
    }
    /// Drives a pending Record press: verify devices, open armed inputs, wait for
    /// them, then count down. Any failure closes what this press opened.
    fn advance_start(&mut self, ctx: &egui::Context) {
        let Some(start) = &mut self.start else { return };
        if start.presence.as_ref().is_some_and(|job| !job.is_finished()) {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        if let Some(job) = start.presence.take() {
            let Ok((camera, mic)) = job.join() else {
                return self.fail_start(InputProblem::Failed { camera: true, detail: "Device check was interrupted".into() });
            };
            let remembered_before = self.camera_device_id.as_ref().and_then(|id| {
                self.camera_names.iter().find(|d| &d.id == id).map(|d| d.name.clone())
            });
            if let Presence::Listed(devices) = &camera {
                let listed: Vec<_> = devices
                    .iter()
                    .map(|(id, name)| super::devices::CameraDevice { id: id.clone(), name: name.clone() })
                    .collect();
                merge_camera_names(&mut self.camera_names, &listed);
            }
            let (want_camera, want_microphone) = self.armed_for_record();
            let name_of = |id: &Option<String>, list: &[super::devices::CameraDevice]| {
                id.as_ref().and_then(|id| list.iter().find(|d| &d.id == id).map(|d| d.name.clone()))
            };
            let armed = [
                ArmedInput {
                    camera: true,
                    wants_open: want_camera,
                    grant: Grant::Allowed,
                    remembered: self.camera_device_id.clone(),
                    remembered_name: remembered_before.or_else(|| name_of(&self.camera_device_id, &self.camera_names)),
                    presence: camera,
                },
                ArmedInput {
                    camera: false,
                    wants_open: want_microphone,
                    grant: Grant::Allowed,
                    remembered: self.microphone_device_id.clone(),
                    remembered_name: self.microphone_device_id.clone(),
                    presence: mic,
                },
            ];
            if let Err(problem) = evaluate_armed(&armed) {
                return self.fail_start(problem);
            }
        }
        let Some(start) = &mut self.start else { return };
        if !start.opened {
            start.opened = true;
            let (want_camera, want_microphone) = (
                self.camera_intent && self.camera.is_none() && self.camera_permission.is_none(),
                self.microphone_intent && self.microphone.is_none() && self.microphone_permission.is_none(),
            );
            start.opened_camera = want_camera;
            start.opened_microphone = want_microphone;
            // Grants were verified above, so these resolve without a prompt.
            if want_camera {
                self.camera_permission = Some((self.os.request_camera)());
            }
            if want_microphone {
                self.microphone_permission = Some((self.os.request_microphone)());
            }
            ctx.request_repaint();
            return;
        }
        if self.camera_permission.is_some() || self.microphone_permission.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        let camera_failed = self.camera_intent && self.camera.is_none();
        let microphone_failed = self.microphone_intent && self.microphone.is_none();
        if camera_failed || microphone_failed {
            let (camera, detail) = self
                .input_error
                .take()
                .unwrap_or((camera_failed, "The input didn't open".into()));
            return self.fail_start(InputProblem::Failed { camera, detail });
        }
        if self.camera.is_some() && !self.camera_live() {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        // Everything armed is open: close the HUD-free preflight and count down.
        self.start = None;
        let seconds = self.prefs.countdown as u64;
        if seconds == 0 {
            self.begin_recording();
        } else {
            self.countdown = Some(Instant::now() + Duration::from_secs(seconds));
        }
        ctx.request_repaint();
    }
    /// Everything a take is started with, decided once, now. The display it records is resolved **fresh** from the
    /// system here (not read from the cache, which can be two seconds old) and handed to the capture as an explicit
    /// target; the frame rate, the capability key and the take's metadata all use that same resolved target. If a
    /// chosen display has gone, the take is refused rather than moved to another one.
    fn prepare_take(&mut self) -> Result<PreparedTake, String> {
        let (displays, main) = (self.os.displays)();
        self.main_display = main;
        let ids: Vec<u32> = displays.iter().map(|(id, _)| *id).collect();
        let target = resolve_take_source(cfg!(target_os = "linux"), self.options.selection, &ids, main)?;
        let mut options = self.options.clone();
        options.selection = target.selection;
        options.quality = self.confirmed.quality;
        options.fps = self.fps_for_source(&target.key);
        options.originals = self.confirmed.originals;
        options.source_error = self.camera.as_ref().map(|c| c.error.clone());
        options.microphone = self.microphone.as_ref().and_then(MicSource::shared);
        Ok(PreparedTake { options, source: target.key })
    }
    fn begin_recording(&mut self) {
        if self.worker.is_some() || self.recording {
            return;
        }
        super::STOP_REQUEST.store(false, Ordering::Relaxed);
        let prepared = match self.prepare_take() {
            Ok(prepared) => prepared,
            Err(message) => {
                self.show_toast(Toast::error(message).with_action(ToastAction::ChooseSource));
                return;
            }
        };
        let directory = self.destination.join(format!(
            "take-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        let camera = self.camera.as_ref().map(|c| c.pixels.clone());
        let connection = self.connection.clone();
        let layout = self.layout.clone();
        let options = prepared.options;
        let pending = self.pending_save.clone();
        self.take_source = prepared.source;
        self.take_camera = self.camera_key();
        self.recording = true;
        self.worker = Some(thread::spawn(move || {
            super::connection::record(connection, directory, camera, layout, options, pending)
        }));
    }
    fn host_action(&self, op: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
        #[cfg(test)]
        if let Some(host) = &self.host_override {
            return host(op, data);
        }
        self.connection
            .as_ref()
            .ok_or_else(|| super::connection::UNAVAILABLE.to_string())?
            .action(op, data)
    }
    fn begin_completion(&mut self, pending: super::connection::PendingCompletion) {
        self.completion = Some(pending);
        self.completion_auto = 0;
        self.completion_next = Some(Instant::now() + Duration::from_secs(2));
    }
    /// One more try at storing the finished take. Automatic tries run at 2, 5 and
    /// 15 s; after that only a person's press (the Record slot, the pill) retries.
    fn retry_completion(&mut self, manual: bool) {
        let Some(mut pending) = self.completion.take() else { return };
        let attempt = pending.attempt(&mut |op, data| self.host_action(op, data));
        match attempt {
            Ok(done) => {
                self.completion_next = None;
                self.completion_auto = 0;
                let aborted = matches!(pending.kind(), super::connection::CompletionKind::Abort(_));
                self.toast = Some(match (done, aborted) {
                    (super::connection::Completed::NotStarted, _) => Toast::info("Nothing was recorded"),
                    (super::connection::Completed::Saved, _) => {
                        self.saved_until = Some(Instant::now() + Duration::from_millis(1500));
                        Toast::info(format!("Saved · {}", format_elapsed(self.last_take_seconds))).with_action(ToastAction::ShowInLibrary)
                    }
                    (_, true) => Toast::info("Kept what was recorded").with_action(ToastAction::ShowInLibrary),
                    _ => Toast::info("Saved without full details").with_action(ToastAction::ShowInLibrary),
                });
                self.flush_bubble_settings();
            }
            Err(error) => {
                if !manual {
                    self.completion_auto += 1;
                }
                self.completion_next = match self.completion_auto {
                    0 => Some(Instant::now() + Duration::from_secs(2)),
                    1 => Some(Instant::now() + Duration::from_secs(5)),
                    2 => Some(Instant::now() + Duration::from_secs(15)),
                    _ => None,
                };
                let give_up = self.completion_next.is_none();
                self.completion = Some(pending);
                if manual || give_up {
                    let starting = matches!(self.completion.as_ref().map(|p| p.kind()), Some(super::connection::CompletionKind::Begin(_)));
                    self.show_toast(
                        Toast::error(if starting { "Couldn't confirm the recording started · Nothing was recorded" } else { "Couldn't finish saving · Your recording is safe" })
                            .with_action(ToastAction::RetrySave)
                            .with_details(error),
                    );
                }
            }
        }
    }
    fn tick_completion(&mut self, ctx: &egui::Context) {
        let Some(due) = self.completion_next else { return };
        if self.completion.is_none() {
            self.completion_next = None;
        } else if Instant::now() >= due {
            self.retry_completion(false);
        } else {
            ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
        }
    }
    fn finish_take(&mut self, result: std::thread::Result<Result<(), String>>) {
        let frames = self.options.progress.frames.load(Ordering::Relaxed);
        let seconds = self.elapsed_seconds();
        self.last_take_seconds = seconds;
        let pending = self.pending_save.lock().ok().and_then(|mut slot| slot.take());
        let unsaved = pending.as_ref().map(|p| p.last_error.clone());
        // What the finished take measured teaches the capability record; only the speed gate or a delivered
        // rate changes it.
        if let Some(summary) = self.options.progress.summary.lock().ok().and_then(|mut s| s.take()) {
            let camera = self.take_camera.clone();
            if self.capabilities.record_take(&summary, &self.take_source, camera.as_deref(), super::codec(), super::connection::now_ms()) {
                self.save_capabilities();
            }
        }
        self.heartbeat.reset();
        self.recent_due = Some(Instant::now());
        self.capture_notice = match &result {
            Ok(Err(e)) => Some(super::controls::failure_notice(e, self.options.progress.fps.load(Ordering::Relaxed), self.confirmed.quality, seconds, frames, self.generation)),
            Ok(Ok(())) => None,
            Err(_) => Some(super::controls::failure_notice("The capture worker failed", 30, self.confirmed.quality, seconds, frames, self.generation)),
        };
        match (&result, unsaved) {
            // Captured and confirmed by the library.
            (Ok(Ok(())), None) => {
                self.saved_until = Some(Instant::now() + Duration::from_millis(1500));
                let mut saved = Toast::info(format!("Saved · {}", format_elapsed(seconds)))
                    .with_action(ToastAction::ShowInLibrary);
                saved.last_tick = Instant::now();
                self.toast = Some(saved);
                self.deferred_toast = None;
            }
            // Captured, but the library did not confirm: never "Saved".
            (Ok(Ok(())), Some(error)) => {
                self.toast = Some(
                    Toast::error("Recorded · Saving to the Library didn't finish")
                        .with_action(ToastAction::RetrySave)
                        .with_details(error),
                );
            }
            (Ok(Err(e)), unsaved) => {
                let mut toast = friendly_worker_error(e, frames, seconds);
                if unsaved.is_some() {
                    toast.action = Some(ToastAction::RetrySave);
                }
                self.toast = Some(toast);
            }
            (Err(_), _) => {
                let mut toast = Toast::error("Recording stopped").with_details("The capture worker failed");
                if pending.is_some() {
                    toast.action = Some(ToastAction::RetrySave);
                }
                self.toast = Some(toast);
            }
        }
        if let Some(pending) = pending {
            self.begin_completion(pending);
        }
        if let Some(mut deferred) = self.deferred_toast.take() {
            // Only an error takes the pill from the saved confirmation.
            deferred.expires = Instant::now() + toast_lifetime(false);
            if self.toast.is_none() {
                self.toast = Some(deferred);
            }
        }
        self.flush_bubble_settings();
    }
}

// ---------------------------------------------------------------------------
// Widgets shared by Settings, pickers and setup. Fixed metrics, painter-drawn,
// identical in Light and Dark and at every scale.
// ---------------------------------------------------------------------------
fn panel_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(t.surface)
        .stroke(egui::Stroke::new(1., t.hairline))
        .corner_radius(12)
        .inner_margin(12)
}
fn text_galley(ui: &egui::Ui, text: &str, size: f32) -> Arc<egui::Galley> {
    ui.painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), Color32::PLACEHOLDER)
}
fn focus_ring(painter: &egui::Painter, t: &Tokens, response: &egui::Response, rect: egui::Rect, radius: f32) {
    if response.has_focus() {
        painter.rect_stroke(rect.expand(2.), radius + 2., egui::Stroke::new(2., t.accent), egui::StrokeKind::Outside);
    }
}
fn button(ui: &mut egui::Ui, t: &Tokens, label: &str, min_width: f32, primary: bool, enabled: bool) -> egui::Response {
    let galley = text_galley(ui, label, 13.);
    let width = (galley.size().x + 24.).max(min_width);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, 28.),
        if enabled { egui::Sense::click() } else { egui::Sense::hover() },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let fill = if primary { t.primary } else { t.control };
    let fill = if enabled && response.hovered() { shade(fill, if primary { 0.85 } else { 0.94 }) } else { fill };
    let ink = match (primary, enabled) {
        (_, false) => t.tertiary,
        (true, true) => t.on_primary,
        (false, true) => t.text,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 8, fill);
    painter.galley(rect.center() - galley.size() / 2., galley, ink);
    focus_ring(painter, t, &response, rect, 8.);
    response
}
/// Segmented control. Returns the index the person just picked.
fn segmented(ui: &mut egui::Ui, t: &Tokens, salt: &str, selected: usize, labels: &[&str], seg_width: f32, enabled: bool) -> Option<usize> {
    let width = seg_width * labels.len() as f32 + 4.;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 28.), egui::Sense::hover());
    ui.painter().rect_filled(rect, 8, t.control);
    let thumb = if ui.visuals().dark_mode { t.hairline } else { t.surface };
    let mut picked = None;
    for (index, label) in labels.iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            rect.min + egui::vec2(2. + index as f32 * seg_width, 2.),
            egui::vec2(seg_width, 24.),
        );
        let response = ui.interact(
            cell,
            ui.id().with((salt, index)),
            if enabled { egui::Sense::click() } else { egui::Sense::hover() },
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, index == selected, *label)
        });
        let painter = ui.painter();
        if index == selected {
            painter.rect_filled(cell, 6, thumb);
        }
        let galley = text_galley(ui, label, 13.);
        let ink = if enabled { t.text } else { t.tertiary };
        ui.painter().galley(cell.center() - galley.size() / 2., galley, ink);
        focus_ring(ui.painter(), t, &response, cell, 6.);
        if response.clicked() {
            picked = Some(index);
        }
    }
    picked
}
fn switch(ui: &mut egui::Ui, t: &Tokens, label: &str, value: &mut bool, enabled: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(36., 20.),
        if enabled { egui::Sense::click() } else { egui::Sense::hover() },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *value, label));
    let mut changed = false;
    if response.clicked() {
        *value = !*value;
        changed = true;
    }
    let painter = ui.painter();
    let track = if *value { t.accent } else { t.hairline };
    painter.rect_filled(rect, 10, if enabled { track } else { shade(track, 0.6) });
    let knob_x = if *value { rect.right() - 10. } else { rect.left() + 10. };
    painter.circle_filled(egui::pos2(knob_x, rect.center().y), 8., Color32::WHITE);
    focus_ring(painter, t, &response, rect, 10.);
    changed
}
/// Label left, control right, muted support text under the label, hairline below.
fn settings_row(ui: &mut egui::Ui, t: &Tokens, label: &str, support: Option<&str>, control: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.);
    let total = ui.available_width();
    ui.horizontal(|ui| {
        let left = (total - 250.).max(180.);
        ui.allocate_ui_with_layout(egui::vec2(left, 0.), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_max_width(left);
            ui.label(egui::RichText::new(label).size(13.).color(t.text));
            if let Some(support) = support {
                ui.add(egui::Label::new(egui::RichText::new(support).size(12.).color(t.secondary)).wrap());
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(8.);
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1., t.separator));
}
fn settings_nav(ui: &mut egui::Ui, t: &Tokens, pane: &mut usize) -> Vec<egui::Rect> {
    SETTINGS_PANES
        .iter()
        .enumerate()
        .map(|(index, (label, icon))| {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.), egui::Sense::click());
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, *pane == index, *label));
            if response.clicked() {
                *pane = index;
            }
            let painter = ui.painter();
            if *pane == index {
                painter.rect_filled(rect, 8, t.selected);
            } else if response.hovered() {
                painter.rect_filled(rect, 8, t.control);
            }
            let ink = if *pane == index { t.accent } else { t.text };
            paint_icon(painter, *icon, rect.left_center() + egui::vec2(18., 0.), ink);
            let galley = text_galley(ui, label, 13.);
            ui.painter().galley(rect.left_center() + egui::vec2(36., -galley.size().y / 2.), galley, ink);
            focus_ring(ui.painter(), t, &response, rect, 8.);
            ui.add_space(2.);
            rect
        })
        .collect()
}
/// A picker row: title, optional second line, ✓ on the right when selected.
fn picker_row(ui: &mut egui::Ui, t: &Tokens, title: &str, subtitle: Option<&str>, selected: bool, enabled: bool) -> egui::Response {
    let height = if subtitle.is_some() { 52. } else { 40. };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        if enabled { egui::Sense::click() } else { egui::Sense::hover() },
    );
    let accessible = match subtitle {
        Some(sub) => format!("{title}. {sub}"),
        None => title.to_owned(),
    };
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, accessible.clone()));
    let painter = ui.painter();
    if enabled && response.hovered() {
        painter.rect_filled(rect.shrink2(egui::vec2(0., 1.)), 8, t.control);
    }
    let ink = if enabled { t.text } else { t.tertiary };
    let text_width = rect.width() - 52.;
    let title_galley = truncated_galley(painter, title, egui::FontId::proportional(13.), ink, text_width);
    let sub_galley = subtitle.map(|sub| {
        let mut job = egui::text::LayoutJob::simple(sub.to_owned(), egui::FontId::proportional(12.), t.secondary, text_width);
        job.wrap.max_rows = 2;
        painter.layout_job(job)
    });
    let block = title_galley.size().y + sub_galley.as_ref().map_or(0., |g| g.size().y + 2.);
    let top = rect.center().y - block / 2.;
    painter.galley(egui::pos2(rect.left() + 12., top), title_galley.clone(), ink);
    if let Some(galley) = sub_galley {
        painter.galley(egui::pos2(rect.left() + 12., top + title_galley.size().y + 2.), galley, t.secondary);
    }
    if selected {
        paint_icon(painter, Icon::Check, egui::pos2(rect.right() - 18., rect.center().y), t.accent);
    }
    painter.hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1., t.separator));
    focus_ring(painter, t, &response, rect, 8.);
    response
}
fn search_field(ui: &mut egui::Ui, search: &mut String) {
    ui.add(
        egui::TextEdit::singleline(search)
            .hint_text("Search")
            .desired_width(f32::INFINITY)
            .margin(egui::vec2(10., 7.)),
    );
}

// ---------------------------------------------------------------------------
// Picker contents (pure drawing: events come back, nothing is opened here).
// ---------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
enum PanelEvent {
    None,
    Close,
    Tab(SourceTab),
    Source(Option<(bool, u32)>),
    Device(DeviceChoice),
}
struct SourceView {
    tab: SourceTab,
    displays: Vec<SourceCard>,
    windows: Vec<SourceCard>,
    screen_ready: bool,
    system_picker: bool,
    default_selected: bool,
}
fn source_panel_ui(ui: &mut egui::Ui, t: &Tokens, view: &SourceView, search: &mut String) -> PanelEvent {
    let mut event = PanelEvent::None;
    let selected = if view.tab == SourceTab::Display { 0 } else { 1 };
    ui.horizontal(|ui| {
        if let Some(index) = segmented(ui, t, "source-tab", selected, &["Display", "Window"], 88., true) {
            event = PanelEvent::Tab(if index == 0 { SourceTab::Display } else { SourceTab::Window });
        }
    });
    if view.tab == SourceTab::Window && !view.system_picker {
        ui.add_space(4.);
        search_field(ui, search);
    }
    ui.add_space(6.);
    let footer = 52.;
    let list = ui.available_rect_before_wrap();
    let list = egui::Rect::from_min_max(list.min, egui::pos2(list.max.x, list.max.y - footer));
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        ui.set_clip_rect(list);
        if view.system_picker {
            ui.label(egui::RichText::new("Choose what to record").size(15.).color(t.text));
            ui.add_space(4.);
            ui.add(egui::Label::new(egui::RichText::new("Your desktop shows its own picker each time you record. Press Record to open it.").size(12.).color(t.secondary)).wrap());
        } else if !view.screen_ready {
            ui.add(egui::Label::new(egui::RichText::new("Screen Recording isn't allowed yet. Press Record and your system asks once.").size(12.).color(t.secondary)).wrap());
        } else {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match view.tab {
                SourceTab::Display => {
                    if picker_row(ui, t, "Main display", None, view.default_selected, true).clicked() {
                        event = PanelEvent::Source(None);
                    }
                    for card in &view.displays {
                        if picker_row(ui, t, &card.title, None, card.selected, true).clicked() {
                            event = PanelEvent::Source(Some(card.target));
                        }
                    }
                }
                SourceTab::Window => {
                    if view.windows.is_empty() {
                        ui.label(egui::RichText::new("No windows to show").size(12.).color(t.secondary));
                    }
                    let width = (ui.available_width() - 8.) / 2.;
                    for pair in view.windows.chunks(2) {
                        ui.horizontal(|ui| {
                            for card in pair {
                                if window_card(ui, t, card, width).clicked() {
                                    event = PanelEvent::Source(Some(card.target));
                                }
                            }
                        });
                    }
                }
            });
        }
    });
    let footer_rect = egui::Rect::from_min_max(egui::pos2(list.min.x, list.max.y), ui.max_rect().max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(if view.system_picker {
                    "Toolbar and camera can appear in a whole-screen recording on this system. Choose a window to keep them out."
                } else {
                    capability_line(view.tab)
                })
                .size(12.)
                .color(t.secondary),
            )
            .wrap(),
        );
    });
    event
}
/// Window card. A thumbnail would need real frames from the backend, which
/// supplies none, so every card shows the app monogram and "No preview".
fn window_card(ui: &mut egui::Ui, t: &Tokens, card: &SourceCard, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 64.), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, card.selected, format!("{}, no preview", card.title)));
    let painter = ui.painter();
    painter.rect_filled(rect, 10, if response.hovered() { shade(t.control, 0.96) } else { t.control });
    if card.selected {
        painter.rect_stroke(rect, 10, egui::Stroke::new(2., t.accent), egui::StrokeKind::Inside);
    }
    let tile = egui::Rect::from_min_size(rect.min + egui::vec2(10., 10.), egui::vec2(44., 44.));
    painter.rect_filled(tile, 8, t.hairline);
    painter.text(tile.center(), egui::Align2::CENTER_CENTER, card.monogram, egui::FontId::proportional(17.), t.secondary);
    let text_left = tile.right() + 10.;
    let max = rect.right() - text_left - if card.selected { 28. } else { 10. };
    let title = truncated_galley(painter, &card.title, egui::FontId::proportional(13.), t.text, max);
    painter.galley(egui::pos2(text_left, rect.top() + 14.), title, t.text);
    painter.text(egui::pos2(text_left, rect.top() + 36.), egui::Align2::LEFT_TOP, "No preview", egui::FontId::proportional(11.), t.tertiary);
    if card.selected {
        paint_icon(painter, Icon::Check, egui::pos2(rect.right() - 16., rect.top() + 20.), t.accent);
    }
    focus_ring(painter, t, &response, rect, 10.);
    response
}
struct DeviceView<'a> {
    camera: bool,
    model: &'a DeviceModel,
    /// Why there is no list yet, if there isn't one.
    notice: Option<String>,
    settings_hint: Option<ToastAction>,
}
fn device_panel_ui(ui: &mut egui::Ui, t: &Tokens, view: &DeviceView, search: &mut String) -> PanelEvent {
    let mut event = PanelEvent::None;
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(64., 28.), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Back"));
        if response.hovered() {
            ui.painter().rect_filled(rect, 8, t.control);
        }
        paint_icon(ui.painter(), Icon::Back, rect.left_center() + egui::vec2(12., 0.), t.text);
        ui.painter().text(rect.left_center() + egui::vec2(24., 0.), egui::Align2::LEFT_CENTER, "Back", egui::FontId::proportional(13.), t.text);
        focus_ring(ui.painter(), t, &response, rect, 8.);
        if response.clicked() {
            event = PanelEvent::Close;
        }
        ui.label(egui::RichText::new(if view.camera { "Camera" } else { "Microphone" }).size(15.).color(t.text));
    });
    ui.add_space(4.);
    search_field(ui, search);
    ui.add_space(6.);
    if let Some(notice) = &view.notice {
        ui.add(egui::Label::new(egui::RichText::new(notice).size(12.).color(t.secondary)).wrap());
        if view.settings_hint.is_some() {
            ui.add_space(8.);
            if button(ui, t, "Open Settings", 0., false, true).clicked() {
                let _ = permissions::input_settings(view.camera);
            }
        }
        return event;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for row in &view.model.rows {
            let response = picker_row(ui, t, &row.title, row.subtitle.as_deref(), row.selected, row.enabled);
            if response.clicked() {
                if let Some(choice) = &row.choice {
                    event = PanelEvent::Device(choice.clone());
                }
            }
        }
        if let Some(row) = &view.model.remembered {
            ui.add_space(10.);
            ui.label(egui::RichText::new("Remembered").size(11.).color(t.tertiary));
            picker_row(ui, t, &row.title, row.subtitle.as_deref(), false, false);
        }
    });
    event
}

// ---------------------------------------------------------------------------
// Bubble hover controls: shape glyphs and × in a dark capsule, laid out so the
// capsule stays inside even the smallest circle.
// ---------------------------------------------------------------------------
const CAPSULE_BUTTON: f32 = 20.;
/// Top inset of the hover controls, away from the face: a circle needs 17 % of its height so the
/// capsule's corners stay inside the curve, every other shape 8 pt.
fn capsule_inset(shape: u8, bounds: egui::Rect) -> f32 {
    if shape == SHAPE_CIRCLE {
        bounds.height() * 0.17
    } else {
        8.
    }
}
/// A dark capsule of `count` buttons, `row` rows below the top inset (row 0 is the top edge).
fn capsule_layout(bounds: egui::Rect, count: usize, row: usize, inset: f32) -> (egui::Rect, Vec<egui::Rect>) {
    // 20 pt buttons; the narrowest bubble (a small portrait, 90 pt wide) shrinks them just enough to fit inside it.
    let button = CAPSULE_BUTTON.min(((bounds.width() - 4. - 8. - (count as f32 - 1.) * 2.) / count as f32).floor());
    let width = count as f32 * button + (count as f32 - 1.) * 2. + 8.;
    let height = button + 8.;
    let y = bounds.min.y + inset + row as f32 * (height + 4.);
    let capsule = egui::Rect::from_min_size(egui::pos2(bounds.center().x - width / 2., y), egui::vec2(width, height));
    let buttons = (0..count)
        .map(|i| {
            egui::Rect::from_min_size(
                capsule.min + egui::vec2(4. + i as f32 * (button + 2.), 4.),
                egui::vec2(button, button),
            )
        })
        .collect();
    (capsule, buttons)
}
/// Where capture is controlled, in words. With no tray, Home is the control surface and says so.
fn controls_words(tray: bool, mac: bool) -> (&'static str, &'static str) {
    match (tray, mac) {
        (true, true) => ("Menu bar", "Record, Stop, sources and devices are in the OpenRecorder menu-bar icon. Home has the same controls."),
        (true, false) => ("Tray", "Record, Stop, sources and devices are in the OpenRecorder tray icon. Home has the same controls."),
        (false, _) => ("Home", "No tray on this desktop — capture controls are here in Home."),
    }
}
/// Where the 140 pt countdown HUD goes: the middle of the display being recorded, origin included; with no display
/// list, the middle of the reported monitor.
fn hud_position(display: Option<egui::Rect>, reported: Option<egui::Vec2>) -> egui::Pos2 {
    let region = display.or_else(|| reported.map(|size| egui::Rect::from_min_size(egui::Pos2::ZERO, size)));
    region.map(|r| egui::pos2(r.center().x - 70., r.center().y - 70.)).unwrap_or(egui::pos2(200., 200.))
}
/// A fresh name for this launch of the app: 128 bits that differ on every start. Controls made for an earlier
/// launch name an earlier value and are refused, so nothing delivered late can start a recording after a restart.
fn new_launch_nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = |salt: u64| {
        // `RandomState` is seeded by the operating system; the clock and process id are mixed in as well.
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(salt ^ u64::from(std::process::id()) ^ super::connection::now_ms());
        hasher.finish()
    };
    format!("{:016x}{:016x}", part(1), part(2))
}
/// Whether the old floating toolbar is wanted: only the explicit development switch. Production launches pass
/// `--desktop --config <file>` (an earlier build passed `--toolbar --config <file>`), and neither selects it.
fn toolbar_requested(args: &[String], dev_env: Option<&str>) -> bool {
    args.iter().any(|a| a == "--dev-toolbar") || dev_env == Some("1")
}
/// What a take starts with, fixed at its start.
struct PreparedTake {
    options: super::capture::RecordOptions,
    /// The capability key of the display the options name; the same value is what the capture writes to its metadata.
    source: String,
}
/// The display a take will record and how it is named, from a **fresh** reading of the system's displays. The
/// default source becomes the main display of this very moment, named explicitly so the capture adapter cannot pick
/// a different one a moment later; a chosen display that is no longer there is an error, never a substitute. Where
/// the system reports no main display (or a portal chooses per take) the target is left to the adapter and the
/// take's source is "unresolved", which never shares or earns a qualification.
struct TakeSource {
    selection: Option<(bool, u32)>,
    key: String,
}
fn resolve_take_source(portal: bool, selection: Option<(bool, u32)>, displays: &[u32], main: Option<u32>) -> Result<TakeSource, String> {
    if portal {
        return Ok(TakeSource { selection, key: "unresolved".into() });
    }
    match selection {
        Some((true, _)) => Ok(TakeSource { selection, key: "window".into() }),
        Some((false, id)) => {
            // Where the system lists no displays at all (it can't be asked) the choice stands as made.
            if !displays.is_empty() && !displays.contains(&id) {
                return Err("The display you chose isn't available".into());
            }
            Ok(TakeSource { selection, key: format!("display:{id}") })
        }
        None => Ok(match main {
            Some(id) => TakeSource { selection: Some((false, id)), key: format!("display:{id}") },
            None => TakeSource { selection: None, key: "unresolved".into() },
        }),
    }
}
/// Who a take records, for the purpose of reusing measurements. Only an identity the system actually reports counts:
/// a chosen display's id, or the main display's id when the default is used. A window changes under the recorder,
/// and on Linux the desktop portal picks the real source (a display or any window) on every take without telling
/// this app which, so those are "window" / "unresolved" and never share a measurement. Nothing is invented: an
/// unknown main display is "unresolved", not a made-up name.
fn source_identity(portal: bool, selection: Option<(bool, u32)>, main_display: Option<u32>) -> String {
    if portal {
        return "unresolved".into();
    }
    match selection {
        Some((true, _)) => "window".into(),
        Some((false, id)) => format!("display:{id}"),
        None => main_display.map_or_else(|| "unresolved".into(), |id| format!("display:{id}")),
    }
}
fn shape_from_name(name: &str) -> u8 {
    match name {
        "rounded" => SHAPE_ROUNDED,
        "square" => SHAPE_SQUARE,
        "wide" => SHAPE_WIDE,
        "portrait" => SHAPE_PORTRAIT,
        _ => SHAPE_CIRCLE,
    }
}
/// The line under Frame rate. Unknown is said plainly and never blocks the choice.
fn fps_hint(status: super::capability::Status, fps: u32) -> Option<&'static str> {
    use super::capability::Status;
    if fps != 60 {
        return None;
    }
    Some(match status {
        Status::Unknown => "Not measured for this setup yet. You can still choose it.",
        Status::Qualified => "A recording of this setup on this computer kept up at 60 fps.",
        Status::FailedLast => "The last 60 fps recording of this setup couldn't keep up. You can still choose it.",
        Status::Unavailable => "The video encoder didn't accept 60 fps in the last test. You can still choose it.",
    })
}
fn next_shape(shape: u8) -> u8 {
    match shape {
        SHAPE_CIRCLE => SHAPE_ROUNDED,
        SHAPE_ROUNDED => SHAPE_WIDE,
        SHAPE_WIDE => SHAPE_PORTRAIT,
        _ => SHAPE_CIRCLE,
    }
}
fn paint_shape_glyph(painter: &egui::Painter, shape: u8, centre: egui::Pos2, ink: Color32) {
    let stroke = egui::Stroke::new(1.4, ink);
    match shape {
        SHAPE_CIRCLE => {
            painter.circle_stroke(centre, 6., stroke);
        }
        SHAPE_ROUNDED => {
            painter.rect_stroke(egui::Rect::from_center_size(centre, egui::vec2(12., 12.)), 4, stroke, egui::StrokeKind::Inside);
        }
        SHAPE_PORTRAIT => {
            painter.rect_stroke(egui::Rect::from_center_size(centre, egui::vec2(9., 15.)), 2, stroke, egui::StrokeKind::Inside);
        }
        _ => {
            painter.rect_stroke(egui::Rect::from_center_size(centre, egui::vec2(15., 9.)), 2, stroke, egui::StrokeKind::Inside);
        }
    }
}

/// Why a device list can't be shown yet. Listing never asks for access.
/// Names are only a label for an identifier that is no longer listed: a camera
/// that was unplugged keeps its last known name so the message can say which.
fn merge_camera_names(known: &mut Vec<super::devices::CameraDevice>, listed: &[super::devices::CameraDevice]) {
    for device in listed {
        match known.iter_mut().find(|d| d.id == device.id) {
            Some(existing) => existing.name = device.name.clone(),
            None => known.push(device.clone()),
        }
    }
}
fn device_notice(camera: bool, grant: Grant, loading: bool, error: Option<&str>) -> (Option<String>, Option<ToastAction>) {
    let noun = if camera { "Camera" } else { "Microphone" };
    let settings = if camera { ToastAction::OpenCameraSettings } else { ToastAction::OpenMicSettings };
    match grant {
        Grant::Denied => (
            Some(format!("{noun} access is off. Turn it on in System Settings to choose a device.")),
            Some(settings),
        ),
        Grant::Restricted => (Some(format!("{noun} access is restricted on this computer.")), None),
        Grant::NotAsked => (
            Some(format!("Click {} on the toolbar to allow access first. Opening this list never asks for it.", if camera { "Camera" } else { "Mic" })),
            None,
        ),
        Grant::Allowed | Grant::SystemManaged => {
            if let Some(error) = error {
                (Some(error.to_owned()), None)
            } else if loading {
                (Some(format!("Finding {}…", if camera { "cameras" } else { "microphones" })), None)
            } else {
                (None, None)
            }
        }
    }
}
/// Root-window Space starts Record only when nothing else wants the key.
fn space_starts_record(root_focused: bool, text_editing: bool, widget_focused: bool) -> bool {
    root_focused && !text_editing && !widget_focused
}

impl State {
    fn grants(&self) -> GrantSnapshot {
        self.grants.unwrap_or(GrantSnapshot {
            screen: self.screen_ready,
            camera: Grant::SystemManaged,
            microphone: Grant::SystemManaged,
        })
    }
    /// Passive status read, at most once a second while the toolbar is visible.
    fn refresh_grants(&mut self) {
        let grants = (self.os.snapshot)();
        self.grants = Some(grants);
        self.grants_checked = Some(Instant::now());
        if !self.benchmark {
            self.screen_ready = grants.screen;
        }
        if grants.camera.can_open()
            && matches!(
                self.camera_attention,
                Some(InputProblem::AccessOff { .. } | InputProblem::Restricted { .. } | InputProblem::NeedsAccess { .. })
            )
        {
            self.camera_attention = None;
        }
        if grants.microphone.can_open()
            && matches!(
                self.microphone_attention,
                Some(InputProblem::AccessOff { .. } | InputProblem::Restricted { .. } | InputProblem::NeedsAccess { .. })
            )
        {
            self.microphone_attention = None;
        }
    }
    fn source_label(&self) -> (String, Icon) {
        let found = self.options.selection.and_then(|selection| {
            self.targets.iter().find(|(w, id, _)| (*w, *id) == selection)
        });
        match found {
            Some((true, _, name)) => (name.clone(), Icon::Window),
            Some((false, _, name)) => (name.clone(), Icon::Display),
            None if self.options.selection.is_some_and(|(w, _)| w) => ("Window".into(), Icon::Window),
            None if cfg!(target_os = "linux") => ("Desktop picker".into(), Icon::Display),
            // The default is the first display the system lists, called by its own name; with no listing, just "Display".
            None => match self.targets.iter().find(|(window, ..)| !*window) {
                Some((_, _, name)) if !name.trim().is_empty() => (name.clone(), Icon::Display),
                _ => ("Display".into(), Icon::Display),
            },
        }
    }
    /// The toolbar's global rectangle and the monitor it is on, if that is known.
    fn toolbar_rect(&self, ctx: &egui::Context) -> (egui::Rect, Option<egui::Rect>) {
        let reported = ctx.input(|i| i.viewport().monitor_size);
        if !self.toolbar_mode {
            // No toolbar: notices hang from the top centre of the display the recording uses.
            let monitors = self.space.lock().map(|s| s.monitors.clone()).unwrap_or_default();
            let display = self
                .space
                .lock()
                .ok()
                .and_then(|s| s.capture)
                .or_else(|| monitors.first().copied())
                .or_else(|| reported.map(|size| egui::Rect::from_min_size(egui::Pos2::ZERO, size)))
                .unwrap_or_else(|| egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440., 900.)));
            let rect = egui::Rect::from_min_size(egui::pos2(display.center().x - TOOLBAR_W / 2., display.min.y + 24.), egui::vec2(TOOLBAR_W, 0.));
            return (rect, Some(display));
        }
        let origin = self.toolbar_pos.unwrap_or_else(|| default_toolbar_position(reported.unwrap_or(egui::vec2(1440., 900.))));
        let rect = egui::Rect::from_min_size(origin, egui::vec2(TOOLBAR_W, TOOLBAR_H));
        let monitors = self.space.lock().map(|s| s.monitors.clone()).unwrap_or_default();
        (rect, monitor_for(rect, &monitors, reported))
    }
    /// Display and selected-window rectangles are cached and refreshed from the main
    /// thread: every 2 s for displays, every second while a window is the source (it
    /// can move). One bounded query by the selected window's id, never per frame.
    fn refresh_space(&mut self) {
        let window = self.options.selection.filter(|(is_window, _)| *is_window).map(|(_, id)| id);
        let interval = if window.is_some() { Duration::from_secs(1) } else { Duration::from_secs(2) };
        if self.space_checked.is_some_and(|t| t.elapsed() < interval) {
            return;
        }
        self.space_checked = Some(Instant::now());
        let (displays, main) = (self.os.displays)();
        self.main_display = main;
        let bounds = window.and_then(|id| (self.os.window_bounds)(id));
        let space = space_for(&displays, main, self.options.selection, bounds);
        if let Ok(mut shared) = self.space.lock() {
            *shared = space;
        }
    }
    // ----- pickers -------------------------------------------------------
    fn open_panel(&mut self, kind: PanelKind) {
        if self.locked() || self.hidden {
            return;
        }
        let tab = if matches!(self.options.selection, Some((true, _))) { SourceTab::Window } else { SourceTab::Display };
        self.panel = Some(Panel { kind, tab, search: String::new(), was_focused: false, size: egui::Vec2::ZERO });
        match kind {
            PanelKind::Camera => self.start_camera_listing(),
            PanelKind::Microphone => self.start_microphone_listing(),
            PanelKind::Source => {}
        }
    }
    /// A toolbar press: a second press on an open panel closes it, and a press
    /// right after an outside click closed it doesn't reopen it.
    fn toggle_panel(&mut self, kind: PanelKind) {
        if let Some((at, closed)) = self.panel_closed.take() {
            if closed == kind && at.elapsed() < Duration::from_millis(300) {
                return;
            }
        }
        if self.panel.as_ref().is_some_and(|p| p.kind == kind) {
            self.panel = None;
        } else {
            self.open_panel(kind);
        }
    }
    fn start_camera_listing(&mut self) {
        self.device_error = None;
        if self.camera_discovery.is_none() && self.grants().camera.can_open() {
            let list = self.os.list_cameras;
            self.camera_discovery = Some(thread::spawn(move || list()));
        }
    }
    fn start_microphone_listing(&mut self) {
        self.device_error = None;
        if self.microphone_discovery.is_none() && self.grants().microphone.can_open() {
            let list = self.os.list_microphones;
            self.microphone_discovery = Some(thread::spawn(move || list()));
        }
    }
    fn poll_discovery(&mut self) {
        if self.camera_discovery.as_ref().is_some_and(|h| h.is_finished()) {
            match self.camera_discovery.take().unwrap().join() {
                Ok(Ok(choices)) => {
                    merge_camera_names(&mut self.camera_names, &choices);
                    self.camera_choices = Some(choices);
                }
                Ok(Err(e)) if e.contains("No camera was found") => self.camera_choices = Some(vec![]),
                Ok(Err(e)) => self.device_error = Some(e),
                Err(_) => self.device_error = Some("Camera discovery was interrupted".into()),
            }
        }
        if self.microphone_discovery.as_ref().is_some_and(|h| h.is_finished()) {
            match self.microphone_discovery.take().unwrap().join() {
                Ok(Ok(choices)) => self.microphone_choices = Some(choices),
                Ok(Err(e)) => self.device_error = Some(e),
                Err(_) => self.device_error = Some("Microphone discovery was interrupted".into()),
            }
        }
    }
    /// The person's pick becomes the preference; nothing is substituted. A pick
    /// arms the input (hollow ring) without opening it.
    fn apply_device_choice(&mut self, camera: bool, choice: DeviceChoice) {
        if self.locked() {
            return;
        }
        match choice {
            DeviceChoice::Off => return self.turn_off(camera),
            DeviceChoice::SystemDefault | DeviceChoice::Device(_) => {
                let id = match choice {
                    DeviceChoice::Device(id) => Some(id),
                    _ => None,
                };
                if camera {
                    self.camera = None;
                    self.camera_permission = None;
                    self.camera_device_id = id;
                    self.camera_intent = true;
                    self.camera_attention = None;
                } else {
                    self.microphone = None;
                    self.microphone_permission = None;
                    self.microphone_device_id = id;
                    self.microphone_intent = true;
                    self.microphone_attention = None;
                }
                self.persist_intent();
            }
        }
    }
    fn panels(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.panel.as_ref().map(|p| p.kind) else {
            close_present_viewport(ctx, vp(VP_SOURCE));
            close_present_viewport(ctx, vp(VP_DEVICE));
            return;
        };
        if self.hidden || self.locked() {
            self.panel = None;
            close_present_viewport(ctx, vp(VP_SOURCE));
            close_present_viewport(ctx, vp(VP_DEVICE));
            return;
        }
        let (id_name, other) = if kind == PanelKind::Source { (VP_SOURCE, VP_DEVICE) } else { (VP_DEVICE, VP_SOURCE) };
        close_present_viewport(ctx, vp(other));
        let t = tokens_for(ctx);
        let now = Instant::now();
        let (tab, mut search, mut seen_focus) = {
            let panel = self.panel.as_ref().unwrap();
            (panel.tab, panel.search.clone(), panel.was_focused)
        };
        // Source rows: the capture API is asked only after Screen Recording is allowed.
        #[cfg(not(target_os = "linux"))]
        if kind == PanelKind::Source && self.screen_ready && self.targets_checked.is_none_or(|last| last.elapsed() >= Duration::from_secs(1)) {
            self.targets_checked = Some(now);
            let own_pid = std::process::id();
            self.targets = scap::get_all_targets()
                .into_iter()
                .filter_map(|t| match t {
                    // Own windows are hidden by owning process; a foreign page that merely has a similar
                    // title stays selectable, and unknown ownership is never hidden.
                    scap::Target::Window(w) if super::capture::window_ownership(w.owner_pid, own_pid) != super::capture::Ownership::Own => Some((true, w.id, w.title)),
                    scap::Target::Display(d) => Some((false, d.id, d.title)),
                    _ => None,
                })
                .collect();
        }
        let source_view = SourceView {
            tab,
            displays: source_cards(&self.targets, false, self.options.selection, ""),
            windows: source_cards(&self.targets, true, self.options.selection, &search),
            screen_ready: self.screen_ready,
            system_picker: cfg!(target_os = "linux"),
            default_selected: self.options.selection.is_none(),
        };
        let camera = kind == PanelKind::Camera;
        let grant = if camera { self.grants().camera } else { self.grants().microphone };
        let (preference, intent, remembered_name, listed, devices) = if camera {
            let devices: Vec<(String, String, bool)> = self.camera_choices.as_ref().map(|list| {
                list.iter().map(|d| (d.id.clone(), d.name.clone(), super::devices::shares_name(list, d))).collect()
            }).unwrap_or_default();
            let name = self.camera_device_id.as_ref().and_then(|id| self.camera_names.iter().find(|d| &d.id == id).map(|d| d.name.clone()));
            (self.camera_device_id.clone(), self.camera_intent, name, self.camera_choices.is_some(), devices)
        } else {
            let devices: Vec<(String, String, bool)> = self.microphone_choices.as_ref().map(|list| {
                list.iter().map(|m| (m.name.clone(), m.name.clone(), m.duplicate)).collect()
            }).unwrap_or_default();
            (self.microphone_device_id.clone(), self.microphone_intent, self.microphone_device_id.clone(), self.microphone_choices.is_some(), devices)
        };
        let model = device_model(camera, &devices, preference.as_deref(), remembered_name.as_deref(), listed, intent, &search);
        let loading = if camera { self.camera_discovery.is_some() } else { self.microphone_discovery.is_some() } || (!listed && self.device_error.is_none());
        let (notice, hint) = device_notice(camera, grant, loading && !listed, self.device_error.as_deref());
        let device_view = DeviceView { camera, model: &model, notice, settings_hint: hint };
        let size = match kind {
            PanelKind::Source => source_panel_size(tab, source_view.displays.len() + 1),
            _ => DEVICE_PANEL,
        };
        let (toolbar, monitor) = self.toolbar_rect(ctx);
        let slot_kind = match kind {
            PanelKind::Source => Slot::Source,
            PanelKind::Camera => Slot::Camera,
            PanelKind::Microphone => Slot::Mic,
        };
        let slot = slot_rect(toolbar.min, slot_kind);
        let position = match (self.toolbar_mode, monitor) {
            (false, Some(display)) => display.center() - size / 2.,
            _ => anchor_panel(toolbar, slot, size, monitor, false),
        };
        let id = vp(id_name);
        let known_size = self.panel.as_ref().map(|p| p.size).unwrap_or_default();
        if known_size != size && ctx.input(|i| i.raw.viewports.contains_key(&id)) {
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::OuterPosition(position));
        }
        let mut event = PanelEvent::None;
        let mut lost_focus = false;
        let title = match kind {
            PanelKind::Source => "OpenRecorder Source",
            PanelKind::Camera => "OpenRecorder Camera Choice",
            PanelKind::Microphone => "OpenRecorder Microphone Choice",
        };
        ctx.show_viewport_immediate(
            id,
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size(size)
                .with_position(position)
                .with_decorations(false)
                .with_resizable(false)
                .with_transparent(true)
                .with_taskbar(false)
                .with_always_on_top(),
            |ctx, _| {
                let t = tokens_for(ctx);
                match ctx.input(|i| i.viewport().focused) {
                    Some(true) => seen_focus = true,
                    Some(false) if seen_focus => lost_focus = true,
                    _ => {}
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()) {
                    event = PanelEvent::Close;
                }
                egui::CentralPanel::default().frame(panel_frame(&t)).show(ctx, |ui| {
                    let next = match kind {
                        PanelKind::Source => source_panel_ui(ui, &t, &source_view, &mut search),
                        _ => device_panel_ui(ui, &t, &device_view, &mut search),
                    };
                    if next != PanelEvent::None {
                        event = next;
                    }
                });
            },
        );
        let _ = t;
        if let Some(panel) = &mut self.panel {
            panel.search = search;
            panel.was_focused = seen_focus;
            panel.size = size;
        }
        match event {
            PanelEvent::Tab(next) => {
                if let Some(panel) = &mut self.panel {
                    panel.tab = next;
                }
            }
            PanelEvent::Source(selection) => {
                self.options.selection = selection;
                self.space_checked = None;
                self.close_panel(ctx, kind, now);
            }
            PanelEvent::Device(choice) => {
                self.apply_device_choice(camera, choice);
                self.close_panel(ctx, kind, now);
            }
            PanelEvent::Close => self.close_panel(ctx, kind, now),
            PanelEvent::None if lost_focus => self.close_panel(ctx, kind, now),
            PanelEvent::None => {}
        }
        if kind == PanelKind::Source {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
    fn close_panel(&mut self, ctx: &egui::Context, kind: PanelKind, now: Instant) {
        self.panel = None;
        self.panel_closed = Some((now, kind));
        close_present_viewport(ctx, vp(VP_SOURCE));
        close_present_viewport(ctx, vp(VP_DEVICE));
    }

    // ----- toast ---------------------------------------------------------
    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some(mut toast) = self.toast.take() else {
            close_present_viewport(ctx, vp(VP_TOAST));
            return;
        };
        let now = Instant::now();
        if self.hidden || !toast.tick(now, self.toast_hovered) {
            close_present_viewport(ctx, vp(VP_TOAST));
            return;
        }
        let size = toast.pill_size();
        let (toolbar, monitor) = self.toolbar_rect(ctx);
        let slot = egui::Rect::from_min_size(egui::pos2(toolbar.center().x - size.x / 2., toolbar.min.y), egui::vec2(size.x, TOOLBAR_H));
        let position = anchor_panel(toolbar, slot, size, monitor, true);
        let (mut clicked, mut dismiss, mut hovered, mut details) = (None, false, false, false);
        let dark = ctx.style().visuals.dark_mode;
        ctx.show_viewport_immediate(
            vp(VP_TOAST),
            egui::ViewportBuilder::default()
                .with_title("OpenRecorder Notice")
                .with_inner_size(size)
                .with_position(position)
                .with_decorations(false)
                .with_resizable(false)
                .with_transparent(true)
                .with_taskbar(false)
                .with_always_on_top()
                .with_active(false),
            |ctx, _| {
                hovered = ctx.input(|i| i.pointer.has_pointer());
                let t = tokens(dark);
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let rect = ui.max_rect();
                    let ink = rgb(0xF5F5F7);
                    let link = rgb(0x8CC2FF);
                    let radius = if toast.details_open { 16. } else { rect.height() / 2. };
                    ui.painter().rect_filled(rect, radius, t.toast);
                    let row = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 40.));
                    let mut x = row.right() - 14.;
                    let close = egui::Rect::from_center_size(egui::pos2(x - 8., row.center().y), egui::vec2(24., 24.));
                    let response = ui.interact(close, ui.id().with("toast-close"), egui::Sense::click());
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Dismiss"));
                    paint_icon(ui.painter(), Icon::Close, close.center(), ink);
                    if response.clicked() {
                        dismiss = true;
                    }
                    x = close.left() - 8.;
                    let mut actions: Vec<(String, u8)> = Vec::new();
                    if let Some(action) = toast.action {
                        actions.push((action.label().to_owned(), 0));
                    }
                    if let Some(action) = toast.secondary {
                        actions.push((action.label().to_owned(), 1));
                    }
                    if toast.details.is_some() {
                        actions.push(((if toast.details_open { "Hide details" } else { "Details" }).to_owned(), 2));
                    }
                    for (label, which) in actions.iter().rev() {
                        let galley = text_galley(ui, label, 13.);
                        let r = egui::Rect::from_min_size(egui::pos2(x - galley.size().x, row.center().y - galley.size().y / 2.), galley.size());
                        let response = ui.interact(r.expand2(egui::vec2(4., 8.)), ui.id().with(("toast-action", *which)), egui::Sense::click());
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.as_str()));
                        ui.painter().galley(r.min, galley, link);
                        if response.clicked() {
                            match which {
                                0 => clicked = toast.action,
                                1 => clicked = toast.secondary,
                                _ => details = true,
                            }
                        }
                        x = r.left() - 14.;
                    }
                    let text = truncated_galley(ui.painter(), &toast.text, egui::FontId::proportional(13.), ink, (x - row.left() - 16.).max(40.));
                    ui.painter().galley(egui::pos2(row.left() + 16., row.center().y - text.size().y / 2.), text, ink);
                    if let (true, Some(raw)) = (toast.details_open, &toast.details) {
                        let body = egui::Rect::from_min_max(egui::pos2(rect.left() + 16., row.bottom()), egui::pos2(rect.right() - 16., rect.bottom() - 8.));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                ui.add(egui::Label::new(egui::RichText::new(raw).size(11.).color(rgb(0xC7C7CC))).wrap().selectable(true));
                            });
                        });
                    }
                });
            },
        );
        self.toast_hovered = hovered;
        if details {
            toast.details_open = !toast.details_open;
        }
        if dismiss || clicked.is_some() {
            close_present_viewport(ctx, vp(VP_TOAST));
            if let Some(action) = clicked {
                self.toast_action(action);
            }
            return;
        }
        let remaining = toast.expires.saturating_duration_since(now);
        if self.toast.is_none() {
            self.toast = Some(toast);
        }
        ctx.request_repaint_after(remaining.min(Duration::from_millis(250)));
    }

    // ----- toolbar -------------------------------------------------------
    fn toolbar(&mut self, ctx: &egui::Context) {
        let t = tokens_for(ctx);
        let dark = ctx.style().visuals.dark_mode;
        let now = Instant::now();
        let face = self.face(now);
        let locked = self.locked() || face.locks_inputs();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.toolbar).corner_radius(14))
            .show(ctx, |ui| {
                let area = ctx.content_rect();
                let origin = area.min;
                ui.painter().rect_stroke(area, 14, egui::Stroke::new(1., t.toolbar_edge), egui::StrokeKind::Inside);
                // Drag from any pixel that isn't a control.
                let controls = control_rects(origin);
                let background = ui.interact(area, ui.id().with("toolbar-drag"), egui::Sense::drag());
                if background.drag_started()
                    && ctx.input(|i| i.pointer.press_origin().is_some_and(|p| !controls.iter().any(|r| r.contains(p))))
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let grip = slot_rect(origin, Slot::Grip);
                for y in 0..3 {
                    for x in 0..2 {
                        ui.painter().circle_filled(grip.center() + egui::vec2((x as f32 - 0.5) * 5., (y as f32 - 1.) * 5.), 1.1, t.tertiary);
                    }
                }
                let mut bounds: Vec<(Slot, egui::Rect)> = Vec::new();
                let open_slot = |ui: &mut egui::Ui, bounds: &mut Vec<(Slot, egui::Rect)>, slot: Slot, enabled: bool, name: &str| {
                    let rect = slot_rect(origin, slot);
                    let response = ui.interact(rect, ui.id().with(("slot", name)), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
                    if enabled && response.hovered() {
                        ui.painter().rect_filled(rect, 14, t.control);
                    }
                    focus_ring(ui.painter(), &t, &response, rect, 14.);
                    bounds.push((slot, rect));
                    (rect, response)
                };
                // Library: a plain black circle.
                let (rect, response) = open_slot(ui, &mut bounds, Slot::Library, true, "Open Library");
                paint_library_mark(ui.painter(), rect.center(), dark);
                if response.on_hover_text("Open Library").clicked() {
                    self.open_library();
                }
                // Source.
                let (rect, response) = open_slot(ui, &mut bounds, Slot::Source, !locked, &format!("Source, {}", self.source_label().0));
                let (label, icon) = self.source_label();
                let ink = if locked { t.tertiary } else { t.text };
                paint_icon(ui.painter(), icon, rect.left_center() + egui::vec2(16., 0.), ink);
                let galley = truncated_galley(ui.painter(), &label, egui::FontId::proportional(13.), ink, rect.width() - 30. - 26.);
                ui.painter().galley(rect.left_center() + egui::vec2(30., -galley.size().y / 2.), galley, ink);
                paint_icon(ui.painter(), Icon::Chevron, rect.right_center() - egui::vec2(14., 0.), ink);
                let source_tip = if locked { "Can't change during a recording" } else { "Choose what to record" };
                if response.on_hover_text(source_tip).clicked() {
                    self.toggle_panel(PanelKind::Source);
                }
                // Camera and Mic: marks show intent and truth; only a person's press activates.
                for camera in [true, false] {
                    let slot = if camera { Slot::Camera } else { Slot::Mic };
                    let name = self.input_label(camera);
                    let (rect, response) = open_slot(ui, &mut bounds, slot, !locked, &name);
                    let mark = self.mark(camera);
                    let ink = if locked { t.tertiary } else { t.text };
                    paint_icon(ui.painter(), if camera { Icon::Camera } else { Icon::Mic }, rect.center(), ink);
                    let corner = rect.right_top() + egui::vec2(-6., 6.);
                    match mark {
                        Mark::None => {}
                        Mark::Armed => {
                            ui.painter().circle_stroke(corner, 3., egui::Stroke::new(1.4, if locked { t.tertiary } else { t.secondary }));
                        }
                        Mark::Live => {
                            ui.painter().circle_filled(corner, 3.5, t.live.gamma_multiply(if locked { 0.5 } else { 1. }));
                        }
                        Mark::Attention => {
                            ui.painter().circle_filled(corner, 3.5, t.text);
                        }
                    }
                    if !camera {
                        if let Some(shared) = self.microphone.as_ref().and_then(MicSource::shared) {
                            if self.meter_sample.is_none_or(|last| last.elapsed() >= Duration::from_millis(67)) {
                                self.meter_level = f32::from_bits(shared.level.load(Ordering::Relaxed));
                                self.meter_sample = Some(Instant::now());
                            }
                            let line = egui::Rect::from_min_size(egui::pos2(rect.center().x - 8., rect.bottom() - 5.), egui::vec2(16., 2.));
                            ui.painter().rect_filled(line, 1, t.hairline);
                            ui.painter().rect_filled(egui::Rect::from_min_size(line.min, egui::vec2(16. * (self.meter_level * 4.).clamp(0., 1.), 2.)), 1, t.live);
                        }
                    }
                    let tip = if locked {
                        "Can't change during a recording".to_owned()
                    } else {
                        format!("{name} · {}", match mark {
                            Mark::Live => "click to turn off",
                            Mark::Armed => "click to turn on · right-click to turn off",
                            _ => "click to turn on",
                        })
                    };
                    let (clicked, secondary) = (response.clicked(), response.secondary_clicked());
                    response.on_hover_text(tip);
                    if clicked && !locked {
                        let fresh = (self.os.snapshot)();
                        self.grants = Some(fresh);
                        self.click_input(camera, if camera { fresh.camera } else { fresh.microphone });
                    }
                    if secondary && !locked {
                        self.turn_off(camera);
                    }
                }
                let divider = slot_rect(origin, Slot::Divider);
                ui.painter().vline(divider.center().x, divider.center().y - 8. ..=divider.center().y + 8., egui::Stroke::new(1., t.hairline));
                // Record: a compact pill whose face is derived from the real phase.
                let record = slot_rect(origin, Slot::Record);
                let pressable = !matches!(face, Face::Saved) && (face != Face::Saving || self.completion.is_some());
                let response = ui.interact(record, ui.id().with("slot-record"), if pressable { egui::Sense::click() } else { egui::Sense::hover() });
                let name = face.accessible_name();
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, pressable, name.clone()));
                bounds.push((Slot::Record, record));
                let (fill, ink) = match face {
                    Face::Ready | Face::Recording(_) => (t.record, Color32::WHITE),
                    Face::Saved => (t.control, t.live),
                    Face::Unavailable => (t.control, t.tertiary),
                    _ => (t.control, t.text),
                };
                let fill = if response.hovered() && pressable { shade(fill, 0.92) } else { fill };
                ui.painter().rect_filled(record, 14, fill);
                let (glyph, text) = face.parts();
                let galley = text_galley(ui, &text, 13.);
                let glyph_w = if glyph == Glyph::None { 0. } else { 10. + 6. };
                let start = record.center().x - (glyph_w + galley.size().x) / 2.;
                let mid = egui::pos2(start + 5., record.center().y);
                match glyph {
                    Glyph::Dot => {
                        ui.painter().circle_filled(mid, 4.5, ink);
                    }
                    Glyph::Square => {
                        ui.painter().rect_filled(egui::Rect::from_center_size(mid, egui::vec2(9., 9.)), 2, ink);
                    }
                    Glyph::Check => paint_icon(ui.painter(), Icon::Check, mid, ink),
                    Glyph::None => {}
                }
                ui.painter().galley(egui::pos2(start + glyph_w, record.center().y - galley.size().y / 2.), galley, ink);
                focus_ring(ui.painter(), &t, &response, record, 14.);
                let record_tip = match face {
                    Face::Ready => "Record",
                    Face::Recording(_) => "Stop recording",
                    Face::Starting | Face::Countdown(_) => "Cancel",
                    Face::Saving if self.completion.is_some() => "Try saving again now",
                    Face::RetrySave => "Retry saving the last recording",
                    Face::Unavailable => "Finishing the last recording",
                    _ => "",
                };
                if response.on_hover_text(record_tip).clicked() {
                    if face == Face::Unavailable {
                        self.show_toast(Toast::info("Finishing the last recording…"));
                    } else {
                        self.record_action(ctx);
                    }
                }
                // Settings.
                let (rect, response) = open_slot(ui, &mut bounds, Slot::Settings, true, "Settings");
                let hole = if response.hovered() { t.control } else { t.toolbar };
                paint_cog(ui.painter(), rect.center(), t.text, hole);
                if response.on_hover_text("Settings").clicked() {
                    self.settings_open = !self.settings_open;
                    if !self.settings_open {
                        close_present_viewport(ctx, vp(VP_SETTINGS));
                    }
                }
                // Hide.
                let (rect, response) = open_slot(ui, &mut bounds, Slot::Hide, true, "Hide toolbar");
                paint_icon(ui.painter(), Icon::Close, rect.center(), t.text);
                if response.on_hover_text("Hide toolbar").clicked() {
                    self.hide(ctx);
                }
                self.toolbar_bounds = bounds;
            });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RowAction {
    None,
    Allow,
    OpenSettings,
}
/// Setup row control: Allow triggers the OS prompt directly, Open Settings once
/// it was refused. Only a never-asked grant ever offers Allow.
fn setup_row_action(grant: Grant) -> RowAction {
    match grant {
        Grant::NotAsked => RowAction::Allow,
        Grant::Denied | Grant::Restricted => RowAction::OpenSettings,
        Grant::Allowed | Grant::SystemManaged => RowAction::None,
    }
}
fn screen_row_action(has_permission: bool, already_asked: bool) -> RowAction {
    match permissions::decide_screen_request(has_permission, already_asked) {
        permissions::ScreenDecision::Allowed => RowAction::None,
        permissions::ScreenDecision::Prompt => RowAction::Allow,
        permissions::ScreenDecision::OpenSettings => RowAction::OpenSettings,
    }
}

impl State {
    // ----- settings ------------------------------------------------------
    fn settings_ui(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let before = self.settings_values();
        let mut draft = before.clone();
        let mut path = self.path_draft.clone().unwrap_or_else(|| before.root.display().to_string());
        let mut commit_path = false;
        if self.settings_status_until.is_some_and(|t| Instant::now() >= t) {
            self.settings_status.clear();
            self.settings_status_until = None;
        }
        ctx.show_viewport_immediate(
            vp(VP_SETTINGS),
            egui::ViewportBuilder::default()
                .with_title("OpenRecorder Settings")
                .with_inner_size([760., 600.])
                .with_min_inner_size([760., 560.]),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.settings_open = false;
                }
                let t = tokens_for(ctx);
                egui::SidePanel::left("settings-navigation")
                    .exact_width(200.)
                    .resizable(false)
                    .frame(egui::Frame::new().fill(t.sidebar).inner_margin(12))
                    .show(ctx, |ui| {
                        ui.add_space(8.);
                        ui.label(egui::RichText::new("Settings").size(20.).color(t.text));
                        ui.add_space(14.);
                        settings_nav(ui, &t, &mut self.settings_pane);
                    });
                egui::TopBottomPanel::bottom("settings-footer")
                    .frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(28, 12)))
                    .show(ctx, |ui| {
                        let color = if self.settings_error { t.record } else { t.live };
                        ui.label(egui::RichText::new(if self.settings_status.is_empty() { "Changes apply immediately." } else { &self.settings_status }).size(12.).color(if self.settings_status.is_empty() { t.secondary } else { color }));
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(28, 20)))
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            self.settings_contents(ui, &t, &mut draft, &mut path, &mut commit_path);
                        });
                    });
            },
        );
        if commit_path {
            draft.root = PathBuf::from(&path);
        }
        self.path_draft = Some(path);
        if before != draft {
            self.commit_settings(&draft);
            ctx.request_repaint();
            ctx.request_repaint_of(vp(VP_SETTINGS));
        }
    }
    fn settings_contents(
        &mut self,
        ui: &mut egui::Ui,
        t: &Tokens,
        draft: &mut SettingsValues,
        path: &mut String,
        commit_path: &mut bool,
    ) {
        let locked = self.locked();
        let pane = self.settings_pane;
        ui.label(egui::RichText::new(SETTINGS_PANES[pane].0).size(20.).color(t.text));
        ui.add_space(6.);
        if locked && pane < 3 {
            ui.label(egui::RichText::new(if pane == 2 { "Devices are locked during a recording. Bubble shape and size still work." } else { "These settings are locked during a recording." }).size(12.).color(t.secondary));
        }
        let (mut library, mut camera_panel, mut microphone_panel) = (false, false, false);
        match pane {
            0 => {
                settings_row(ui, t, "Appearance", None, |ui| {
                    if let Some(i) = segmented(ui, t, "theme", draft.theme as usize, &["System", "Light", "Dark"], 72., !locked) {
                        draft.theme = i as u8;
                    }
                });
                let support = if self.library_offline { "The Library service is reconnecting." } else { "Opens in your browser. Recordings, folders and sharing live there." };
                settings_row(ui, t, "Library", Some(support), |ui| {
                    library = button(ui, t, "Open Library", 0., true, true).clicked();
                });
                let (controls, controls_note) = controls_words(self.tray.as_ref().is_some_and(|t| !t.host_lost()), cfg!(target_os = "macos"));
                settings_row(ui, t, "Controls", Some(controls_note), |ui| {
                    ui.label(egui::RichText::new(controls).size(13.).color(t.text));
                });
            }
            1 => {
                settings_row(ui, t, "Save location", Some("Existing recordings stay where they are."), |ui| {
                    if button(ui, t, "Change…", 0., false, !locked).clicked() {
                        if let Some(folder) = rfd::FileDialog::new().set_title("Save OpenRecorder recordings").set_directory(&self.confirmed.root).pick_folder() {
                            *path = folder.display().to_string();
                            *commit_path = true;
                        }
                    }
                });
                ui.add_space(6.);
                ui.add_enabled_ui(!locked, |ui| {
                    let field = ui.add(egui::TextEdit::singleline(path).desired_width(f32::INFINITY).margin(egui::vec2(10., 7.)));
                    field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, !locked, "Save location"));
                    *commit_path |= field.lost_focus() || (field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                });
                settings_row(ui, t, "Countdown", Some("Esc or the Record button cancels it."), |ui| {
                    if let Some(i) = segmented(ui, t, "countdown", if draft.countdown == 0 { 0 } else { 1 }, &["Off", "3 s"], 56., !locked) {
                        draft.countdown = [0, 3][i];
                    }
                });
                settings_row(ui, t, "Keep originals", Some("Saves separate screen and camera files next to the combined MP4. Uses more disk."), |ui| {
                    switch(ui, t, "Keep originals", &mut draft.originals, !locked);
                });
                settings_row(ui, t, "Combined video", Some("Resolution"), |ui| {
                    let values = [1280, 1920, 2560];
                    let selected = values.iter().position(|v| *v == draft.quality).unwrap_or(1);
                    if let Some(i) = segmented(ui, t, "quality", selected, &["720p", "1080p", "1440p"], 64., !locked) {
                        draft.quality = values[i];
                    }
                });
                let fps_note = fps_hint(self.fps60_entry().status, draft.fps);
                settings_row(ui, t, "Frame rate", fps_note, |ui| {
                    let values = [24, 30, 60];
                    let selected = values.iter().position(|v| *v == draft.fps).unwrap_or(1);
                    if let Some(i) = segmented(ui, t, "fps", selected, &["24 fps", "30 fps", "60 fps"], 64., !locked) {
                        draft.fps = values[i];
                        // A person's pick is never overwritten by a qualified default later.
                        draft.fps_chosen = true;
                    }
                });
            }
            2 => {
                let camera_name = match &self.camera_device_id {
                    Some(id) => self.camera_names.iter().find(|d| &d.id == id).map(|d| d.name.clone()).unwrap_or_else(|| "Remembered camera".into()),
                    None => "System default".into(),
                };
                let microphone_name = self.microphone_device_id.clone().unwrap_or_else(|| "System default".into());
                for (camera, name) in [(true, camera_name), (false, microphone_name)] {
                    let label = if camera { "Camera" } else { "Microphone" };
                    settings_row(ui, t, label, None, |ui| {
                        let clipped = if name.chars().count() > 22 { format!("{}…", name.chars().take(21).collect::<String>()) } else { name.clone() };
                        let clicked = button(ui, t, &format!("{clipped} ›"), 190., false, !locked).clicked();
                        if camera { camera_panel |= clicked } else { microphone_panel |= clicked }
                    });
                }
                settings_row(ui, t, "Remember camera and mic on or off", Some("When off, camera and mic start off every time."), |ui| {
                    switch(ui, t, "Remember camera and mic on or off", &mut draft.remember, !locked);
                });
                let shapes: Vec<(u8, &str)> = [(SHAPE_CIRCLE, "Circle"), (SHAPE_ROUNDED, "Rounded"), (SHAPE_WIDE, "Wide"), (SHAPE_PORTRAIT, "Portrait")]
                    .into_iter()
                    .chain((draft.shape == SHAPE_SQUARE).then_some((SHAPE_SQUARE, "Square")))
                    .collect();
                settings_row(ui, t, "Bubble shape", Some("The camera is cropped to fill, never stretched."), |ui| {
                    let labels: Vec<&str> = shapes.iter().map(|(_, l)| *l).collect();
                    let selected = shapes.iter().position(|(s, _)| *s == draft.shape).unwrap_or(0);
                    if let Some(i) = segmented(ui, t, "shape", selected, &labels, 58., true) {
                        draft.shape = shapes[i].0;
                    }
                });
                settings_row(ui, t, "Bubble size", None, |ui| {
                    let labels: Vec<&str> = SIZES.iter().map(|(_, l)| *l).collect();
                    if let Some(i) = segmented(ui, t, "size", size_class(draft.size), &labels, 56., true) {
                        draft.size = SIZES[i].0;
                    }
                });
                settings_row(ui, t, "Mirror camera preview", Some("Flips the bubble you see. The camera in your recording isn't mirrored. A display recording can also capture the bubble as shown."), |ui| {
                    switch(ui, t, "Mirror camera preview", &mut draft.mirror, true);
                });
                ui.add_space(10.);
                ui.label(egui::RichText::new("Microphone only — system audio isn't available.").size(12.).color(t.secondary));
            }
            3 => {
                ui.add_space(6.);
                ui.add(egui::Label::new(egui::RichText::new("Share a temporary link from a recording in the Library. Codes and viewer sessions are kept in memory only.").size(12.).color(t.secondary)).wrap());
                ui.add_space(6.);
                ui.add(egui::Label::new(egui::RichText::new("Links stop when OpenRecorder quits. Keep this computer awake while sharing.").size(12.).color(t.secondary)).wrap());
                ui.add_space(14.);
                library = button(ui, t, "Open Library", 0., true, true).clicked();
            }
            4 => {
                let grants = self.grants();
                let settings_relevant = |grant: Grant| grant.is_blocked() && cfg!(any(target_os = "macos", target_os = "windows"));
                let rows: [(&str, &str, bool, u8); 3] = [
                    ("Screen Recording", if cfg!(target_os = "macos") { if grants.screen { "Allowed" } else { "Not allowed" } } else { "Managed by your system" }, cfg!(target_os = "macos") && !grants.screen, 0),
                    ("Camera", grants.camera.label(), settings_relevant(grants.camera), 1),
                    ("Microphone", grants.microphone.label(), settings_relevant(grants.microphone), 2),
                ];
                for (name, status, offer, which) in rows {
                    settings_row(ui, t, name, Some(status), |ui| {
                        if offer && button(ui, t, "Open Settings", 0., false, true).clicked() {
                            let result = match which {
                                0 => permissions::screen_settings(),
                                1 => permissions::input_settings(true),
                                _ => permissions::input_settings(false),
                            };
                            if let Err(e) = result {
                                self.settings_status = format!("Couldn't open System Settings. {e}");
                                self.settings_error = true;
                            }
                        }
                    });
                }
                ui.add_space(10.);
                let live = |on: bool, opening: bool| if on { "on" } else if opening { "opening" } else { "off" };
                ui.label(egui::RichText::new(format!(
                    "Live now: Camera {} · Microphone {} · {}",
                    live(self.camera_live(), self.camera_opening()),
                    live(self.microphone.is_some(), self.microphone_opening()),
                    if self.recording { "Recording" } else { "Not recording" }
                )).size(12.).color(t.secondary));
                ui.add_space(6.);
                ui.add(egui::Label::new(egui::RichText::new("OpenRecorder never resets system access and never asks again after you decline. Camera and Mic ask only when you turn them on.").size(12.).color(t.secondary)).wrap());
            }
            _ => {
                ui.add_space(6.);
                ui.label(egui::RichText::new(format!("OpenRecorder {}", env!("CARGO_PKG_VERSION"))).size(15.).color(t.text));
                ui.add_space(6.);
                ui.add(egui::Label::new(egui::RichText::new("Recordings stay on this computer.").size(12.).color(t.secondary)).wrap());
            }
        }
        if library {
            self.open_library();
        }
        if camera_panel {
            self.open_panel(PanelKind::Camera);
        }
        if microphone_panel {
            self.open_panel(PanelKind::Microphone);
        }
    }

    // ----- setup ---------------------------------------------------------
    /// One window, three statuses and one Done. Nothing here opens a device.
    fn setup_ui(&mut self, ctx: &egui::Context) {
        if !self.setup_open {
            return;
        }
        let grants = self.grants();
        let mut finished = false;
        let (mut allow_screen, mut allow_camera, mut allow_microphone) = (false, false, false);
        let (mut open_screen, mut open_camera_settings, mut open_microphone_settings) = (false, false, false);
        let mut close = false;
        let screen_action = screen_row_action(grants.screen, self.screen_attempted);
        let error = self.setup_error.clone();
        let dark_mark = ctx.style().visuals.dark_mode;
        ctx.show_viewport_immediate(
            vp(VP_SETUP),
            egui::ViewportBuilder::default()
                .with_title("Welcome to OpenRecorder")
                .with_inner_size([460., 440.])
                .with_resizable(false),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
                let t = tokens(ctx.style().visuals.dark_mode);
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface).inner_margin(24)).show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(36., 36.), egui::Sense::hover());
                        paint_library_mark(ui.painter(), rect.center(), dark_mark);
                        ui.label(egui::RichText::new("Welcome to OpenRecorder").size(20.).color(t.text));
                    });
                    ui.add_space(6.);
                    ui.add(egui::Label::new(egui::RichText::new("Record your screen with camera and mic. Recordings stay on this computer.").size(13.).color(t.secondary)).wrap());
                    ui.add_space(10.);
                    let mut row = |ui: &mut egui::Ui, name: &str, tag: &str, status: &str, action: RowAction, allow: &mut bool, open: &mut bool| {
                        settings_row(ui, &t, &format!("{name} · {tag}"), Some(status), |ui| match action {
                            RowAction::Allow => *allow = button(ui, &t, "Allow", 0., true, true).clicked(),
                            RowAction::OpenSettings => *open = button(ui, &t, "Open Settings", 0., false, true).clicked(),
                            RowAction::None => {}
                        });
                    };
                    if cfg!(target_os = "macos") {
                        row(ui, "Screen Recording", "Required", if grants.screen { "Allowed" } else if self.screen_attempted { "Not allowed" } else { "Not asked" }, screen_action, &mut allow_screen, &mut open_screen);
                    }
                    row(ui, "Camera", "Optional", grants.camera.label(), setup_row_action(grants.camera), &mut allow_camera, &mut open_camera_settings);
                    row(ui, "Microphone", "Optional", grants.microphone.label(), setup_row_action(grants.microphone), &mut allow_microphone, &mut open_microphone_settings);
                    if let Some(error) = &error {
                        ui.add_space(6.);
                        ui.label(egui::RichText::new(error).size(12.).color(t.record));
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                        finished = button(ui, &t, "Done", 96., true, true).clicked();
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new("Camera and mic stay off during setup.").size(12.).color(t.secondary));
                        });
                    });
                });
            },
        );
        // Requests only record the OS answer; granting never opens a device.
        if allow_screen {
            self.screen_attempted = true;
            self.screen_permission = Some((self.os.request_screen)());
        }
        if allow_camera {
            let _ = (self.os.request_camera)();
        }
        if allow_microphone {
            let _ = (self.os.request_microphone)();
        }
        if open_screen {
            let _ = permissions::screen_settings();
        }
        if open_camera_settings {
            let _ = permissions::input_settings(true);
        }
        if open_microphone_settings {
            let _ = permissions::input_settings(false);
        }
        if allow_screen || allow_camera || allow_microphone {
            self.refresh_grants();
        }
        if finished {
            let result = self
                .connection
                .as_ref()
                .map(|c| c.action("native-onboarding-complete", serde_json::json!({})).map(|_| ()))
                .unwrap_or(Ok(()));
            match result {
                Ok(()) => close = true,
                Err(_) => self.setup_error = Some("Couldn't save setup. Try Done again.".into()),
            }
        }
        if close {
            self.setup_open = false;
            self.setup_error = None;
            close_present_viewport(ctx, vp(VP_SETUP));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    // ----- countdown and quit -------------------------------------------
    fn countdown_ui(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && (self.countdown.is_some() || self.countdown_closing) {
            self.countdown = None;
            self.countdown_closing = false;
            self.cancel_start();
        }
        if self.countdown_closing {
            let hud = vp(VP_COUNTDOWN);
            if ctx.input(|i| i.raw.viewports.contains_key(&hud)) {
                close_present_viewport(ctx, hud);
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            self.countdown_closing = false;
            self.begin_recording();
        }
        if let Some(deadline) = self.countdown {
            if Instant::now() >= deadline {
                self.countdown = None;
                self.countdown_closing = true;
                close_present_viewport(ctx, vp(VP_COUNTDOWN));
                // Wait for native viewport removal before creating any recording files.
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            let n = countdown_number(deadline.saturating_duration_since(Instant::now()));
            let mut cancel = false;
            ctx.show_viewport_immediate(
                vp(VP_COUNTDOWN),
                egui::ViewportBuilder::default()
                    .with_title("OpenRecorder Countdown")
                    .with_position(hud_position(
                        self.space.lock().ok().and_then(|s| s.capture.or_else(|| s.monitors.first().copied())),
                        ctx.input(|i| i.viewport().monitor_size),
                    ))
                    .with_inner_size([140., 140.])
                    .with_resizable(false)
                    .with_decorations(false)
                    .with_transparent(true)
                    .with_taskbar(false)
                    .with_always_on_top(),
                |ctx, _| {
                    cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested());
                    let t = tokens_for(ctx);
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        let rect = ui.max_rect();
                        ui.painter().rect_filled(rect, 28, t.toast);
                        ui.painter().text(rect.center() - egui::vec2(0., 8.), egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(64.), rgb(0xF5F5F7));
                        ui.painter().text(rect.center_bottom() - egui::vec2(0., 18.), egui::Align2::CENTER_CENTER, "Esc to cancel", egui::FontId::proportional(11.), rgb(0xC7C7CC));
                    });
                },
            );
            if cancel {
                self.countdown = None;
                self.cancel_start();
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            close_present_viewport(ctx, vp(VP_COUNTDOWN));
        }
    }
    fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.quit_confirmation {
            return;
        }
        ctx.show_viewport_immediate(
            vp(VP_QUIT),
            egui::ViewportBuilder::default()
                .with_title("Stop recording and quit?")
                .with_inner_size([440., 150.])
                .with_resizable(false),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.quit_confirmation = false;
                }
                let t = tokens_for(ctx);
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface).inner_margin(20)).show(ctx, |ui| {
                    ui.label(egui::RichText::new("Stop recording and quit?").size(15.).color(t.text));
                    ui.add_space(4.);
                    ui.label(egui::RichText::new("The current recording is saved first.").size(12.).color(t.secondary));
                    ui.add_space(14.);
                    ui.horizontal(|ui| {
                        if button(ui, &t, "Keep Recording", 0., false, true).clicked() {
                            self.quit_confirmation = false;
                        }
                        if button(ui, &t, "Stop & Quit", 0., true, true).clicked() {
                            self.quit_confirmation = false;
                            self.close_after_save = true;
                            self.quit_after_save = true;
                            super::STOP_REQUEST.store(true, Ordering::Relaxed);
                        }
                    });
                });
            },
        );
    }

    // ----- camera bubble -------------------------------------------------
    fn bubble_commands(&mut self) {
        let commands = self.bubble_commands.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default();
        for command in commands {
            let mut draft = self.prefs.clone();
            match command {
                BubbleCommand::TurnOff => {
                    if !self.locked() {
                        self.turn_off(true);
                    }
                    continue;
                }
                BubbleCommand::Stop => {
                    if self.recording {
                        super::STOP_REQUEST.store(true, Ordering::Relaxed);
                    }
                    continue;
                }
                BubbleCommand::Mirror(on) => draft.mirror = on,
                BubbleCommand::Shape(shape) => draft.shape = shape,
                BubbleCommand::Size(size) => draft.size = size,
                BubbleCommand::ShowCamera(on) => draft.show_camera = on,
            }
            if draft != self.prefs {
                self.commit_settings(&draft);
            }
        }
    }
    fn bubble(&mut self, ctx: &egui::Context) {
        let Some(camera) = &self.camera else {
            close_present_viewport(ctx, vp(VP_CAMERA));
            return;
        };
        // The camera may be on while the bubble is hidden: the bubble is a choice of its own.
        if !self.prefs.show_camera {
            close_present_viewport(ctx, vp(VP_CAMERA));
            return;
        }
        if !camera.ready.load(Ordering::Relaxed) || (self.hidden && !self.recording) {
            return;
        }
        let pixels = camera.pixels.clone();
        let layout = self.layout.clone();
        let commands = self.bubble_commands.clone();
        let mirror = self.preview_mirror.clone();
        let can_turn_off = !self.locked();
        let recording = self.recording;
        let current_size = self.prefs.size;
        let progress = self.options.progress.clone();
        #[cfg(target_os = "linux")]
        let track_native_position = !self.hyprland.available();
        #[cfg(not(target_os = "linux"))]
        let track_native_position = true;
        let (width, height) = bubble_dimensions(self.prefs.size as f32, self.prefs.shape);
        let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1440., 900.));
        let space = self.space.clone();
        ctx.show_viewport_deferred(
            vp(VP_CAMERA),
            egui::ViewportBuilder::default()
                .with_title("OpenRecorder Camera")
                .with_active(false)
                .with_transparent(true)
                .with_inner_size([width, height])
                .with_position([(monitor.x - 24. - 224. * 4. / 3.).max(10.), (monitor.y - 24. - 168. - 60.).max(10.)])
                .with_decorations(false)
                .with_resizable(false)
                .with_taskbar(false)
                .with_always_on_top(),
            move |ctx, _| {
                #[cfg(target_os = "windows")]
                if !ctx.data_mut(|d| d.get_temp::<bool>(egui::Id::new("content-protected")).unwrap_or(false)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::ContentProtected(true));
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("content-protected"), true));
                }
                let push = |command: BubbleCommand| {
                    if let Ok(mut queue) = commands.lock() {
                        queue.push(command);
                    }
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                };
                if ctx.input(|i| i.viewport().close_requested()) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    if can_turn_off {
                        push(BubbleCommand::TurnOff);
                    }
                }
                let shape = layout.lock().map(|l| l.shape).unwrap_or(SHAPE_CIRCLE);
                if track_native_position {
                    if let Some((rect, reported)) = ctx.input(|i| i.viewport().outer_rect.zip(i.viewport().monitor_size)) {
                        // Fractions of the area the recording shows, origin included. When
                        // that area is unknown the last layout is kept rather than guessed.
                        let region = space.lock().ok().and_then(|s| layout_region(rect, &s, Some(reported)));
                        if let Some((x, y, size)) = region.and_then(|r| bubble_layout(rect, shape, r)) {
                            if let Ok(mut l) = layout.lock() {
                                l.x = x;
                                l.y = y;
                                l.size = size;
                            }
                        }
                    }
                }
                // Keyboard only while the bubble itself is focused.
                if ctx.input(|i| i.viewport().focused == Some(true)) {
                    ctx.input(|i| {
                        let step = if i.modifiers.shift { 50. } else { 10. };
                        let outer = i.viewport().outer_rect;
                        let delta = [(egui::Key::ArrowLeft, egui::vec2(-step, 0.)), (egui::Key::ArrowRight, egui::vec2(step, 0.)), (egui::Key::ArrowUp, egui::vec2(0., -step)), (egui::Key::ArrowDown, egui::vec2(0., step))]
                            .into_iter()
                            .filter(|(key, _)| i.key_pressed(*key))
                            .fold(egui::Vec2::ZERO, |sum, (_, d)| sum + d);
                        if let (Some(outer), true) = (outer, delta != egui::Vec2::ZERO) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(outer.min + delta));
                        }
                        for (index, key) in [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3].into_iter().enumerate() {
                            if i.key_pressed(key) {
                                push(BubbleCommand::Size(SIZES[index].0));
                            }
                        }
                        if i.key_pressed(egui::Key::S) {
                            push(BubbleCommand::Shape(next_shape(shape)));
                        }
                    });
                }
                let t = tokens_for(ctx);
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let bounds = ui.max_rect();
                    let (dw, dh) = preview_dimensions(shape);
                    if let Ok(frame) = pixels.lock() {
                        let image = egui::ColorImage::from_rgba_unmultiplied([dw, dh], &preview_cover(&frame, mirror.load(Ordering::Relaxed), dw, dh));
                        let id = egui::Id::new("camera-texture");
                        let cached = ctx.data_mut(|d| d.get_temp::<(egui::TextureHandle, [usize; 2])>(id));
                        let texture = match cached {
                            Some((texture, dims)) if dims == [dw, dh] => {
                                texture.clone().set_partial([0, 0], image, egui::TextureOptions::LINEAR);
                                texture
                            }
                            _ => ctx.load_texture("camera-frame", image, egui::TextureOptions::LINEAR),
                        };
                        ctx.data_mut(|d| d.insert_temp(id, (texture.clone(), [dw, dh])));
                        let radius = bubble_radius(shape, bounds.width(), bounds.height()).min(255.) as u8;
                        let response = ui.add(
                            egui::Image::new(&texture)
                                .maintain_aspect_ratio(false)
                                .corner_radius(egui::CornerRadius::same(radius))
                                .fit_to_exact_size(bounds.size())
                                .sense(egui::Sense::click_and_drag()),
                        );
                        if response.drag_started() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        // Right-click: Shape, Size, Mirror Preview, then Turn Camera Off, then Stop while recording.
                        response.context_menu(|ui| {
                            ui.menu_button("Shape", |ui| {
                                for (label, target) in [("Circle", SHAPE_CIRCLE), ("Rounded Square", SHAPE_ROUNDED), ("Wide 16:9", SHAPE_WIDE), ("Portrait 9:16", SHAPE_PORTRAIT)] {
                                    if ui.radio(shape == target, label).clicked() {
                                        push(BubbleCommand::Shape(target));
                                        ui.close();
                                    }
                                }
                            });
                            ui.menu_button("Size", |ui| {
                                for (size, label) in SIZES {
                                    if ui.radio(size_class(current_size) == size_class(size), label).clicked() {
                                        push(BubbleCommand::Size(size));
                                        ui.close();
                                    }
                                }
                            });
                            let mirrored = mirror.load(Ordering::Relaxed);
                            if ui.checkbox(&mut { mirrored }, "Mirror Preview").clicked() {
                                push(BubbleCommand::Mirror(!mirrored));
                                ui.close();
                            }
                            if ui.button("Hide Camera").clicked() {
                                push(BubbleCommand::ShowCamera(false));
                                ui.close();
                            }
                            ui.separator();
                            if ui.add_enabled(can_turn_off, egui::Button::new("Turn Camera Off")).clicked() {
                                push(BubbleCommand::TurnOff);
                                ui.close();
                            }
                            if recording {
                                let seconds = progress.frames.load(Ordering::Relaxed) / progress.fps.load(Ordering::Relaxed).max(1) as u64;
                                if ui.button(format!("Stop Recording {}", format_elapsed(seconds))).clicked() {
                                    push(BubbleCommand::Stop);
                                    ui.close();
                                }
                            }
                        });
                    }
                    if ui.rect_contains_pointer(bounds) {
                        // Controls sit on the top edge, away from the face. A small bubble shows one compact button
                        // that opens the same choices below it.
                        let compact = bounds.width().min(bounds.height()) < 150.;
                        let inset = capsule_inset(shape, bounds);
                        let more_id = egui::Id::new("bubble-more");
                        let more = ctx.data_mut(|d| d.get_temp::<bool>(more_id).unwrap_or(false));
                        let mut row = 0;
                        if compact {
                            let (capsule, buttons) = capsule_layout(bounds, 1, row, inset);
                            ui.painter().rect_filled(capsule, capsule.height() / 2., Color32::from_black_alpha(190));
                            let response = ui.interact(buttons[0], ui.id().with("bubble-more"), egui::Sense::click());
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Camera bubble options"));
                            if response.hovered() || more {
                                ui.painter().circle_filled(buttons[0].center(), buttons[0].width() / 2., Color32::from_white_alpha(30));
                            }
                            ui.painter().text(buttons[0].center(), egui::Align2::CENTER_CENTER, "•••", egui::FontId::proportional(11.), Color32::WHITE);
                            if response.on_hover_text("Options").clicked() {
                                ctx.data_mut(|d| d.insert_temp(more_id, !more));
                            }
                            row += 1;
                        }
                        if !compact || more {
                            // Four shapes, then × where there is room; the right-click menu always has Turn Camera Off.
                            let close_room = can_turn_off && bounds.width().min(bounds.height()) >= 168.;
                            let count = if close_room { 5 } else { 4 };
                            let (capsule, buttons) = capsule_layout(bounds, count, row, inset);
                            ui.painter().rect_filled(capsule, capsule.height() / 2., Color32::from_black_alpha(190));
                            // A stored legacy square highlights none of the four.
                            for (index, (target, rect)) in [SHAPE_CIRCLE, SHAPE_ROUNDED, SHAPE_WIDE, SHAPE_PORTRAIT].into_iter().zip(buttons.iter()).enumerate() {
                                let response = ui.interact(*rect, ui.id().with(("bubble-shape", index)), egui::Sense::click());
                                let names = ["Circle", "Rounded square", "Wide", "Portrait"];
                                response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, shape == target, format!("{} shape", names[index])));
                                if shape == target || response.hovered() {
                                    ui.painter().circle_filled(rect.center(), rect.width() / 2., Color32::from_white_alpha(if shape == target { 60 } else { 30 }));
                                }
                                paint_shape_glyph(ui.painter(), target, rect.center(), Color32::WHITE);
                                if response.on_hover_text(names[index]).clicked() {
                                    push(BubbleCommand::Shape(target));
                                }
                            }
                            if close_room {
                                let rect = buttons[4];
                                let response = ui.interact(rect, ui.id().with("bubble-off"), egui::Sense::click());
                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Turn camera off"));
                                if response.hovered() {
                                    ui.painter().circle_filled(rect.center(), rect.width() / 2., Color32::from_white_alpha(30));
                                }
                                paint_icon(ui.painter(), Icon::Close, rect.center(), Color32::WHITE);
                                if response.on_hover_text("Turn Camera Off").clicked() {
                                    push(BubbleCommand::TurnOff);
                                }
                            }
                        }
                    }
                    let _ = t;
                });
                ctx.request_repaint_after(Duration::from_millis(34));
            },
        );
    }
}

impl State {
    fn camera_lost(&mut self, text: &str, detail: &str) {
        if let Some(camera) = &self.camera {
            if let Ok(mut error) = camera.error.lock() {
                *error = Some(text.into());
            }
        }
        self.camera = None;
        self.camera_attention = Some(InputProblem::Failed { camera: true, detail: detail.to_owned() });
        self.input_error = Some((true, detail.to_owned()));
        if self.start.is_none() {
            self.show_toast(Toast::error(text).with_action(ToastAction::ChooseCamera).with_details(detail.to_owned()));
        }
    }
    fn complete_permissions(&mut self, ctx: &egui::Context) {
        if let Some(permission) = self.screen_permission.clone() {
            match permission.load(Ordering::Relaxed) {
                1 => {
                    self.screen_permission = None;
                    self.screen_ready = true;
                    self.refresh_grants();
                }
                -1 => {
                    self.screen_permission = None;
                    self.show_toast(Toast::error("Screen Recording is off").with_action(ToastAction::OpenScreenSettings));
                }
                _ => ctx.request_repaint_after(Duration::from_millis(200)),
            }
        }
        for camera in [true, false] {
            let pending = if camera { self.camera_permission.clone() } else { self.microphone_permission.clone() };
            let Some(permission) = pending else { continue };
            match permission.load(Ordering::Relaxed) {
                1 => {
                    if camera {
                        self.camera_permission = None;
                        if let Err(e) = self.open_camera_now() {
                            self.input_failed(true, e);
                        }
                    } else {
                        self.microphone_permission = None;
                        if let Err(e) = self.open_microphone_now() {
                            self.input_failed(false, e);
                        }
                    }
                }
                -1 => {
                    // The OS said no: the person's click didn't turn anything on.
                    if camera {
                        self.camera_permission = None;
                        self.camera_intent = false;
                    } else {
                        self.microphone_permission = None;
                        self.microphone_intent = false;
                    }
                    self.persist_intent();
                    self.attention(InputProblem::AccessOff { camera });
                }
                _ => ctx.request_repaint_after(Duration::from_millis(200)),
            }
        }
    }
    fn watch_camera(&mut self) {
        let stalled = self.camera.as_ref().is_some_and(|c| {
            c.ready.load(Ordering::Relaxed)
                && c.last_frame.as_ref().is_some_and(|last| {
                    (c.opened.elapsed().as_millis() as u64).saturating_sub(last.load(Ordering::Relaxed)) > 2500
                })
        });
        if stalled {
            super::event_log::event("camera_stalled", serde_json::json!({"timeoutMs":2500}));
            return self.camera_lost("Camera stopped sending frames", "Reconnect it or choose another camera.");
        }
        if self.camera.as_ref().is_some_and(|c| !c.ready.load(Ordering::Relaxed) && c.opened.elapsed() > Duration::from_secs(8)) {
            return self.camera_lost("Camera didn't send video", "Check access or choose another camera.");
        }
        let exit = self
            .camera
            .as_mut()
            .and_then(|c| c.child.as_mut())
            .and_then(|c| c.try_wait().ok().flatten());
        if let Some(status) = exit {
            super::event_log::event("camera_process_exited", serde_json::json!({"status":status.to_string()}));
            self.camera_lost("Camera couldn't open", "Choose another camera or check access in System Settings.");
        }
    }
    // ----- the capture machine, the menu and Home ------------------------------------------------
    /// The one stage every surface renders from. Derived from the machine's facts, never stored a second time.
    fn stage(&self, now: Instant) -> Stage {
        if self.recording {
            return match self.options.progress.phase.load(Ordering::Relaxed) {
                0 => Stage::Starting,
                2 => Stage::Stopping,
                _ => Stage::Recording,
            };
        }
        if let Some(deadline) = self.countdown {
            return Stage::Countdown(countdown_number(deadline.saturating_duration_since(now)) as u32);
        }
        if self.countdown_closing {
            return Stage::Countdown(1);
        }
        if self.start.is_some() {
            return Stage::Starting;
        }
        if let Some(pending) = &self.completion {
            // Waiting to retry by hand is idle: the notice says so and Record retries.
            if self.completion_next.is_none() {
                return Stage::Idle;
            }
            return if matches!(pending.kind(), super::connection::CompletionKind::Begin(_)) { Stage::Starting } else { Stage::Saving };
        }
        if self.worker.is_some() {
            return Stage::Stopping;
        }
        Stage::Idle
    }
    /// The source a take would record, as capability evidence names it: the real display id, or "unresolved".
    fn source_key(&self) -> String {
        source_identity(cfg!(target_os = "linux"), self.options.selection, self.main_display)
    }
    /// The camera a take would include (None when it will not), by device.
    fn camera_key(&self) -> Option<String> {
        (self.camera_intent || self.camera.is_some()).then(|| self.camera_device_id.clone().unwrap_or_else(|| "default".into()))
    }
    /// The take as it would run now: evidence for a different kind of take does not apply to it.
    fn planned_workload(&self) -> super::capability::Workload {
        self.workload_for(&self.source_key())
    }
    /// The same, for a named source: a take uses the one it resolved at its own start, not the cached guess.
    fn workload_for(&self, source: &str) -> super::capability::Workload {
        self.capabilities.plan(
            60,
            self.confirmed.quality,
            super::codec(),
            source,
            self.camera_key().as_deref(),
            self.microphone_intent || self.microphone.is_some(),
            self.confirmed.originals,
        )
    }
    fn fps60_entry(&self) -> super::capability::Entry {
        self.capabilities.status(&self.planned_workload())
    }
    /// The rate a take will use: the stored choice, or 60 only where it is qualified and was never chosen against.
    #[cfg(test)]
    fn consumed_len(&self) -> usize {
        self.consumed.len()
    }
    #[cfg(test)]
    fn heartbeat_for_test(&self) -> String {
        format!("{:?}", self.heartbeat.last_for_test())
    }
    fn fps_in_use(&self) -> u32 {
        super::capability::effective_fps(self.confirmed.fps, self.confirmed.fps_chosen, self.fps60_entry().status)
    }
    /// The rate for a take of this exact source.
    fn fps_for_source(&self, source: &str) -> u32 {
        super::capability::effective_fps(self.confirmed.fps, self.confirmed.fps_chosen, self.capabilities.status(&self.workload_for(source)).status)
    }
    fn input_mark(&self, camera: bool) -> super::tray::Mark {
        use super::tray::Mark as M;
        let grant = if camera { self.grants().camera } else { self.grants().microphone };
        match self.mark(camera) {
            Mark::Live => M::On,
            // Opening for a take that is starting reads "Starting…"; being chosen while idle never opens anything.
            Mark::Armed if self.locked() && (if camera { self.camera_opening() } else { self.microphone_opening() }) => M::Starting,
            Mark::Armed => M::Armed,
            Mark::Attention => {
                let problem = if camera { &self.camera_attention } else { &self.microphone_attention };
                if matches!(problem, Some(InputProblem::AccessOff { .. } | InputProblem::Restricted { .. } | InputProblem::NeedsAccess { .. })) { M::AccessOff } else { M::NotConnected }
            }
            Mark::None if grant.is_blocked() => M::AccessOff,
            Mark::None => M::Off,
        }
    }
    /// The chosen device's name when the system has reported it: a camera's from the listing it came from, a
    /// microphone's own name. An id that no list resolves has no name here; none is made up.
    fn input_name(&self, camera: bool) -> Option<String> {
        if camera {
            self.camera_device_id.as_ref().and_then(|id| self.camera_names.iter().find(|d| &d.id == id).map(|d| d.name.clone()))
        } else {
            self.microphone_device_id.clone().filter(|n| !n.is_empty())
        }
    }
    fn tray_devices(&self, camera: bool) -> Vec<super::tray::Device> {
        if camera {
            let Some(list) = &self.camera_choices else { return Vec::new() };
            list.iter()
                .map(|d| {
                    let suffix: String = d.id.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                    let label = if super::devices::shares_name(list, d) { format!("{} …{}", d.name, suffix.to_uppercase()) } else { d.name.clone() };
                    super::tray::Device { uid: d.id.clone(), label, selected: self.camera_device_id.as_deref() == Some(d.id.as_str()) }
                })
                .collect()
        } else {
            // A name shared by two microphones can't be told apart, so it isn't offered (as in the old picker).
            let Some(list) = &self.microphone_choices else { return Vec::new() };
            list.iter()
                .filter(|m| !m.duplicate)
                .map(|m| super::tray::Device { uid: m.name.clone(), label: m.name.clone(), selected: self.microphone_device_id.as_deref() == Some(m.name.as_str()) })
                .collect()
        }
    }
    fn tray_displays(&self) -> Vec<super::tray::Display> {
        self.targets
            .iter()
            .filter(|(window, ..)| !window)
            .enumerate()
            .map(|(index, (_, id, name))| super::tray::Display {
                id: *id,
                label: if name.trim().is_empty() { format!("Display {}", index + 1) } else { name.clone() },
                selected: match self.options.selection {
                    Some((false, selected)) => selected == *id,
                    None => index == 0,
                    Some((true, _)) => false,
                },
            })
            .collect()
    }
    /// The notice a person should see for the machine as it is: a failure that was recorded, or an
    /// unsaved take that is waiting for them.
    fn current_notice(&self) -> Option<super::controls::Notice> {
        if let Some(notice) = &self.capture_notice {
            return Some(notice.clone());
        }
        (self.completion.is_some() && self.completion_next.is_none()).then(|| super::controls::Notice {
            kind: "failed",
            text: "The last recording isn't saved yet. Choose Record to try again.".into(),
            details: self.completion.as_ref().map(|p| p.last_error.clone()),
            kept: false,
            generation: self.generation,
        })
    }
    fn tray_state(&self, stage: Stage) -> super::tray::TrayState {
        use super::tray::Phase;
        let elapsed = self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        let phase = match stage {
            Stage::Idle => Phase::Idle,
            Stage::Countdown(n) => Phase::Countdown(n),
            Stage::Starting => Phase::Starting,
            Stage::Recording => Phase::Recording(elapsed),
            Stage::Stopping | Stage::Saving => Phase::Stopping,
        };
        let window_selected = matches!(self.options.selection, Some((true, _)));
        let wayland = cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some();
        super::tray::TrayState {
            phase,
            camera: super::tray::Input { mark: self.input_mark(true), devices: self.tray_devices(true), name: self.input_name(true) },
            microphone: super::tray::Input { mark: self.input_mark(false), devices: self.tray_devices(false), name: self.input_name(false) },
            source_label: self.source_label().0,
            displays: self.tray_displays(),
            window_selected,
            capability_line: super::controls::capability_line(wayland, window_selected).to_owned(),
            shape: super::connection::shape_name(self.prefs.shape).to_owned(),
            size: self.prefs.size,
            mirror: self.prefs.mirror,
            show_camera: self.prefs.show_camera,
            recent: self.recent.clone(),
            notice: self.current_notice().map(|n| super::tray::Notice { text: n.text, kept: n.kept }),
        }
    }
    fn snapshot(&self, stage: Stage) -> super::controls::Snapshot {
        let p = &self.options.progress;
        let mark_word = |m: super::tray::Mark| match m {
            super::tray::Mark::Off => "off",
            super::tray::Mark::Armed => "armed",
            super::tray::Mark::Starting => "starting",
            super::tray::Mark::On => "live",
            super::tray::Mark::NotConnected => "not-connected",
            super::tray::Mark::AccessOff => "access-off",
        };
        let devices = |camera: bool| self.tray_devices(camera).into_iter().map(|d| (d.uid, d.label, d.selected)).collect::<Vec<_>>();
        let milli = p.delivered_milli_fps.load(Ordering::Relaxed);
        let source_frames = p.source_frames.load(Ordering::Relaxed);
        let recording = matches!(stage, Stage::Recording | Stage::Stopping);
        let seconds = self.started_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.);
        super::controls::Snapshot {
            stage,
            generation: self.generation,
            launch: self.launch.clone(),
            elapsed_ms: self.started_at.map(|t| t.elapsed().as_millis() as u64),
            fps_set: p.fps.load(Ordering::Relaxed).max(self.fps_in_use()),
            output_frames: p.frames.load(Ordering::Relaxed),
            fps_delivered: (recording && milli > 0).then(|| milli as f64 / 1000.),
            source_frames,
            source_fps: (recording && seconds > 1. && source_frames > 1).then(|| source_frames as f64 / seconds),
            tray_available: self.tray.as_ref().is_some_and(|t| !t.host_lost()),
            tray_note: self.tray_error.clone().or_else(|| self.tray.as_ref().and_then(|t| t.lost_reason()).map(str::to_owned)),
            screen_allowed: self.screen_ready,
            camera: (mark_word(self.input_mark(true)), self.input_name(true)),
            microphone: (mark_word(self.input_mark(false)), self.input_name(false)),
            devices: (devices(true), devices(false)),
            appearance: (super::connection::shape_name(self.prefs.shape).to_owned(), self.prefs.size, self.prefs.mirror, self.prefs.show_camera),
            source: {
                let (label, icon) = self.source_label();
                (if matches!(icon, Icon::Window) { "window" } else { "display" }, label)
            },
            notice: self.current_notice(),
            fps60: Some(self.capabilities.publish_fps60(&self.planned_workload())),
            sources: self.tray_displays().into_iter().map(|d| (d.id, "display", d.label, d.selected)).collect(),
            screen_state: if self.screen_ready { "allowed" } else if self.screen_attempted { "denied" } else { "not-asked" },
        }
    }
    /// One pass of the machine: keep `started_at` honest, draw the tray, run what was chosen in it or relayed
    /// from Home, and tell Home. Runs every update; it only does work when something changed.
    fn tray_update(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let stage = self.stage(now);
        match (stage, self.started_at) {
            (Stage::Recording, None) => self.started_at = Some(now),
            (Stage::Recording, Some(_)) => {}
            // Any other stage ends the timer in the same pass: nothing keeps a frozen time alive.
            (_, Some(_)) => self.started_at = None,
            _ => {}
        }
        if self.benchmark {
            return;
        }
        self.home_fallback();
        self.refresh_devices_for_menu(stage);
        self.poll_recent();
        self.poll_capacity();
        let actions = self.tray.as_ref().map(|t| t.actions()).unwrap_or_default();
        for action in actions {
            self.menu_action(ctx, action);
        }
        self.human_commands(ctx);
        // Re-derive after the actions: the tray draws what the machine is now, not what it was.
        let stage = self.stage(Instant::now());
        if self.tray.is_some() {
            let state = self.tray_state(stage);
            if let Some(tray) = self.tray.as_mut() {
                tray.show(&state);
            }
        }
        // The next publication is scheduled explicitly: an idle app with everything off has nothing else to repaint it.
        if let Some(wake) = self.publish_capture(stage, super::connection::now_ms()) {
            ctx.request_repaint_after(Duration::from_millis(wake));
        }
        if stage.busy() && self.tray.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if let Some(wait) = self.fallback.next_check(super::connection::now_ms()) {
            ctx.request_repaint_after(Duration::from_millis(wait));
        }
    }
    /// Opens Home when the tray cannot be used (never there, or lost later) and Home is not already open. The
    /// decision is made once per loss, after a short grace, and loss events are bounded (see `HomeFallback`).
    fn home_fallback(&mut self) {
        self.home_fallback_at(super::connection::now_ms());
    }
    fn home_fallback_at(&mut self, now: u64) {
        if !self.fallback_enabled {
            return;
        }
        let available = self.tray.as_ref().is_some_and(|t| !t.host_lost());
        self.fallback.observe(available, now);
        let connection = self.connection.clone();
        let home_age = move || {
            let state = connection?.action("state", serde_json::json!({})).ok()?;
            state["homeAgeMs"].as_u64()
        };
        if self.fallback.due(now, home_age) {
            self.open_library();
        }
    }
    /// Keeps the menu's device and display lists fresh. Listing is read-only: no device is opened and no
    /// permission is asked for.
    fn refresh_devices_for_menu(&mut self, stage: Stage) {
        // Whether a tray exists is irrelevant: Home's pickers need these lists on a desktop with no tray too. Listing opens
        // nothing and asks for nothing (it only runs where access already exists, see `start_camera_listing`).
        if stage != Stage::Idle {
            return;
        }
        if self.devices_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(20)) {
            return;
        }
        // The first update runs this before the periodic permission refresh, when nothing is known yet and `grants()`
        // would stand in "system managed" (which lists even where macOS access is unasked or denied). Read the passive
        // status first: the same silent read the refresh makes (no request, no prompt, no device), so an unknown
        // permission is never treated as allowed.
        if self.grants.is_none() {
            self.refresh_grants();
        }
        self.devices_checked = Some(Instant::now());
        if self.camera_discovery.is_none() {
            self.start_camera_listing();
        }
        if self.microphone_discovery.is_none() {
            self.start_microphone_listing();
        }
        #[cfg(not(target_os = "linux"))]
        if self.screen_ready {
            let own_pid = std::process::id();
            self.targets = scap::get_all_targets()
                .into_iter()
                .filter_map(|t| match t {
                    scap::Target::Window(w) if super::capture::window_ownership(w.owner_pid, own_pid) != super::capture::Ownership::Own => Some((true, w.id, w.title)),
                    scap::Target::Display(d) => Some((false, d.id, d.title)),
                    _ => None,
                })
                .collect();
        }
    }
    fn poll_recent(&mut self) {
        if self.recent_job.as_ref().is_some_and(|h| h.is_finished()) {
            if let Ok(list) = self.recent_job.take().unwrap().join() {
                self.recent = list;
            }
        }
        if self.recent_job.is_none() && self.recent_due.is_some_and(|t| Instant::now() >= t) {
            self.recent_due = None;
            if let Some(conn) = self.connection.clone() {
                self.recent_job = Some(thread::spawn(move || {
                    conn.action("state", serde_json::json!({})).map(|v| super::controls::recent_from_state(&v)).unwrap_or_default()
                }));
            }
        }
    }
    /// Publishes the state to Home on the heartbeat's schedule and returns how long until the next publication is
    /// due, so the caller can ask for exactly that wakeup. None: there is no host to tell.
    fn publish_capture(&mut self, stage: Stage, now_ms: u64) -> Option<u64> {
        let conn = self.connection.clone().filter(|_| !self.library_offline)?;
        let publisher = self.publisher.get_or_insert_with(|| super::connection::publisher(conn)).clone();
        let snapshot = self.snapshot(stage);
        let key = format!(
            "{}|{}|{:?}|{}|{}|{}|{}|{}|{:?}",
            stage.name(),
            self.generation,
            stage,
            snapshot.camera.0,
            snapshot.microphone.0,
            snapshot.source.1,
            snapshot.tray_available,
            snapshot.fps60.as_ref().map(|v| v.to_string()).unwrap_or_default(),
            snapshot.notice.as_ref().map(|n| (&n.text, n.generation)),
        );
        // A look change and a newly listed device are published at once, not on the next heartbeat.
        let key = format!("{key}|{:?}|{:?}", snapshot.appearance, snapshot.devices);
        // Often enough that the host never sees an idle app as silent (it refuses controls for an app silent for 12 s).
        let period = if stage.busy() { super::controls::BUSY_PERIOD_MS } else { super::controls::IDLE_PERIOD_MS };
        let (publish, wake) = self.heartbeat.due(&key, now_ms, period);
        if publish {
            publisher.publish(super::controls::progress_json(&snapshot));
        }
        Some(wake)
    }
    fn cancel_pending(&mut self, ctx: &egui::Context) {
        if self.countdown.take().is_some() || self.countdown_closing {
            self.countdown_closing = false;
            close_present_viewport(ctx, vp(VP_COUNTDOWN));
        }
        self.cancel_start();
    }
    fn set_input(&mut self, camera: bool, want: bool) {
        let on = if camera { self.camera.is_some() || self.camera_permission.is_some() } else { self.microphone.is_some() || self.microphone_permission.is_some() };
        if on != want {
            // The person's own press, so a fresh status is read now; a refusal is one notice, not a prompt loop.
            self.click_input(camera, permissions::input_grant(camera));
        }
    }
    fn live_look(&mut self, shape: Option<u8>, size: Option<u32>, mirror: Option<bool>) {
        let mut draft = self.prefs.clone();
        if let Some(shape) = shape {
            draft.shape = shape;
        }
        if let Some(size) = size {
            draft.size = size;
        }
        if let Some(mirror) = mirror {
            draft.mirror = mirror;
        }
        if draft != self.prefs {
            self.commit_settings(&draft);
        }
    }
    fn live_show_camera(&mut self, on: bool) {
        let mut draft = self.prefs.clone();
        draft.show_camera = on;
        if draft != self.prefs {
            self.commit_settings(&draft);
        }
    }
    fn choose_fps(&mut self, fps: u32) {
        if self.locked() || ![24, 30, 60].contains(&fps) {
            return;
        }
        let mut draft = self.prefs.clone();
        draft.fps = fps;
        draft.fps_chosen = true;
        self.commit_settings(&draft);
    }
    fn choose_source(&mut self, window: bool, id: Option<u32>) {
        if self.locked() {
            return;
        }
        self.options.selection = id.map(|id| (window, id));
        self.space_checked = None;
    }
    fn start_capacity_check(&mut self) {
        if self.capacity_job.is_some() || self.locked() {
            return;
        }
        let (fps, quality) = (60, self.confirmed.quality);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let ffmpeg = std::env::var("OPENRECORDER_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        // Synthetic pictures through the encoder for a few seconds: no screen, camera or microphone is touched.
        self.capacity_job = Some((thread::spawn(move || super::capability::capacity_check(&ffmpeg, fps, quality, 4.0, &flag)), cancel, quality));
        self.show_toast(Toast::info("Testing 60 fps without using any camera, microphone or screen…"));
    }
    fn poll_capacity(&mut self) {
        if !self.capacity_job.as_ref().is_some_and(|(h, ..)| h.is_finished()) {
            return;
        }
        let (handle, _, quality) = self.capacity_job.take().unwrap();
        let outcome = handle.join().unwrap_or_else(|_| super::capability::CheckOutcome::Failed("The test stopped unexpectedly".into()));
        let changed = self.capabilities.record_check(60, quality, super::codec(), &outcome, super::connection::now_ms());
        self.save_capabilities();
        self.heartbeat.reset();
        self.show_toast(match &outcome {
            super::capability::CheckOutcome::Passed { output_fps } => Toast::info(format!("60 fps kept up in the test · {output_fps:.0} fps")),
            super::capability::CheckOutcome::Slow { output_fps } => Toast::error(format!("60 fps didn't keep up in the test · {output_fps:.0} fps")),
            super::capability::CheckOutcome::EncoderRefused(why) => Toast::error("The video encoder didn't accept 60 fps").with_details(why.clone()),
            super::capability::CheckOutcome::Cancelled => Toast::info("Test cancelled"),
            super::capability::CheckOutcome::Failed(why) => Toast::error("The 60 fps test couldn't run").with_details(why.clone()),
        });
        let _ = changed;
    }
    fn capabilities_path(&self) -> PathBuf {
        super::library_host::home().join("capabilities.json")
    }
    fn save_capabilities(&self) {
        let _ = std::fs::create_dir_all(super::library_host::home());
        let _ = self.capabilities.save(&self.capabilities_path());
    }
    fn menu_action(&mut self, ctx: &egui::Context, action: super::tray::Action) {
        use super::tray::Action as A;
        match action {
            A::Record => self.record_action(ctx),
            A::Stop => {
                if self.recording {
                    super::STOP_REQUEST.store(true, Ordering::Relaxed);
                }
            }
            A::CancelCountdown => self.cancel_pending(ctx),
            A::CameraOn(on) => self.set_input(true, on),
            A::MicOn(on) => self.set_input(false, on),
            A::CameraDevice(uid) => self.apply_device_choice(true, uid.map_or(DeviceChoice::Off, DeviceChoice::Device)),
            A::MicDevice(uid) => self.apply_device_choice(false, uid.map_or(DeviceChoice::Off, DeviceChoice::Device)),
            A::MicDefault => self.apply_device_choice(false, DeviceChoice::SystemDefault),
            A::Display(id) => self.choose_source(false, Some(id)),
            A::ChooseWindow => {
                self.open_panel(PanelKind::Source);
                if let Some(panel) = &mut self.panel {
                    panel.tab = SourceTab::Window;
                }
            }
            A::Shape(name) => self.live_look(Some(shape_from_name(name)), None, None),
            A::Size(size) => self.live_look(None, Some(size), None),
            A::Mirror(on) => self.live_look(None, None, Some(on)),
            A::ShowCamera(on) => self.live_show_camera(on),
            A::OpenRecent(_) | A::OpenHome => {
                self.capture_notice = None;
                self.open_library();
            }
            // One Settings: Home's Settings route, the same in the browser and in an agent's app window.
            A::Settings => self.open_library_route("settings"),
            A::OpenLastRecording => {
                self.capture_notice = None;
                self.open_library_route("last");
            }
            A::Quit => self.request_quit(ctx),
        }
        ctx.request_repaint();
    }
    /// Commands relayed from Home. Each is gated by the machine's own stage and take number, and by the time
    /// this process started, so nothing queued earlier can ever start or stop anything.
    fn human_commands(&mut self, ctx: &egui::Context) {
        let Some(inbox) = self.human.clone() else { return };
        let commands: Vec<super::connection::HumanCommand> = inbox.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
        for command in commands {
            let stage = self.stage(Instant::now());
            // Everything is read again here, at the moment of execution: the polling thread may have taken the
            // command long ago, and the UI may have been stalled since.
            let session = super::controls::Session { launch: &self.launch, generation: self.generation, launched_at: self.launched_ms, now: super::connection::now_ms() };
            if super::controls::gate(&command, stage, &session, &mut self.consumed) != super::controls::Gate::Run {
                continue;
            }
            use super::connection::HumanAction as H;
            match command.action {
                H::Record => self.record_action(ctx),
                H::Stop => self.menu_action(ctx, super::tray::Action::Stop),
                H::Cancel => self.cancel_pending(ctx),
                H::Input { camera, on } => self.set_input(camera, on),
                H::Source { kind, id } => self.choose_source(kind == "window", id.filter(|_| kind != "default")),
                H::Device { camera, uid } => self.apply_device_choice(camera, uid.map_or(DeviceChoice::Off, DeviceChoice::Device)),
                H::Shape(name) => self.live_look(Some(shape_from_name(match name.as_str() {
                    "circle" => "circle",
                    "rounded" => "rounded",
                    "square" => "square",
                    "wide" => "wide",
                    _ => "portrait",
                })), None, None),
                H::Size(size) if valid_size(size as u64) => self.live_look(None, Some(size), None),
                H::Size(_) => {}
                H::Mirror(on) => self.live_look(None, None, Some(on)),
                H::ShowCamera(on) => self.live_show_camera(on),
                // A person's press opens the privacy page; it requests nothing and resets nothing.
                H::OpenPrivacy(kind) => {
                    let _ = match kind.as_str() {
                        "screen" => permissions::screen_settings(),
                        "camera" => permissions::input_settings(true),
                        _ => permissions::input_settings(false),
                    };
                }
                H::Fps(fps) => self.choose_fps(fps),
                H::CapacityCheck => self.start_capacity_check(),
                H::ChooseWindow => self.menu_action(ctx, super::tray::Action::ChooseWindow),
                H::DismissNotice => self.capture_notice = None,
            }
        }
    }
}

impl eframe::App for State {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        super::event_log::event(
            "ui_exit",
            serde_json::json!({"recording":self.recording,"closeAfterSave":self.close_after_save,"quitAfterSave":self.quit_after_save}),
        );
        if let Some(conn) = &self.connection {
            let _ = conn.action("native-window-closed", serde_json::json!({}));
        }
    }
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0., 0., 0., 0.]
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.styles_configured {
            configure_native_styles(ctx);
            self.styles_configured = true;
        }
        if self.last_theme_choice != Some(self.prefs.theme) {
            self.last_theme_choice = Some(self.prefs.theme);
            ctx.set_theme(match self.prefs.theme {
                1 => egui::ThemePreference::Light,
                2 => egui::ThemePreference::Dark,
                _ => egui::ThemePreference::System,
            });
        }
        #[cfg(target_os = "linux")]
        {
            let active = self.hyprland.refresh(&self.layout, self.prefs.size as f32, self.toolbar_mode);
            if active && !self.hyprland.ready() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if self.reconnecting.as_ref().is_some_and(|h| h.is_finished()) {
            match self.reconnecting.take().unwrap().join() {
                Ok(Ok(Some(conn))) => {
                    let _ = conn.action("native-window-ready", serde_json::json!({"pid":std::process::id()}));
                    let (command, human) = super::connection::commands(conn.clone(), ctx.clone());
                    (self.command, self.human) = (Some(command), Some(human));
                    self.connection = Some(conn);
                    // A new connection publishes afresh: the host forgets capture state when it restarts.
                    self.publisher = None;
                    self.heartbeat.reset();
                    self.library_offline = false;
                    self.reconnect_attempted = false;
                    self.show_toast(Toast::info("Library reconnected"));
                }
                _ => self.show_toast(Toast::error("The Library couldn't reconnect · Your files are safe")),
            }
        }
        if self.library_offline && !self.recording && self.reconnecting.is_none() && !self.reconnect_attempted {
            self.reconnect_attempted = true;
            self.reconnecting = Some(thread::spawn(super::connection::Connection::load));
        }
        if self.reconnecting.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if let Some(command) = &self.command {
            match command.swap(0, Ordering::Relaxed) {
                // Passive: the toolbar returns and inputs with intent show as armed.
                1 => self.show_toolbar(ctx),
                4 => self.request_quit(ctx),
                5 => {
                    self.library_offline = true;
                    if self.recording {
                        super::STOP_REQUEST.store(true, Ordering::Relaxed);
                    }
                    self.show_toast(Toast::error("The Library stopped · Reconnecting · Files stay on disk"));
                }
                7 => {
                    if let (Some(conn), false) = (&self.connection, self.recording) {
                        if let Ok(v) = conn.action("state", serde_json::json!({})) {
                            if let Some(root) = v["root"].as_str() {
                                self.destination = PathBuf::from(root);
                            }
                            self.preferences(&v["settings"]);
                        }
                    }
                }
                6 => {
                    self.heartbeat.reset();
                    self.library_offline = false;
                    self.show_toast(Toast::info("Library reconnected"));
                }
                _ => {}
            }
        }
        self.tray_update(ctx);
        if let Some(deadline) = self.idle_deadline {
            if Instant::now() >= deadline {
                self.quitting = true;
                close_recorder(ctx);
                return;
            } else {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        if !self.positioned {
            #[cfg(target_os = "windows")]
            ctx.send_viewport_cmd(egui::ViewportCommand::ContentProtected(true));
            if !self.toolbar_mode {
                // The root window only hosts the bubble, countdown and pickers: one transparent pixel that
                // takes no clicks, and, where a tray exists, no window at all.
                ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));
                #[cfg(target_os = "macos")]
                if self.tray.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                self.positioned = true;
            } else if let Some(size) = ctx.input(|i| i.viewport().monitor_size) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(default_toolbar_position(size)));
                self.positioned = true;
            }
        }
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            self.toolbar_pos = Some(rect.min);
        }
        // Passive status poll. A content request is made exclusively from a
        // person's Record press, never from a timer.
        if !self.hidden && self.grants_checked.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            self.refresh_grants();
        }
        self.refresh_space();
        self.complete_permissions(ctx);
        self.watch_camera();
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(ctx);
        }
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let result = self.worker.take().unwrap().join();
            self.recording = false;
            self.quit_confirmation = false;
            let finish_quit = self.close_after_save && self.quit_after_save && matches!(&result, Ok(Ok(())));
            self.close_after_save = false;
            self.quit_after_save = false;
            if self.hidden {
                self.release_idle(ctx);
            }
            if self.benchmark {
                if !matches!(&result, Ok(Ok(()))) {
                    self.benchmark_failed.store(true, Ordering::Relaxed);
                    eprintln!("Native benchmark failed: {result:?}");
                }
                self.camera = None;
                self.settings_open = false;
                self.quitting = true;
                close_recorder(ctx);
                return;
            }
            self.finish_take(result);
            if finish_quit {
                self.request_quit(ctx);
            }
        }
        // Space acts as Record only when the toolbar is focused and nothing else wants the key.
        if !self.hidden
            && ctx.input(|i| i.key_pressed(egui::Key::Space))
            && space_starts_record(
                ctx.input(|i| i.viewport().focused == Some(true)),
                ctx.wants_keyboard_input(),
                ctx.memory(|m| m.focused().is_some()),
            )
        {
            self.record_action(ctx);
        }
        self.tick_completion(ctx);
        self.advance_start(ctx);
        self.countdown_ui(ctx);
        if !self.hidden {
            if self.toolbar_mode {
                self.toolbar(ctx);
            }
            self.toast_ui(ctx);
        } else {
            close_present_viewport(ctx, vp(VP_TOAST));
        }
        self.panels(ctx);
        self.poll_discovery();
        if self.camera_discovery.is_some() || self.microphone_discovery.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.settings_ui(ctx);
        self.setup_ui(ctx);
        self.quit_dialog(ctx);
        self.bubble_commands();
        self.bubble(ctx);
        if self.camera.is_none() {
            close_present_viewport(ctx, vp(VP_CAMERA));
        }
        if self.recording || self.start.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.camera.is_some() {
            ctx.request_repaint_after(Duration::from_millis(34));
        } else if self.microphone.is_some() {
            ctx.request_repaint_after(Duration::from_millis(67));
        } else if !self.hidden
            && (!self.screen_ready
                || self.camera_attention.is_some()
                || self.microphone_attention.is_some()
                || self.saved_until.is_some_and(|t| Instant::now() < t))
        {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}
/// `send_viewport_cmd_to` inserts missing viewport state in egui 0.33.3.
/// Read the backend's existing viewport snapshot first; closing an absent child
/// must never manufacture a window just to close it. ROOT uses close_recorder.
fn close_present_viewport(ctx: &egui::Context, id: egui::ViewportId) {
    if ctx.input(|input| input.raw.viewports.contains_key(&id)) {
        ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Close);
    }
}

fn close_recorder(ctx: &egui::Context) {
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
    ctx.request_repaint_of(egui::ViewportId::ROOT);
}

fn countdown_number(remaining: Duration) -> u64 {
    remaining
        .as_secs()
        .saturating_add(u64::from(remaining.subsec_nanos() > 0))
        .clamp(1, 3)
}

/// Proportional cover crop of the delivered frame into a `dw` × `dh` RGBA preview.
fn preview_cover(frame: &super::camera_frame::Frame, mirror: bool, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0; dw * dh * 4];
    if frame.bgra.len() != frame.width * frame.height * 4 {
        return out;
    }
    let (rx, ry, rw, rh) = cover_rect(frame.width, frame.height, dw, dh);
    if rw == 0 || rh == 0 {
        return out;
    }
    for y in 0..dh {
        for x in 0..dw {
            let sx = rx + (if mirror { dw - 1 - x } else { x }) * rw / dw;
            let sy = ry + y * rh / dh;
            let i = (sy * frame.width + sx) * 4;
            let o = (y * dw + x) * 4;
            out[o..o + 4].copy_from_slice(&[frame.bgra[i + 2], frame.bgra[i + 1], frame.bgra[i], 255]);
        }
    }
    out
}
/// Centre-crop the delivered frame to a square (the original 360 x 360 preview).
#[cfg(test)]
fn preview_rgba(frame: &super::camera_frame::Frame, mirror: bool) -> Vec<u8> {
    preview_cover(frame, mirror, 360, 360)
}
// hyprland.rs is Linux-only in the build; its pure planner and tests are compiled here too so macOS runs them.
#[cfg(all(test, unix, not(target_os = "linux")))]
#[path = "hyprland.rs"]
#[allow(dead_code)]
mod hyprland_planner;

#[cfg(test)]
mod fixtures {
    use super::*;
    use std::cell::RefCell;
    thread_local! {
        /// Every OS request the code under test attempted on this thread.
        pub static REQUESTS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    }
    pub fn requests() -> Vec<&'static str> {
        REQUESTS.with(|r| r.borrow().clone())
    }
    pub fn granted() -> GrantSnapshot {
        GrantSnapshot { screen: true, camera: Grant::Allowed, microphone: Grant::Allowed }
    }
    pub fn fake_camera() -> Camera {
        Camera {
            opened: Instant::now(),
            child: None,
            pixels: Arc::new(Mutex::new(super::super::camera_frame::Frame { width: 0, height: 0, bgra: vec![], sequence: 0 })),
            ready: Arc::new(AtomicBool::new(true)),
            last_frame: None,
            error: Arc::new(Mutex::new(None)),
        }
    }
    /// Inert OS: requests are recorded and answered "allowed", nothing is opened
    /// or enumerated, and no real status is read.
    pub fn fake_os() -> Os {
        Os {
            snapshot: granted,
            displays: || (Vec::new(), None),
            window_bounds: |_| None,
            request_screen: || {
                REQUESTS.with(|r| r.borrow_mut().push("screen"));
                Arc::new(AtomicI8::new(1))
            },
            request_camera: || {
                REQUESTS.with(|r| r.borrow_mut().push("camera"));
                Arc::new(AtomicI8::new(1))
            },
            request_microphone: || {
                REQUESTS.with(|r| r.borrow_mut().push("microphone"));
                Arc::new(AtomicI8::new(1))
            },
            open_camera: |_| Ok(fake_camera()),
            open_microphone: |_| Ok(MicSource::Fake),
            list_cameras: || {
                Ok(vec![super::super::devices::CameraDevice { id: "UID-A".into(), name: "Studio Camera".into() }])
            },
            list_microphones: || {
                Ok(vec![super::super::devices::MicrophoneChoice { name: "Built-in".into(), duplicate: false }])
            },
        }
    }
    pub fn test_state() -> State {
        REQUESTS.with(|r| r.borrow_mut().clear());
        // The legacy toolbar is what these helpers exercise; the menu-driven app has its own tests.
        State { os: fake_os(), screen_ready: true, positioned: true, toolbar_mode: true, ..Default::default() }
    }
    pub fn root_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(TOOLBAR_W, TOOLBAR_H))),
            ..Default::default()
        }
    }
    pub fn settle(state: &mut State, ctx: &egui::Context) {
        for _ in 0..300 {
            state.complete_permissions(ctx);
            state.advance_start(ctx);
            if state.start.is_none() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("start never settled");
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn toolbar_slots_are_the_exact_design_numbers() {
        assert_eq!((TOOLBAR_W, TOOLBAR_H), (427., 44.));
        let expected = [
            (Slot::Grip, 6., 10.),
            (Slot::Library, 18., 28.),
            (Slot::Source, 50., 148.),
            (Slot::Camera, 202., 28.),
            (Slot::Mic, 234., 28.),
            (Slot::Divider, 270., 1.),
            (Slot::Record, 279., 76.),
            (Slot::Settings, 359., 28.),
            (Slot::Hide, 391., 28.),
        ];
        assert_eq!(SLOTS, expected);
        let origin = egui::pos2(100., 50.);
        let mut previous_right = 0.;
        for (slot, x, w) in expected {
            let rect = slot_rect(origin, slot);
            assert_eq!(rect.min, origin + egui::vec2(x, 8.), "{slot:?}");
            assert_eq!(rect.size(), egui::vec2(w, 28.), "{slot:?}");
            assert!(rect.min.x - origin.x >= previous_right, "{slot:?} overlaps its neighbour");
            previous_right = rect.max.x - origin.x;
        }
        assert_eq!(TOOLBAR_W - previous_right, 8.);
        // Only controls own a press: grip and divider are drag surface.
        assert_eq!(control_rects(origin).len(), 7);
        assert!(!control_rects(origin).iter().any(|r| r.contains(slot_rect(origin, Slot::Grip).center())));
    }
    #[test]
    fn record_face_text_follows_the_real_phase() {
        assert_eq!(Face::Ready.label(), "● Record");
        assert_eq!(Face::Countdown(2).label(), "2  Cancel");
        assert_eq!(Face::Recording(42).label(), "■ 0:42");
        assert_eq!(Face::Recording(3725).label(), "■ 1:02:05");
        assert_eq!(Face::Saving.label(), "Saving…");
        assert_eq!(Face::Starting.label(), "Starting…");
        assert_eq!(Face::Saved.label(), "✓ Saved");
        assert!(Face::Recording(1).locks_inputs() && Face::Countdown(3).locks_inputs() && Face::Starting.locks_inputs());
        assert!(!Face::Ready.locks_inputs() && !Face::Saved.locks_inputs());
        for face in [Face::Ready, Face::Starting, Face::Countdown(1), Face::Recording(1), Face::Saving, Face::Saved, Face::Unavailable] {
            assert!(!face.accessible_name().is_empty());
        }
    }
    #[test]
    fn marks_separate_intent_from_live_and_attention() {
        assert_eq!(input_mark(false, false, false, false), Mark::None);
        assert_eq!(input_mark(false, false, false, true), Mark::Armed);
        assert_eq!(input_mark(false, true, false, false), Mark::Armed);
        assert_eq!(input_mark(true, false, false, true), Mark::Live);
        assert_eq!(input_mark(false, false, true, true), Mark::Attention);
        // A live source is never shown as attention, and armed never reads as live.
        assert_eq!(input_mark(true, false, true, true), Mark::Live);
    }
    #[test]
    fn cog_is_eight_attached_teeth_around_a_solid_body_with_a_hole() {
        let centre = egui::pos2(100., 100.);
        let teeth = gear_teeth(centre);
        assert_eq!(teeth.len(), 8);
        for tooth in teeth {
            let radii: Vec<f32> = tooth.iter().map(|p| (*p - centre).length()).collect();
            // Base corners sit inside the body: attached, never detached rays.
            assert!(radii[0] < GEAR_BODY_RADIUS && radii[3] < GEAR_BODY_RADIUS, "{radii:?}");
            // Tips stay inside the 16 pt icon box.
            assert!(radii[1] > GEAR_BODY_RADIUS && radii[1] <= 8. && radii[2] <= 8., "{radii:?}");
        }
        assert!(GEAR_HOLE_RADIUS > 0. && GEAR_HOLE_RADIUS < GEAR_BODY_RADIUS);
        // Teeth are evenly spaced.
        let angle = |p: egui::Pos2| (p - centre).angle();
        let step = (angle(teeth[1][1]) - angle(teeth[0][1])).rem_euclid(std::f32::consts::TAU);
        assert!((step - std::f32::consts::TAU / 8.).abs() < 0.2);
    }
    #[test]
    fn library_mark_is_a_plain_black_circle() {
        let (radius, fill, light_edge) = library_mark(false);
        assert_eq!((radius * 2., fill, light_edge), (18., Color32::BLACK, None));
        let (_, fill, dark_edge) = library_mark(true);
        assert_eq!(fill, Color32::BLACK);
        assert_eq!(dark_edge.map(|s| (s.width, s.color)), Some((1., rgb(0x4A4A50))));
    }
    #[test]
    fn bubble_sizes_shapes_and_radii_match_the_design() {
        assert_eq!(SIZES.map(|(s, _)| s), [120, 168, 224]);
        let wide: Vec<(f32, f32)> = SIZES.iter().map(|(s, _)| bubble_dimensions(*s as f32, SHAPE_WIDE)).collect();
        assert_eq!(wide, vec![(160., 90.), (224., 126.), (299., 168.)]);
        for shape in [SHAPE_CIRCLE, SHAPE_ROUNDED, SHAPE_SQUARE] {
            assert_eq!(bubble_dimensions(168., shape), (168., 168.));
        }
        assert_eq!(bubble_radius(SHAPE_CIRCLE, 168., 168.), 84.);
        assert!((bubble_radius(SHAPE_ROUNDED, 168., 168.) - 36.96).abs() < 0.01);
        assert!((bubble_radius(SHAPE_WIDE, 224., 126.) - 25.2).abs() < 0.01);
        assert_eq!(bubble_radius(SHAPE_SQUARE, 168., 168.), 0.);
        // Layout::size stores the base: a wide bubble reads back to the same base.
        for (size, _) in SIZES {
            let (w, _) = bubble_dimensions(size as f32, SHAPE_WIDE);
            assert!((bubble_base(w, SHAPE_WIDE) - size as f32).abs() < 0.5);
        }
    }
    #[test]
    fn legacy_sizes_and_square_stay_valid_and_read_as_s_m_l() {
        for legacy in LEGACY_SIZES {
            assert!(valid_size(legacy as u64));
        }
        assert_eq!([140, 200, 280].map(size_class), [0, 1, 2]);
        assert_eq!([120, 168, 224].map(size_class), [0, 1, 2]);
        assert!(!valid_size(150));
    }
    #[test]
    fn cover_crop_fills_without_stretch_or_letterbox() {
        // 16:9 into a square: crop the sides.
        assert_eq!(cover_rect(1280, 720, 360, 360), (280, 0, 720, 720));
        // 16:9 into wide: whole frame.
        assert_eq!(cover_rect(1280, 720, 480, 270), (0, 0, 1280, 720));
        // Portrait into wide: crop top and bottom.
        let (x, y, w, h) = cover_rect(405, 720, 480, 270);
        assert_eq!((x, w), (0, 405));
        assert!(h < 720 && y > 0 && (w as f32 / h as f32 - 480. / 270.).abs() < 0.02);
        for (sw, sh, dw, dh) in [(1280, 720, 360, 360), (720, 1280, 480, 270), (640, 480, 480, 270), (1000, 1000, 360, 360)] {
            let (x, y, w, h) = cover_rect(sw, sh, dw, dh);
            assert!(x + w <= sw && y + h <= sh && w > 0 && h > 0);
            assert!((w as f32 / h as f32 - dw as f32 / dh as f32).abs() < 0.02, "{sw}x{sh}->{dw}x{dh}");
            // Cover: at least one axis is used whole.
            assert!(w == sw || h == sh);
        }
        assert_eq!(cover_rect(0, 10, 10, 10), (0, 0, 0, 0));
    }
    #[test]
    fn panels_anchor_outside_a_44_pt_toolbar_on_every_monitor_origin() {
        let size = egui::vec2(1512., 982.);
        // Primary, a secondary far to the right, one to the left (negative x) and one above (negative y).
        let monitors = [
            egui::Rect::from_min_size(egui::Pos2::ZERO, size),
            egui::Rect::from_min_size(egui::pos2(1512., 120.), egui::vec2(2560., 1440.)),
            egui::Rect::from_min_size(egui::pos2(-1920., 300.), egui::vec2(1920., 1080.)),
            egui::Rect::from_min_size(egui::pos2(200., -1080.), egui::vec2(1920., 1080.)),
        ];
        for monitor in monitors {
            for local in [
                default_toolbar_position(monitor.size()),
                egui::pos2(300., 6.),
                egui::pos2(0., 400.),
                egui::pos2(monitor.width() - TOOLBAR_W, monitor.height() - 44.),
            ] {
                let toolbar = egui::Rect::from_min_size(monitor.min + local.to_vec2(), egui::vec2(TOOLBAR_W, TOOLBAR_H));
                for (slot, panel) in [(Slot::Source, DEVICE_PANEL), (Slot::Source, WINDOW_PANEL), (Slot::Camera, DEVICE_PANEL), (Slot::Mic, DEVICE_PANEL)] {
                    let rect = egui::Rect::from_min_size(anchor_panel(toolbar, slot_rect(toolbar.min, slot), panel, Some(monitor), false), panel);
                    assert!(panel.y > TOOLBAR_H * 9., "pickers are far taller than the toolbar");
                    assert!(!rect.intersects(toolbar), "{rect:?} overlaps {toolbar:?}");
                    assert!(monitor.contains_rect(rect), "{rect:?} left its monitor {monitor:?}");
                    let gap = if rect.max.y <= toolbar.min.y { toolbar.min.y - rect.max.y } else { rect.min.y - toolbar.max.y };
                    assert!(gap >= PANEL_GAP - 0.01, "gap {gap}");
                }
            }
        }
        // Bottom of a display: above. Near its top: below, on a secondary display too.
        let secondary = monitors[1];
        let low = egui::Rect::from_min_size(secondary.min + default_toolbar_position(secondary.size()).to_vec2(), egui::vec2(TOOLBAR_W, TOOLBAR_H));
        assert!(anchor_panel(low, slot_rect(low.min, Slot::Mic), DEVICE_PANEL, Some(secondary), false).y < low.min.y);
        let high = egui::Rect::from_min_size(secondary.min + egui::vec2(300., 6.), egui::vec2(TOOLBAR_W, TOOLBAR_H));
        assert!(anchor_panel(high, slot_rect(high.min, Slot::Mic), DEVICE_PANEL, Some(secondary), false).y > high.max.y);
        // A zero-origin clamp would have pulled this picker back onto the primary display.
        let right_edge = egui::Rect::from_min_size(secondary.max - egui::vec2(TOOLBAR_W + 4., 80.), egui::vec2(TOOLBAR_W, TOOLBAR_H));
        let placed = anchor_panel(right_edge, slot_rect(right_edge.min, Slot::Hide), DEVICE_PANEL, Some(secondary), false);
        assert!(placed.x >= secondary.min.x && placed.x + DEVICE_PANEL.x <= secondary.max.x);
        // Unknown monitor: placed relative to the toolbar only, never clamped to a guessed edge.
        let unknown = anchor_panel(right_edge, slot_rect(right_edge.min, Slot::Mic), DEVICE_PANEL, None, false);
        assert_eq!(unknown.x, slot_rect(right_edge.min, Slot::Mic).min.x);
        assert!(unknown.y + DEVICE_PANEL.y < right_edge.min.y);
    }
    #[test]
    fn bubble_fractions_are_relative_to_the_monitor_origin_not_to_zero() {
        // 1920-point-wide secondary display starting at desktop x = 1920.
        let secondary = egui::Rect::from_min_size(egui::pos2(1920., 0.), egui::vec2(1920., 1080.));
        let bubble = egui::Rect::from_min_size(egui::pos2(2020., 540.), egui::vec2(168., 168.));
        let (x, y, size) = bubble_layout(bubble, SHAPE_ROUNDED, secondary).unwrap();
        assert!((x - 100. / 1920.).abs() < 1e-6 && (y - 0.5).abs() < 1e-6, "x={x} y={y}");
        assert!((size - 168. / 1920.).abs() < 1e-6);
        // The same bubble against a zero-origin 1920 monitor (the old arithmetic) hits the right wall.
        let wrong = bubble.min.x / 1920.;
        assert!(wrong > 1., "the old division produced {wrong}");
        // A display to the left of the primary (negative origin).
        let left = egui::Rect::from_min_size(egui::pos2(-1920., 100.), egui::vec2(1920., 1080.));
        let (x, y, _) = bubble_layout(egui::Rect::from_min_size(egui::pos2(-1820., 200.), egui::vec2(120., 120.)), SHAPE_CIRCLE, left).unwrap();
        assert!((x - 100. / 1920.).abs() < 1e-6 && (y - 100. / 1080.).abs() < 1e-6);
    }
    #[test]
    fn small_bubbles_keep_their_size_on_large_displays() {
        // The inherited 0.08 floor turned 120 pt into 205 pt on a 2560 pt display.
        let display = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2560., 1440.));
        for (base, shape) in [(120., SHAPE_CIRCLE), (120., SHAPE_ROUNDED), (120., SHAPE_SQUARE)] {
            let bubble = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(base, base));
            let (_, _, size) = bubble_layout(bubble, shape, display).unwrap();
            assert!((size * 2560. - 120.).abs() < 0.01, "{size}");
            assert!(size < 0.08, "no floor may lift it");
        }
        // A wide bubble reports its base, not its wider edge: 160 x 90 is base 120 again.
        let (w, h) = bubble_dimensions(120., SHAPE_WIDE);
        let wide = egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(w, h));
        let (_, _, size) = bubble_layout(wide, SHAPE_WIDE, display).unwrap();
        assert!((size * 2560. - 120.).abs() < 0.5);
        // Sanity bounds still protect the encoder from absurd input.
        let tiny = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1., 1.));
        assert_eq!(bubble_layout(tiny, SHAPE_CIRCLE, display).unwrap().2, 0.01);
        let huge = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(5000., 5000.));
        assert_eq!(bubble_layout(huge, SHAPE_CIRCLE, display).unwrap().2, 0.5);
        assert_eq!(bubble_layout(wide, SHAPE_WIDE, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO)), None);
    }
    #[test]
    fn the_recorded_region_is_the_selected_display_with_its_origin() {
        let displays = vec![
            (1, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1512., 982.))),
            (7, egui::Rect::from_min_size(egui::pos2(1512., -200.), egui::vec2(2560., 1440.))),
        ];
        let secondary = displays[1].1;
        let space = space_for(&displays, Some(1), Some((false, 7)), None);
        let bubble = egui::Rect::from_min_size(egui::pos2(1612., 100.), egui::vec2(120., 120.));
        assert_eq!(layout_region(bubble, &space, None), Some(secondary));
        let (x, y, size) = bubble_layout(bubble, SHAPE_CIRCLE, secondary).unwrap();
        assert!((x - 100. / 2560.).abs() < 1e-6 && (y - 300. / 1440.).abs() < 1e-6 && (size - 120. / 2560.).abs() < 1e-6);
        // No selection: the main display. A window whose rectangle is unavailable: the bubble's own monitor.
        assert_eq!(space_for(&displays, Some(1), None, None).capture, Some(displays[0].1));
        let unknown_window = space_for(&displays, Some(1), Some((true, 9)), None);
        assert_eq!(unknown_window.capture, None);
        assert_eq!(layout_region(bubble, &unknown_window, None), Some(secondary));
        // Selection that is no longer attached: unknown, never invented.
        assert_eq!(space_for(&displays, Some(1), Some((false, 99)), None).capture, None);
        // A window rectangle is never applied to a display selection.
        let stray = egui::Rect::from_min_size(egui::pos2(5., 5.), egui::vec2(50., 50.));
        assert_eq!(space_for(&displays, Some(1), Some((false, 7)), Some(stray)).capture, Some(secondary));
    }
    #[test]
    fn a_selected_window_rectangle_is_the_origin_and_scale_of_the_composite() {
        // 800 x 600 pt window at (2000, 150) on a secondary display; bubble 300 pt in, 90 pt down.
        let displays = vec![
            (1, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1512., 982.))),
            (7, egui::Rect::from_min_size(egui::pos2(1512., -200.), egui::vec2(2560., 1440.))),
        ];
        let window = egui::Rect::from_min_size(egui::pos2(2000., 150.), egui::vec2(800., 600.));
        let space = space_for(&displays, Some(1), Some((true, 42)), Some(window));
        assert_eq!(space.capture, Some(window));
        let bubble = egui::Rect::from_min_size(egui::pos2(2300., 240.), egui::vec2(160., 90.));
        let region = layout_region(bubble, &space, None).unwrap();
        assert_eq!(region, window, "the window, not the monitor, is the recorded area");
        let (x, y, size) = bubble_layout(bubble, SHAPE_WIDE, region).unwrap();
        assert!((x - 300. / 800.).abs() < 1e-6 && (y - 90. / 600.).abs() < 1e-6, "x={x} y={y}");
        // Wide 160 x 90 is base 120: relative to the window width, not the monitor's.
        assert!((size - 120. / 800.).abs() < 1e-6, "{size}");
        // The monitor-relative answer (the previous limit) would have been different.
        assert!((y - (240. + 200.) / 1440.).abs() > 0.1, "monitor-relative y would be {}", (240. + 200.) / 1440.);
    }
    thread_local! {
        static WINDOW_QUERIES: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    fn mocked_window_bounds(id: u32) -> Option<egui::Rect> {
        WINDOW_QUERIES.with(|q| q.borrow_mut().push(id));
        (id == 9).then(|| egui::Rect::from_min_size(egui::pos2(40., 60.), egui::vec2(640., 480.)))
    }
    #[test]
    fn window_rectangles_are_cached_queried_only_for_the_selected_window_and_never_for_displays() {
        let mut state = fixtures::test_state();
        state.os.window_bounds = mocked_window_bounds;
        WINDOW_QUERIES.with(|q| q.borrow_mut().clear());
        let queries = || WINDOW_QUERIES.with(|q| q.borrow().clone());
        // No window selected: nothing is queried, however often it refreshes.
        state.refresh_space();
        state.space_checked = None;
        state.options.selection = Some((false, 3));
        state.refresh_space();
        assert!(queries().is_empty(), "{:?}", queries());
        // Selected window 9: one query now, none within the cache interval (a UI pass runs many times a second).
        state.options.selection = Some((true, 9));
        state.space_checked = None;
        for _ in 0..50 {
            state.refresh_space();
        }
        assert_eq!(queries(), vec![9]);
        assert_eq!(state.space.lock().unwrap().capture, Some(egui::Rect::from_min_size(egui::pos2(40., 60.), egui::vec2(640., 480.))));
        // The window moves: picked up on the next interval, not before.
        state.space_checked = Some(Instant::now() - Duration::from_millis(1100));
        state.refresh_space();
        assert_eq!(queries(), vec![9, 9]);
        // A window that vanished falls back to unknown, not to the last rectangle.
        state.options.selection = Some((true, 77));
        state.space_checked = None;
        state.refresh_space();
        assert_eq!(queries(), vec![9, 9, 77], "only the selected id is ever asked about");
        assert_eq!(state.space.lock().unwrap().capture, None);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn quartz_window_bounds_for_a_window_that_does_not_exist_is_none() {
        // Passive Quartz query by id; an id no window has yields nothing, not a guess.
        assert_eq!(system_window_bounds(u32::MAX), None);
        assert_eq!(system_window_bounds(0), None);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn quartz_display_rectangles_are_passive_and_self_consistent() {
        // Display geometry only: no capture, no permission, no prompt.
        let (displays, main) = system_displays();
        for (_, rect) in &displays {
            assert!(rect.width() > 0. && rect.height() > 0., "{rect:?}");
        }
        if let Some(main) = main.filter(|_| !displays.is_empty()) {
            let (_, rect) = displays.iter().find(|(id, _)| *id == main).expect("the main display is active");
            assert_eq!(rect.min, egui::Pos2::ZERO, "the main display defines the global origin");
        }
        // Displays never overlap, so a bubble is on at most one of them.
        for (i, (_, a)) in displays.iter().enumerate() {
            for (_, b) in displays.iter().skip(i + 1) {
                assert!(a.intersect(*b).area() <= 0.01, "{a:?} overlaps {b:?}");
            }
        }
    }
    #[test]
    fn without_a_monitor_list_only_a_certain_position_is_converted() {
        let inside = egui::Rect::from_min_size(egui::pos2(100., 100.), egui::vec2(168., 168.));
        let reported = Some(egui::vec2(1920., 1080.));
        assert_eq!(monitor_for(inside, &[], reported), Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920., 1080.))));
        // Beyond the reported size: another monitor with an origin this platform doesn't tell us.
        let beyond = egui::Rect::from_min_size(egui::pos2(2020., 100.), egui::vec2(168., 168.));
        assert_eq!(monitor_for(beyond, &[], reported), None);
        assert_eq!(monitor_for(egui::Rect::from_min_size(egui::pos2(-50., 0.), egui::vec2(168., 168.)), &[], reported), None);
        assert_eq!(monitor_for(inside, &[], None), None);
        // Unknown stays unknown all the way to the layout: no region means no write, so an
        // adapter-owned Layout (Hyprland) or the last known one is never overwritten by a guess.
        assert_eq!(layout_region(beyond, &BubbleSpace::default(), reported), None);
        assert_eq!(layout_region(inside, &BubbleSpace::default(), None), None);
        // With a list, a rectangle on no monitor is unknown too.
        let list = [egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000., 1000.))];
        assert_eq!(monitor_for(beyond, &list, reported), None);
    }
    #[test]
    fn source_panel_sizes_match_the_design() {
        assert_eq!(DEVICE_PANEL, egui::vec2(330., 440.));
        assert_eq!(WINDOW_PANEL, egui::vec2(520., 460.));
        assert_eq!(source_panel_size(SourceTab::Window, 3), egui::vec2(520., 460.));
        let display = source_panel_size(SourceTab::Display, 2);
        assert_eq!(display.x, 330.);
        assert!(display.y > TOOLBAR_H && display.y <= 440.);
        assert_eq!(source_panel_size(SourceTab::Display, 40).y, 440.);
    }
    #[test]
    fn bubble_hover_controls_sit_on_the_top_edge_and_stay_inside_every_shape() {
        for (w, h, shape) in [(120., 120., SHAPE_CIRCLE), (160., 90., SHAPE_WIDE), (90., 160., SHAPE_PORTRAIT), (168., 168., SHAPE_CIRCLE), (224., 224., SHAPE_ROUNDED), (299., 168., SHAPE_WIDE), (168., 299., SHAPE_PORTRAIT)] {
            let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
            let inset = capsule_inset(shape, bounds);
            // A bubble under 150 pt on its short side shows one button, then the four shapes; larger ones show four, or five with ×.
            let compact = w.min(h) < 150.;
            for row in [0, 1] {
                for count in if compact { vec![1, 4] } else { vec![4, 5] } {
                    let (capsule, buttons) = capsule_layout(bounds, count, row, inset);
                    assert!(bounds.contains_rect(capsule), "{w}x{h} row {row}: {capsule:?}");
                    assert!(buttons.iter().all(|b| capsule.contains_rect(*b)));
                    // Away from the face: the first row is in the top 42 %, never over the middle.
                    if row == 0 {
                        assert!(capsule.max.y <= bounds.min.y + h * 0.42, "{w}x{h} {shape}: {capsule:?}");
                    }
                }
            }
        }
        // Circle inset is 17 % of the height; every other shape 8 pt.
        let circle = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(168., 168.));
        assert!((capsule_inset(SHAPE_CIRCLE, circle) - 28.56).abs() < 0.01);
        assert_eq!(capsule_inset(SHAPE_WIDE, circle), 8.);
        assert_eq!(capsule_inset(SHAPE_PORTRAIT, circle), 8.);
        // Every corner of the top capsule stays inside a medium circle, not just its box.
        for count in [4, 5] {
            let (capsule, _) = capsule_layout(circle, count, 0, capsule_inset(SHAPE_CIRCLE, circle));
            for corner in [capsule.left_top(), capsule.right_top(), capsule.left_bottom(), capsule.right_bottom()] {
                assert!((corner - circle.center()).length() <= 84.5, "x{count}: {corner:?}");
            }
        }
        // The compact button of a small circle fits its curve too, and the row it opens sits below it.
        let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(120., 120.));
        let inset = capsule_inset(SHAPE_CIRCLE, small);
        let (one, _) = capsule_layout(small, 1, 0, inset);
        for corner in [one.left_top(), one.right_top()] {
            assert!((corner - small.center()).length() <= 60.5, "{corner:?}");
        }
        let (opened, _) = capsule_layout(small, 4, 1, inset);
        assert!(opened.min.y > one.max.y);
        for corner in [opened.left_top(), opened.right_top(), opened.left_bottom(), opened.right_bottom()] {
            assert!((corner - small.center()).length() <= 60.5, "{corner:?}");
        }
        assert_eq!([SHAPE_CIRCLE, SHAPE_ROUNDED, SHAPE_WIDE, SHAPE_PORTRAIT, SHAPE_SQUARE].map(next_shape), [1, 3, 4, 0, 0]);
    }
    #[test]
    fn portrait_is_nine_by_sixteen_in_every_surface_that_draws_the_bubble() {
        assert_eq!(bubble_dimensions(168., SHAPE_PORTRAIT), (126., 224.));
        assert_eq!(bubble_dimensions(168., SHAPE_WIDE), (224., 126.));
        assert_eq!(bubble_dimensions(120., SHAPE_PORTRAIT), (90., 160.));
        assert_eq!(bubble_dimensions(224., SHAPE_PORTRAIT), (168., 299.));
        // What Layout stores is the base: the inverse round-trips for every shape.
        for shape in [SHAPE_CIRCLE, SHAPE_ROUNDED, SHAPE_SQUARE, SHAPE_WIDE, SHAPE_PORTRAIT] {
            let (w, _) = bubble_dimensions(168., shape);
            assert!((bubble_base(w, shape) - 168.).abs() < 1.0, "{shape}");
        }
        assert_eq!(preview_dimensions(SHAPE_PORTRAIT), (270, 480));
        assert_eq!(bubble_radius(SHAPE_PORTRAIT, 126., 224.), 126. * 0.2);
        assert_eq!(shape_from_name("portrait"), SHAPE_PORTRAIT);
        assert_eq!(super::super::connection::shape_name(SHAPE_PORTRAIT), "portrait");
        // Stored choices survive: the legacy square and rounded square are read back as themselves.
        assert_eq!(SettingsValues::from_host(&serde_json::json!({"cameraShape":"square","fps":24}), PathBuf::new()).shape, SHAPE_SQUARE);
        assert_eq!(SettingsValues::from_host(&serde_json::json!({"cameraShape":"rounded"}), PathBuf::new()).shape, SHAPE_ROUNDED);
        assert_eq!(SettingsValues::from_host(&serde_json::json!({"cameraShape":"portrait"}), PathBuf::new()).shape, SHAPE_PORTRAIT);
        let kept = SettingsValues::from_host(&serde_json::json!({"fps":24,"cameraSize":200}), PathBuf::new());
        assert_eq!((kept.fps, kept.size, kept.fps_chosen), (24, 200, true), "a stored 24 fps and legacy size are not remapped, and a rate with no flag counts as chosen");
        assert!(!SettingsValues::from_host(&serde_json::json!({"fps":30,"fpsChosen":false}), PathBuf::new()).fps_chosen, "only an explicit false is unchosen");
        assert!(SettingsValues::from_host(&serde_json::json!({"fps":30,"fpsChosen":true}), PathBuf::new()).fps_chosen);
    }
    #[test]
    fn time_and_space_formats() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(59), "0:59");
        assert_eq!(format_elapsed(3600), "1:00:00");
        assert_eq!(format_free(1_400_000_000), "1.4 GB");
        assert!(MIN_FREE_BYTES > 1_400_000_000);
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;

    fn devices() -> Vec<(String, String, bool)> {
        vec![
            ("UID-A".into(), "USB Camera".into(), true),
            ("UID-B3F2".into(), "USB Camera".into(), true),
            ("UID-C".into(), "Studio".into(), false),
        ]
    }
    #[test]
    fn device_rows_mark_the_selection_and_disambiguate_only_real_duplicates() {
        let model = device_model(true, &devices(), Some("UID-C"), None, true, true, "");
        assert_eq!(model.rows[0].title, "No Camera");
        assert_eq!(model.rows[1].title, "System default");
        let selected: Vec<&str> = model.rows.iter().filter(|r| r.selected).map(|r| r.title.as_str()).collect();
        assert_eq!(selected, vec!["Studio"]);
        assert_eq!(model.rows[2].subtitle.as_deref(), Some("ID …UIDA"));
        assert_eq!(model.rows[3].subtitle.as_deref(), Some("ID …B3F2"));
        assert_eq!(model.rows[4].subtitle, None);
        // Off: only "No Camera" is checked.
        let off = device_model(true, &devices(), Some("UID-C"), None, true, false, "");
        assert_eq!(off.rows.iter().filter(|r| r.selected).map(|r| r.title.as_str()).collect::<Vec<_>>(), vec!["No Camera"]);
        assert!(model.remembered.is_none());
    }
    #[test]
    fn a_missing_remembered_device_is_disabled_unselected_and_never_replaced() {
        let model = device_model(true, &devices(), Some("UID-GONE"), Some("Logitech C920"), true, true, "");
        let remembered = model.remembered.expect("remembered row");
        assert!(!remembered.enabled && !remembered.selected && remembered.choice.is_none());
        assert_eq!(remembered.title, "Logitech C920");
        assert_eq!(remembered.subtitle.as_deref(), Some("Logitech C920 isn't connected. Choose a replacement — nothing is picked for you."));
        // Nothing is preselected: no row is checked, not even a same-named device.
        assert!(model.rows.iter().all(|r| !r.selected));
        // Before a listing exists nothing is claimed missing.
        assert!(device_model(true, &[], Some("UID-GONE"), None, false, true, "").remembered.is_none());
    }
    #[test]
    fn microphones_with_the_same_name_are_listed_but_cannot_be_chosen() {
        let mics = vec![
            ("Built-in".to_owned(), "Built-in".to_owned(), false),
            ("USB Mic".to_owned(), "USB Mic".to_owned(), true),
        ];
        let model = device_model(false, &mics, Some("USB Mic"), None, true, true, "");
        let usb = model.rows.iter().find(|r| r.title == "USB Mic").unwrap();
        assert!(!usb.enabled);
        assert_eq!(usb.subtitle.as_deref(), Some("Several microphones share this name"));
        assert!(model.rows.iter().find(|r| r.title == "Built-in").unwrap().enabled);
        assert_eq!(model.rows[0].title, "No Microphone");
        // The model never invents a stable identity: the id is the name.
        assert_eq!(usb.choice, Some(DeviceChoice::Device("USB Mic".into())));
    }
    #[test]
    fn device_search_filters_by_title() {
        let model = device_model(true, &devices(), None, None, true, true, "stud");
        assert_eq!(model.rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), vec!["Studio"]);
    }
    #[test]
    fn source_cards_never_invent_previews_and_filter_by_kind_and_search() {
        let targets = vec![
            (false, 1, "Built-in Display".to_owned()),
            (true, 7, "Notes — Meeting".to_owned()),
            (true, 9, "Browser".to_owned()),
        ];
        let windows = source_cards(&targets, true, Some((true, 9)), "");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].monogram, 'N');
        assert!(windows[1].selected && !windows[0].selected);
        assert_eq!(source_cards(&targets, true, None, "brow").len(), 1);
        assert_eq!(source_cards(&targets, false, None, "").len(), 1);
        assert_eq!(monogram("  ·· zed"), 'Z');
        assert_eq!(monogram("—"), '•');
        assert!(capability_line(SourceTab::Display).contains("can appear"));
    }
    #[test]
    fn device_notice_explains_without_requesting_anything() {
        let (text, action) = device_notice(true, Grant::Denied, false, None);
        assert!(text.unwrap().contains("Camera access is off"));
        assert_eq!(action, Some(ToastAction::OpenCameraSettings));
        let (text, action) = device_notice(false, Grant::NotAsked, false, None);
        assert!(text.unwrap().contains("never asks"));
        assert_eq!(action, None);
        assert_eq!(device_notice(true, Grant::Allowed, true, None).0.as_deref(), Some("Finding cameras…"));
        assert_eq!(device_notice(true, Grant::Allowed, false, None), (None, None));
    }
    #[test]
    fn setup_rows_offer_allow_only_before_any_answer() {
        assert_eq!(setup_row_action(Grant::NotAsked), RowAction::Allow);
        assert_eq!(setup_row_action(Grant::Denied), RowAction::OpenSettings);
        assert_eq!(setup_row_action(Grant::Restricted), RowAction::OpenSettings);
        assert_eq!(setup_row_action(Grant::Allowed), RowAction::None);
        assert_eq!(screen_row_action(false, false), RowAction::Allow);
        assert_eq!(screen_row_action(false, true), RowAction::OpenSettings);
        assert_eq!(screen_row_action(true, true), RowAction::None);
    }
    #[test]
    fn space_is_ignored_in_text_fields_and_when_a_control_has_focus() {
        assert!(space_starts_record(true, false, false));
        assert!(!space_starts_record(false, false, false));
        assert!(!space_starts_record(true, true, false));
        assert!(!space_starts_record(true, false, true));
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    fn armed(camera: bool, grant: Grant, remembered: Option<&str>, presence: Presence) -> ArmedInput {
        ArmedInput {
            camera,
            wants_open: true,
            grant,
            remembered: remembered.map(str::to_owned),
            remembered_name: remembered.map(|_| "Logitech C920".into()),
            presence,
        }
    }
    #[test]
    fn nothing_armed_means_nothing_to_check() {
        let mut idle = armed(true, Grant::Denied, Some("UID"), Presence::Listed(vec![]));
        idle.wants_open = false;
        assert_eq!(evaluate_armed(&[idle]), Ok(()));
    }
    #[test]
    fn grants_come_first_and_never_resolve_to_a_prompt() {
        assert_eq!(evaluate_armed(&[armed(true, Grant::Denied, None, Presence::Unchecked)]), Err(InputProblem::AccessOff { camera: true }));
        assert_eq!(evaluate_armed(&[armed(false, Grant::Restricted, None, Presence::Unchecked)]), Err(InputProblem::Restricted { camera: false }));
        assert_eq!(evaluate_armed(&[armed(true, Grant::NotAsked, None, Presence::Unchecked)]), Err(InputProblem::NeedsAccess { camera: true }));
        // The camera is reported before the mic.
        let both = [
            armed(false, Grant::Denied, None, Presence::Unchecked),
            armed(true, Grant::Denied, None, Presence::Unchecked),
        ];
        assert_eq!(evaluate_armed(&both), Err(InputProblem::AccessOff { camera: false }));
        assert_eq!(evaluate_armed(&[armed(true, Grant::Allowed, None, Presence::Unchecked)]), Ok(()));
    }
    #[test]
    fn a_missing_remembered_uid_is_not_resolved_by_name() {
        // A different device with the very same display name is present.
        let present = Presence::Listed(vec![("UID-OTHER".into(), "Logitech C920".into())]);
        let result = evaluate_armed(&[armed(true, Grant::Allowed, Some("UID-C920"), present)]);
        assert_eq!(result, Err(InputProblem::Missing { camera: true, name: Some("Logitech C920".into()) }));
        let found = Presence::Listed(vec![("UID-C920".into(), "Logitech C920".into())]);
        assert_eq!(evaluate_armed(&[armed(true, Grant::Allowed, Some("UID-C920"), found)]), Ok(()));
    }
    #[test]
    fn microphones_match_exactly_and_duplicates_are_rejected() {
        let one = Presence::Listed(vec![("USB Mic".into(), "USB Mic".into())]);
        assert_eq!(evaluate_armed(&[armed(false, Grant::Allowed, Some("USB Mic"), one)]), Ok(()));
        let none = Presence::Listed(vec![("USB Mic 2".into(), "USB Mic 2".into())]);
        assert!(matches!(evaluate_armed(&[armed(false, Grant::Allowed, Some("USB Mic"), none)]), Err(InputProblem::Missing { camera: false, .. })));
        let two = Presence::Listed(vec![("USB Mic".into(), "USB Mic".into()), ("USB Mic".into(), "USB Mic".into())]);
        assert_eq!(evaluate_armed(&[armed(false, Grant::Allowed, Some("USB Mic"), two)]), Err(InputProblem::Duplicate { name: "USB Mic".into() }));
        // A same-named camera pair is not a duplicate problem: cameras have stable UIDs.
        let cams = Presence::Listed(vec![("A".into(), "Cam".into()), ("B".into(), "Cam".into())]);
        assert_eq!(evaluate_armed(&[armed(true, Grant::Allowed, Some("A"), cams)]), Ok(()));
    }
    #[test]
    fn a_failed_listing_is_reported_not_guessed() {
        let result = evaluate_armed(&[armed(true, Grant::Allowed, Some("UID"), Presence::ListFailed("ffmpeg missing".into()))]);
        assert_eq!(result, Err(InputProblem::Failed { camera: true, detail: "ffmpeg missing".into() }));
    }
    #[test]
    fn problems_read_short_and_carry_one_fix() {
        let missing = InputProblem::Missing { camera: true, name: Some("Logitech C920".into()) };
        assert_eq!(missing.message(), "Logitech C920 isn't connected");
        assert_eq!(missing.action(), Some(ToastAction::ChooseCamera));
        assert_eq!(missing.toast(true).secondary, Some(ToastAction::TurnOffCamera));
        let off = InputProblem::AccessOff { camera: false };
        assert_eq!((off.message().as_str(), off.action()), ("Mic access is off", Some(ToastAction::OpenMicSettings)));
        assert_eq!(off.toast(true).secondary, Some(ToastAction::TurnOffMicrophone));
        assert_eq!(InputProblem::Missing { camera: false, name: None }.message(), "Your mic isn't connected");
        assert_eq!(ToastAction::ChooseCamera.label(), "Choose…");
        assert!(InputProblem::Failed { camera: true, detail: "raw".into() }.toast(false).details.is_some());
    }
}

#[cfg(test)]
mod toast_tests {
    use super::*;

    #[test]
    fn info_lasts_four_seconds_and_errors_eight() {
        let info = Toast::info("Saved");
        let error = Toast::error("Couldn't");
        assert_eq!(info.expires - info.last_tick, Duration::from_secs(4));
        assert_eq!(error.expires - error.last_tick, Duration::from_secs(8));
    }
    #[test]
    fn hovering_pauses_expiry() {
        let mut toast = Toast::info("Saved");
        let start = toast.last_tick;
        assert!(toast.tick(start + Duration::from_secs(3), false));
        // Three more seconds under the pointer do not count.
        assert!(toast.tick(start + Duration::from_secs(6), true));
        assert!(toast.tick(start + Duration::from_secs(6) + Duration::from_millis(900), false));
        assert!(!toast.tick(start + Duration::from_secs(8), false));
    }
    #[test]
    fn a_take_that_may_capture_the_pill_defers_non_critical_pills() {
        let mut state = State::default();
        state.recording = true;
        state.show_toast(Toast::info("Library reconnected"));
        assert!(state.toast.is_none() && state.deferred_toast.is_some());
        state.show_toast(Toast::error("Recording stopped"));
        assert!(state.toast.is_some_and(|t| t.error));
        // A window recording is isolated, so pills may show.
        let mut window = State::default();
        window.recording = true;
        window.options.selection = Some((true, 5));
        window.show_toast(Toast::info("Library reconnected"));
        assert!(window.toast.is_some());
    }
    #[test]
    fn worker_errors_become_short_pills_with_raw_text_behind_details() {
        let early = friendly_worker_error("raw failure 0x1", 0, 0);
        assert_eq!(early.text, "Couldn't start recording");
        assert_eq!(early.details.as_deref(), Some("raw failure 0x1"));
        let camera = friendly_worker_error("Camera disconnected. Partial files are preserved.", 900, 198);
        assert!(camera.text.starts_with("Recording stopped — camera disconnected"));
        assert!(camera.text.contains("3:18"));
        assert!(!early.details_open && early.error);
    }
}

#[cfg(test)]
mod theme_geometry_tests {
    use super::*;

    fn draw_settings_sample(
        ctx: &egui::Context,
        theme: egui::ThemePreference,
        system: egui::Theme,
        viewport: egui::ViewportId,
    ) -> Vec<egui::Rect> {
        ctx.set_theme(theme);
        let mut bounds = Vec::new();
        // Identical input/scene, including a stable layout pass for each theme.
        for _ in 0..2 {
            let mut raw = egui::RawInput {
                viewport_id: viewport,
                system_theme: Some(system),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(760., 600.))),
                ..Default::default()
            };
            if viewport != egui::ViewportId::ROOT {
                raw.viewports.insert(viewport, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), ..Default::default() });
            }
            ctx.begin_pass(raw);
            bounds.clear();
            let t = tokens_for(ctx);
            egui::SidePanel::left("settings-navigation")
                .exact_width(200.)
                .resizable(false)
                .frame(egui::Frame::new().inner_margin(12))
                .show(ctx, |ui| {
                    ui.add_space(8.);
                    bounds.push(ui.heading("Settings").rect);
                    ui.add_space(14.);
                    bounds.extend(settings_nav(ui, &t, &mut 0));
                });
            egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(24)).show(ctx, |ui| {
                bounds.push(ui.heading("General").rect);
                bounds.push(ui.label("Appearance").rect);
                ui.horizontal(|ui| {
                    let mut selected = 0;
                    for (value, label) in [(0, "System"), (1, "Light"), (2, "Dark")] {
                        bounds.push(ui.selectable_value(&mut selected, value, label).rect);
                    }
                });
                bounds.push(ui.button("Change…").rect);
                bounds.push(ui.small("Changes apply immediately.").rect);
            });
            let _ = ctx.end_pass();
        }
        bounds
    }

    #[test]
    fn settings_widget_bounds_match_light_dark_system_and_child_viewports() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        ctx.set_theme(egui::Theme::Dark);
        configure_native_styles(&ctx);
        let root = egui::ViewportId::ROOT;
        let child = vp(VP_SETTINGS);
        let light = draw_settings_sample(&ctx, egui::ThemePreference::Light, egui::Theme::Light, root);
        for (preference, system, viewport) in [
            (egui::ThemePreference::Dark, egui::Theme::Light, root),
            (egui::ThemePreference::Light, egui::Theme::Light, child),
            (egui::ThemePreference::Dark, egui::Theme::Light, child),
            (egui::ThemePreference::System, egui::Theme::Dark, child),
            (egui::ThemePreference::System, egui::Theme::Light, child),
            (egui::ThemePreference::Light, egui::Theme::Dark, root),
        ] {
            let actual = draw_settings_sample(&ctx, preference, system, viewport);
            assert_eq!(actual, light, "Widget bounds changed for {preference:?}, system {system:?}, viewport {viewport:?}");
        }
        assert!(ctx.style_of(egui::Theme::Dark).visuals.dark_mode);
        assert!(!ctx.style_of(egui::Theme::Light).visuals.dark_mode);
        assert_ne!(ctx.style_of(egui::Theme::Dark).visuals.panel_fill, ctx.style_of(egui::Theme::Light).visuals.panel_fill);
        // The design tokens are installed in both styles.
        assert_eq!(ctx.style_of(egui::Theme::Light).visuals.panel_fill, tokens(false).surface);
        assert_eq!(ctx.style_of(egui::Theme::Dark).visuals.panel_fill, tokens(true).surface);
    }

    fn run_toolbar(state: &mut State, ctx: &egui::Context, theme: egui::ThemePreference, ppp: f32) -> Vec<(Slot, egui::Rect)> {
        ctx.set_theme(theme);
        ctx.set_pixels_per_point(ppp);
        for _ in 0..2 {
            let mut raw = fixtures::root_input();
            raw.system_theme = Some(egui::Theme::Light);
            ctx.begin_pass(raw);
            state.toolbar(ctx);
            let output = ctx.end_pass();
            assert_eq!(output.viewport_output.len(), 1, "the toolbar must not create windows");
        }
        state.toolbar_bounds.clone()
    }

    #[test]
    fn toolbar_geometry_is_identical_in_both_themes_at_every_scale_and_state() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        configure_native_styles(&ctx);
        let expected: Vec<(Slot, egui::Rect)> = [Slot::Library, Slot::Source, Slot::Camera, Slot::Mic, Slot::Record, Slot::Settings, Slot::Hide]
            .into_iter()
            .map(|slot| (slot, slot_rect(egui::Pos2::ZERO, slot)))
            .collect();
        let states: Vec<(&str, Box<dyn Fn(&mut State)>)> = vec![
            ("idle", Box::new(|_| {})),
            ("armed", Box::new(|s| {
                s.camera_intent = true;
                s.microphone_intent = true;
            })),
            ("live", Box::new(|s| {
                s.camera = Some(fixtures::fake_camera());
                s.microphone = Some(MicSource::Fake);
                s.options.selection = Some((true, 3));
                s.targets = vec![(true, 3, "A very long window title that must truncate instead of growing its slot".into())];
            })),
            ("attention", Box::new(|s| {
                s.camera_attention = Some(InputProblem::AccessOff { camera: true });
            })),
            ("countdown", Box::new(|s| s.countdown = Some(Instant::now() + Duration::from_secs(2)))),
            ("recording", Box::new(|s| {
                s.recording = true;
                s.options.progress.phase.store(1, Ordering::Relaxed);
                s.options.progress.fps.store(30, Ordering::Relaxed);
                s.options.progress.frames.store(30 * 3725, Ordering::Relaxed);
            })),
            ("saving", Box::new(|s| {
                s.recording = true;
                s.options.progress.phase.store(2, Ordering::Relaxed);
            })),
            ("saved", Box::new(|s| s.saved_until = Some(Instant::now() + Duration::from_secs(1)))),
        ];
        for (name, setup) in &states {
            for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
                for ppp in [1., 1.25, 2.] {
                    let mut state = fixtures::test_state();
                    setup(&mut state);
                    assert_eq!(run_toolbar(&mut state, &ctx, theme, ppp), expected, "{name} {theme:?} {ppp}");
                }
            }
        }
    }
    #[test]
    fn toolbar_labels_give_every_control_a_name_and_state() {
        let mut state = fixtures::test_state();
        assert_eq!(state.input_label(true), "Camera, off");
        state.camera_intent = true;
        assert_eq!(state.input_label(true), "Camera, armed, not opened");
        state.camera = Some(fixtures::fake_camera());
        assert_eq!(state.input_label(true), "Camera, on");
        state.microphone_attention = Some(InputProblem::Missing { camera: false, name: None });
        assert_eq!(state.input_label(false), "Mic, needs attention");
        // Live is green only for an actually open source; armed alone is not live.
        assert_eq!(state.mark(true), Mark::Live);
        state.camera = None;
        assert_eq!(state.mark(true), Mark::Armed);
    }

    fn settings_extent(state: &mut State, ctx: &egui::Context, pane: usize, theme: egui::ThemePreference, ppp: f32) -> egui::Vec2 {
        ctx.set_theme(theme);
        ctx.set_pixels_per_point(ppp);
        state.settings_pane = pane;
        let mut extent = egui::Vec2::ZERO;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560., 560.))),
                ..Default::default()
            };
            ctx.begin_pass(raw);
            let t = tokens_for(ctx);
            let (mut draft, mut path, mut commit) = (state.prefs.clone(), String::new(), false);
            egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(28, 20))).show(ctx, |ui| {
                state.settings_contents(ui, &t, &mut draft, &mut path, &mut commit);
                extent = ui.min_rect().size();
            });
            let _ = ctx.end_pass();
        }
        extent
    }
    #[test]
    fn every_settings_pane_keeps_its_geometry_across_themes_and_fits_the_content_width() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        configure_native_styles(&ctx);
        for pane in 0..SETTINGS_PANES.len() {
            let mut state = fixtures::test_state();
            let light = settings_extent(&mut state, &ctx, pane, egui::ThemePreference::Light, 1.);
            let dark = settings_extent(&mut state, &ctx, pane, egui::ThemePreference::Dark, 1.);
            assert_eq!(light, dark, "pane {pane}");
            assert!(light.x <= 504.5, "pane {pane} is {light:?} wide");
            let retina = settings_extent(&mut state, &ctx, pane, egui::ThemePreference::Light, 2.);
            assert!((retina.x - light.x).abs() < 2. && (retina.y - light.y).abs() < 24., "pane {pane}: {light:?} vs {retina:?}");
        }
        // The window is 760 wide with a 200 sidebar and 28 margins: 504 for content.
        assert_eq!(760. - 200. - 2. * 28., 504.);
    }
    #[test]
    fn pickers_fit_their_windows_in_both_themes() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        configure_native_styles(&ctx);
        let many: Vec<SourceCard> = (0..30)
            .map(|i| SourceCard { target: (true, i), title: format!("Window number {i} with a rather long title"), monogram: 'W', selected: i == 4 })
            .collect();
        let displays = vec![SourceCard { target: (false, 1), title: "Built-in Retina Display".into(), monogram: 'B', selected: false }];
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(theme);
            for (size, tab) in [(WINDOW_PANEL, SourceTab::Window), (source_panel_size(SourceTab::Display, 2), SourceTab::Display)] {
                let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
                ctx.begin_pass(raw);
                let t = tokens_for(&ctx);
                let view = SourceView { tab, displays: displays.clone(), windows: many.clone(), screen_ready: true, system_picker: false, default_selected: true };
                let mut search = String::new();
                let mut used = egui::Rect::NOTHING;
                egui::CentralPanel::default().frame(panel_frame(&t)).show(&ctx, |ui| {
                    source_panel_ui(ui, &t, &view, &mut search);
                    used = ui.min_rect();
                });
                let _ = ctx.end_pass();
                assert!(used.max.x <= size.x + 0.5 && used.max.y <= size.y + 0.5, "{tab:?} {theme:?}: {used:?} in {size:?}");
                assert!(size.y > TOOLBAR_H);
            }
            let cameras: Vec<(String, String, bool)> = vec![("A".into(), "Camera".into(), false); 40];
            let model = device_model(true, &cameras, None, None, true, false, "");
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, DEVICE_PANEL)), ..Default::default() };
            ctx.begin_pass(raw);
            let t = tokens_for(&ctx);
            let view = DeviceView { camera: true, model: &model, notice: None, settings_hint: None };
            let mut search = String::new();
            let mut used = egui::Rect::NOTHING;
            egui::CentralPanel::default().frame(panel_frame(&t)).show(&ctx, |ui| {
                device_panel_ui(ui, &t, &view, &mut search);
                used = ui.min_rect();
            });
            let _ = ctx.end_pass();
            assert!(used.max.x <= DEVICE_PANEL.x + 0.5 && used.max.y <= DEVICE_PANEL.y + 0.5, "{theme:?}: {used:?}");
        }
    }
}

#[cfg(test)]
mod viewport_lifecycle_tests {
    use super::fixtures::*;
    use super::*;
    use eframe::App;

    fn assert_root_only(output: &egui::FullOutput) {
        assert_eq!(
            output.viewport_output.len(),
            1,
            "Unexpected child viewport: {:?}",
            output.viewport_output.keys().collect::<Vec<_>>()
        );
        assert!(output.viewport_output.contains_key(&egui::ViewportId::ROOT));
    }

    #[test]
    fn every_child_window_is_named_once_and_closes_through_the_present_guard() {
        let production = include_str!("desktop.rs").split("#[cfg(test)]\nmod fixtures").next().unwrap();
        assert_eq!(production.matches("ViewportId::from_hash_of(").count(), 1, "only vp() may hash ids");
        for name in CHILD_VIEWPORTS {
            assert!(production.contains(&format!("const {}: &str = \"{name}\"", format!("VP_{}", name.to_uppercase().replace('-', "_")).replace("VP_NOTICE", "VP_TOAST").replace("VP_SOURCE_PANEL", "VP_SOURCE").replace("VP_DEVICE_PANEL", "VP_DEVICE").replace("VP_QUIT_CONFIRMATION", "VP_QUIT"))), "{name}");
        }
        let unique: std::collections::HashSet<_> = CHILD_VIEWPORTS.iter().collect();
        assert_eq!(unique.len(), CHILD_VIEWPORTS.len());
        // No ad-hoc viewport id and no raw Close command outside the guard.
        assert!(!production.contains("vp(\""), "viewports must be addressed through constants");
        let closes = production.matches("egui::ViewportCommand::Close").count();
        assert_eq!(closes, 2, "the guard and close_recorder are the only Close senders");
        for name in CHILD_VIEWPORTS {
            assert!(production.contains(&format!("vp({})", format!("VP_{}", name.to_uppercase().replace('-', "_")).replace("VP_NOTICE", "VP_TOAST").replace("VP_SOURCE_PANEL", "VP_SOURCE").replace("VP_DEVICE_PANEL", "VP_DEVICE").replace("VP_QUIT_CONFIRMATION", "VP_QUIT"))), "{name} is never closed");
        }
    }

    #[test]
    fn closing_every_absent_child_creates_nothing() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        for _ in 0..3 {
            ctx.begin_pass(root_input());
            close_children_except(&ctx, &[]);
            for name in CHILD_VIEWPORTS {
                close_present_viewport(&ctx, vp(name));
            }
            assert_root_only(&ctx.end_pass());
        }
    }

    #[test]
    fn repeated_idle_app_frames_and_cleanup_do_not_create_dormant_children() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let repaints = Arc::new(Mutex::new(Vec::new()));
        let recorded = repaints.clone();
        ctx.set_request_repaint_callback(move |info| recorded.lock().unwrap().push(info.viewport_id));
        let mut state = test_state();
        let mut frame = eframe::Frame::_new_kittest();
        for pass in 0..12 {
            let input = root_input();
            assert_eq!(input.viewports.len(), 1);
            ctx.begin_pass(input);
            state.update(&ctx, &mut frame);
            if pass == 3 {
                state.hide(&ctx);
            }
            if pass == 5 {
                state.release_idle(&ctx);
            }
            if pass == 7 {
                state.countdown = Some(Instant::now() + Duration::from_secs(3));
                state.record_press(&ctx, granted()); // Cancel before a HUD ever existed.
            }
            let output = ctx.end_pass();
            assert_root_only(&output);
            assert!(!output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::Close)));
            // end_pass can prune orphan state. Also check the integration-facing
            // repaint callbacks, which send_viewport_cmd_to triggers immediately.
            assert!(repaints.lock().unwrap().iter().all(|id| *id == egui::ViewportId::ROOT));
            assert!(state.worker.is_none());
        }
        assert!(requests().is_empty(), "idle frames made OS requests: {:?}", requests());
    }

    #[test]
    fn cleanup_closes_present_camera_then_does_not_recreate_it_after_removal() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let camera = vp(VP_CAMERA);
        let mut input = root_input();
        input.viewports.insert(camera, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), ..Default::default() });
        ctx.begin_pass(input);
        // Register a real egui child, without any native renderer or device.
        ctx.show_viewport_deferred(camera, egui::ViewportBuilder::default().with_title("Camera fixture"), |_, _| {});
        let mut state = test_state();
        state.release_idle(&ctx);
        let output = ctx.end_pass();
        assert_eq!(output.viewport_output.len(), 2);
        assert!(output.viewport_output[&camera].commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::Close)));
        let mut frame = eframe::Frame::_new_kittest();
        for _ in 0..3 {
            ctx.begin_pass(root_input());
            state.update(&ctx, &mut frame);
            assert_root_only(&ctx.end_pass());
        }
    }

    #[test]
    fn every_new_child_present_in_the_snapshot_is_closed_on_cleanup() {
        for name in [VP_SOURCE, VP_DEVICE, VP_TOAST, VP_SETUP, VP_SETTINGS, VP_QUIT, VP_COUNTDOWN] {
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(false);
            let child = vp(name);
            let mut input = root_input();
            input.viewports.insert(child, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), ..Default::default() });
            ctx.begin_pass(input);
            ctx.show_viewport_deferred(child, egui::ViewportBuilder::default().with_title("fixture"), |_, _| {});
            test_state().release_idle(&ctx);
            let output = ctx.end_pass();
            assert!(output.viewport_output[&child].commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::Close)), "{name}");
        }
    }

    #[test]
    fn a_live_camera_adds_only_the_bubble_and_hide_while_recording_keeps_it() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut state = test_state();
        state.camera = Some(fake_camera());
        let mut frame = eframe::Frame::_new_kittest();
        ctx.begin_pass(root_input());
        state.update(&ctx, &mut frame);
        let output = ctx.end_pass();
        assert_eq!(output.viewport_output.len(), 2);
        assert!(output.viewport_output.contains_key(&vp(VP_CAMERA)));
        // Hide during a take: toolbar goes, bubble stays, nothing is released.
        state.recording = true;
        state.hide(&ctx);
        assert!(state.hidden && state.camera.is_some());
        ctx.begin_pass(root_input());
        state.update(&ctx, &mut frame);
        let output = ctx.end_pass();
        assert!(output.viewport_output.contains_key(&vp(VP_CAMERA)));
    }

    #[test]
    fn expired_idle_mode_closes_root_and_cannot_turn_into_hide() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut state = State { idle_deadline: Some(Instant::now()), ..test_state() };
        let mut frame = eframe::Frame::_new_kittest();
        for close_requested in [false, true] {
            let mut input = root_input();
            if close_requested {
                input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
            }
            ctx.begin_pass(input);
            state.update(&ctx, &mut frame);
            let output = ctx.end_pass();
            assert_root_only(&output);
            let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
            assert!(commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::Close)));
            assert!(!commands.iter().any(|cmd| matches!(
                cmd,
                egui::ViewportCommand::CancelClose | egui::ViewportCommand::Minimized(true) | egui::ViewportCommand::Visible(false)
            )));
            assert!(state.quitting && !state.hidden && state.worker.is_none());
        }
    }
}

#[cfg(test)]
mod input_flow_tests {
    use super::fixtures::*;
    use super::*;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        ctx
    }
    fn press(state: &mut State, ctx: &egui::Context, grants: GrantSnapshot) {
        state.record_press(ctx, grants);
    }

    #[test]
    fn denied_input_gets_a_pill_every_click_and_never_another_os_request() {
        let mut state = test_state();
        for _ in 0..2 {
            state.toast = None;
            state.click_input(true, Grant::Denied);
            let toast = state.toast.as_ref().expect("a pill each time");
            assert_eq!(toast.text, "Camera access is off");
            assert_eq!(toast.action, Some(ToastAction::OpenCameraSettings));
            assert_eq!(state.mark(true), Mark::Attention);
        }
        state.click_input(false, Grant::Restricted);
        assert!(requests().is_empty(), "{:?}", requests());
        assert!(state.camera.is_none() && state.camera_permission.is_none() && !state.camera_intent);
    }
    #[test]
    fn a_click_arms_and_requests_once_and_the_device_opens_only_after_the_answer() {
        let ctx = ctx();
        let mut state = test_state();
        state.click_input(true, Grant::NotAsked);
        assert_eq!(requests(), vec!["camera"]);
        assert!(state.camera_intent && state.camera.is_none() && state.camera_permission.is_some());
        assert_eq!(state.mark(true), Mark::Armed);
        state.complete_permissions(&ctx);
        assert!(state.camera.is_some() && state.camera_permission.is_none());
        assert_eq!(state.mark(true), Mark::Live);
        // Clicking a live input turns it off and clears intent.
        state.click_input(true, Grant::Allowed);
        assert!(state.camera.is_none() && !state.camera_intent);
        assert_eq!(requests(), vec!["camera"]);
    }
    #[test]
    fn a_refusal_from_the_os_clears_the_click_and_explains() {
        let ctx = ctx();
        let mut state = test_state();
        state.microphone_permission = Some(Arc::new(AtomicI8::new(-1)));
        state.microphone_intent = true;
        state.complete_permissions(&ctx);
        assert!(!state.microphone_intent && state.microphone.is_none());
        assert_eq!(state.toast.as_ref().map(|t| t.text.as_str()), Some("Mic access is off"));
    }
    #[test]
    fn record_with_a_denied_armed_input_starts_nothing_and_requests_nothing() {
        let ctx = ctx();
        let mut state = test_state();
        state.camera_intent = true;
        press(&mut state, &ctx, GrantSnapshot { camera: Grant::Denied, ..granted() });
        assert!(state.start.is_none() && state.countdown.is_none() && state.worker.is_none() && !state.recording);
        assert!(state.camera.is_none() && state.camera_permission.is_none());
        assert!(requests().is_empty());
        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.text, "Camera access is off");
        assert_eq!(toast.secondary, Some(ToastAction::TurnOffCamera));
        assert!(state.camera_intent, "intent is kept until the person turns it off");
        // Turning the input off is the explicit way to proceed.
        state.toast_action(ToastAction::TurnOffCamera);
        assert!(!state.camera_intent);
        press(&mut state, &ctx, GrantSnapshot { camera: Grant::Denied, ..granted() });
        assert!(state.start.is_some());
        assert!(state.start.take().is_some());
    }
    #[test]
    fn record_with_a_never_asked_input_does_not_prompt() {
        let ctx = ctx();
        let mut state = test_state();
        state.microphone_intent = true;
        press(&mut state, &ctx, GrantSnapshot { microphone: Grant::NotAsked, ..granted() });
        assert!(state.start.is_none() && requests().is_empty());
        assert_eq!(state.toast.as_ref().unwrap().text, "Mic access isn't allowed yet · Click Mic");
    }
    #[test]
    fn a_missing_remembered_camera_stops_record_and_no_same_named_device_is_substituted() {
        let ctx = ctx();
        let mut state = test_state();
        state.camera_intent = true;
        state.camera_device_id = Some("UID-GONE".into());
        state.camera_names = vec![super::super::devices::CameraDevice { id: "UID-GONE".into(), name: "Studio Camera".into() }];
        // The fixture lists a *different* device that is also called "Studio Camera".
        press(&mut state, &ctx, granted());
        assert!(state.start.is_some());
        settle(&mut state, &ctx);
        assert!(state.camera.is_none() && state.camera_permission.is_none(), "nothing may open");
        assert!(state.countdown.is_none() && state.worker.is_none() && !state.recording);
        assert!(!requests().contains(&"camera"));
        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.text, "Studio Camera isn't connected");
        assert_eq!((toast.action, toast.secondary), (Some(ToastAction::ChooseCamera), Some(ToastAction::TurnOffCamera)));
        assert_eq!(state.camera_device_id.as_deref(), Some("UID-GONE"), "the preference is never rewritten");
        assert_eq!(state.mark(true), Mark::Attention);
    }
    #[test]
    fn present_devices_open_then_the_countdown_runs_with_no_prompt_and_no_take() {
        let ctx = ctx();
        let mut state = test_state();
        state.camera_intent = true;
        state.microphone_intent = true;
        state.camera_device_id = Some("UID-A".into());
        state.microphone_device_id = Some("Built-in".into());
        press(&mut state, &ctx, granted());
        settle(&mut state, &ctx);
        assert!(state.camera.is_some() && state.microphone.is_some());
        assert!(state.countdown.is_some() && !state.recording && state.worker.is_none());
        assert_eq!(state.face(Instant::now()), Face::Countdown(3));
        assert!(state.locked());
        // Cancelling writes nothing and leaves the inputs as they were.
        press(&mut state, &ctx, granted());
        assert!(state.countdown.is_none() && state.worker.is_none());
    }
    #[test]
    fn an_input_that_fails_to_open_rolls_back_what_this_press_opened() {
        let ctx = ctx();
        let mut state = test_state();
        state.os.open_camera = |_| Err("fixture: the camera refused".into());
        state.camera_intent = true;
        state.microphone_intent = true;
        press(&mut state, &ctx, granted());
        settle(&mut state, &ctx);
        assert!(state.camera.is_none() && state.microphone.is_none(), "the mic opened for this press is closed again");
        assert!(state.countdown.is_none() && state.worker.is_none());
        let toast = state.toast.as_ref().unwrap();
        assert_eq!(toast.text, "Couldn't start the camera");
        assert_eq!(toast.details.as_deref(), Some("fixture: the camera refused"));
    }
    #[test]
    fn an_input_that_was_already_live_is_not_closed_by_a_failed_press() {
        let ctx = ctx();
        let mut state = test_state();
        state.os.open_microphone = |_| Err("fixture: mic refused".into());
        state.camera = Some(fake_camera());
        state.camera_intent = true;
        state.microphone_intent = true;
        press(&mut state, &ctx, granted());
        settle(&mut state, &ctx);
        assert!(state.camera.is_some(), "a camera the person already turned on stays on");
        assert!(state.microphone.is_none() && state.countdown.is_none());
    }
    #[test]
    fn screen_recording_is_requested_at_most_once_per_process_from_a_press() {
        let ctx = ctx();
        let mut state = test_state();
        let denied = GrantSnapshot { screen: false, ..granted() };
        press(&mut state, &ctx, denied);
        assert_eq!(requests(), vec!["screen"]);
        assert!(state.screen_permission.is_some() && state.start.is_none());
        // The OS refuses; later presses explain and never ask again.
        state.screen_permission = Some(Arc::new(AtomicI8::new(-1)));
        state.complete_permissions(&ctx);
        press(&mut state, &ctx, denied);
        press(&mut state, &ctx, denied);
        assert_eq!(requests(), vec!["screen"]);
        let toast = state.toast.as_ref().unwrap();
        assert_eq!((toast.text.as_str(), toast.action), ("Screen Recording is off", Some(ToastAction::OpenScreenSettings)));
    }
    #[test]
    fn the_record_press_uses_the_status_it_just_read_not_a_cached_one() {
        let ctx = ctx();
        let mut state = test_state();
        state.screen_ready = false; // stale: the person allowed it in System Settings meanwhile
        press(&mut state, &ctx, granted());
        assert!(state.screen_ready && requests().is_empty());
        assert!(state.start.is_some());
    }
    #[test]
    fn hide_releases_live_inputs_keeps_intent_and_show_only_arms() {
        let ctx = ctx();
        let mut state = test_state();
        state.camera = Some(fake_camera());
        state.microphone = Some(MicSource::Fake);
        state.camera_intent = true;
        state.microphone_intent = true;
        state.camera_permission = Some(Arc::new(AtomicI8::new(1)));
        state.hide(&ctx);
        assert!(state.camera.is_none() && state.microphone.is_none() && state.camera_permission.is_none());
        assert!(state.camera_intent && state.microphone_intent);
        state.show_toolbar(&ctx);
        assert!(!state.hidden && state.camera.is_none() && state.microphone.is_none());
        assert_eq!((state.mark(true), state.mark(false)), (Mark::Armed, Mark::Armed));
        assert!(requests().is_empty());
    }
    #[test]
    fn inputs_and_source_lock_during_a_take_but_the_bubble_does_not() {
        let mut state = test_state();
        state.recording = true;
        state.camera = Some(fake_camera());
        state.camera_intent = true;
        state.click_input(true, Grant::Allowed);
        assert!(state.camera.is_some(), "the click is ignored while locked");
        state.turn_off(true);
        assert!(state.camera.is_some());
        state.open_panel(PanelKind::Camera);
        assert!(state.panel.is_none());
        state.apply_device_choice(true, DeviceChoice::Device("UID-A".into()));
        assert!(state.camera.is_some() && state.camera_device_id.is_none());
        // Bubble shape and size stay live locally and are saved after the take.
        let mut draft = state.prefs.clone();
        draft.shape = SHAPE_WIDE;
        draft.size = 224;
        state.commit_settings(&draft);
        assert_eq!((state.prefs.shape, state.prefs.size), (SHAPE_WIDE, 224));
        assert!(state.bubble_dirty && state.confirmed.shape != SHAPE_WIDE);
        assert_eq!(state.layout.lock().unwrap().shape, SHAPE_WIDE);
    }
    #[test]
    fn choosing_a_device_arms_without_opening_and_off_clears_intent() {
        let mut state = test_state();
        state.camera = Some(fake_camera());
        state.apply_device_choice(true, DeviceChoice::Device("UID-A".into()));
        assert!(state.camera.is_none(), "a different device is never swapped in silently");
        assert_eq!((state.camera_device_id.as_deref(), state.camera_intent), (Some("UID-A"), true));
        assert_eq!(state.mark(true), Mark::Armed);
        state.apply_device_choice(true, DeviceChoice::SystemDefault);
        assert_eq!((state.camera_device_id.clone(), state.camera_intent), (None, true));
        state.apply_device_choice(true, DeviceChoice::Off);
        assert!(!state.camera_intent);
        assert!(requests().is_empty());
    }
    #[test]
    fn panels_open_only_when_idle_and_a_second_press_or_outside_click_closes() {
        let ctx = ctx();
        let mut state = test_state();
        state.toggle_panel(PanelKind::Source);
        assert_eq!(state.panel.as_ref().map(|p| p.kind), Some(PanelKind::Source));
        state.toggle_panel(PanelKind::Source);
        assert!(state.panel.is_none());
        state.open_panel(PanelKind::Microphone);
        state.close_panel(&ctx, PanelKind::Microphone, Instant::now());
        // The press that dismissed it by clicking outside must not reopen it.
        state.toggle_panel(PanelKind::Microphone);
        assert!(state.panel.is_none());
        state.hidden = true;
        state.open_panel(PanelKind::Camera);
        assert!(state.panel.is_none());
    }
    #[test]
    fn opening_a_picker_lists_devices_only_when_access_already_exists() {
        let mut state = test_state();
        state.grants = Some(GrantSnapshot { camera: Grant::NotAsked, ..granted() });
        state.open_panel(PanelKind::Camera);
        assert!(state.camera_discovery.is_none(), "no listing before access, and never a request");
        state.panel = None;
        state.grants = Some(granted());
        state.open_panel(PanelKind::Camera);
        assert!(state.camera_discovery.is_some());
        for _ in 0..200 {
            state.poll_discovery();
            if state.camera_choices.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.camera_choices.as_ref().map(Vec::len), Some(1));
        assert!(requests().is_empty());
    }
    #[test]
    fn remote_show_is_passive() {
        let ctx = ctx();
        let mut state = test_state();
        state.camera_intent = true;
        state.hidden = true;
        state.show_toolbar(&ctx);
        assert!(!state.hidden && state.camera.is_none() && state.camera_permission.is_none() && state.start.is_none());
        assert!(requests().is_empty());
    }
}

#[cfg(test)]
mod completion_tests {
    use super::fixtures::*;
    use super::*;
    use super::super::connection::{Completed, PendingCompletion};
    use std::sync::atomic::AtomicUsize;

    fn metadata_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openrecorder-ui-completion-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("recording.mp4"), b"media").unwrap();
        std::fs::write(dir.join("capture.json"), r#"{"duration":9.0}"#).unwrap();
        dir
    }
    /// A library that refuses completions until `ok` is flipped, like a full disk.
    fn host(ok: Arc<AtomicBool>, calls: Arc<AtomicUsize>) -> Arc<dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync> {
        Arc::new(move |op, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            match op {
                "native-finish" | "native-abort" if ok.load(Ordering::SeqCst) => Ok(serde_json::json!({"status":"ready"})),
                "native-finish" | "native-abort" => Err("No space left on device".into()),
                _ => Err("unexpected".into()),
            }
        })
    }
    fn finished_worker_with_unsaved_take(state: &mut State, name: &str) -> PathBuf {
        let dir = metadata_dir(name);
        let mut parked = PendingCompletion::finish(serde_json::json!("take-9"), dir.clone());
        parked.last_error = "No space left on device".into();
        *state.pending_save.lock().unwrap() = Some(parked);
        state.options.progress.fps.store(30, Ordering::Relaxed);
        state.options.progress.frames.store(30 * 9, Ordering::Relaxed);
        state.recording = false;
        state.finish_take(Ok(Ok(())));
        dir
    }

    #[test]
    fn a_capture_the_library_did_not_confirm_is_never_shown_as_saved() {
        let mut state = test_state();
        let dir = finished_worker_with_unsaved_take(&mut state, "never-saved");
        let toast = state.toast.as_ref().unwrap();
        assert!(toast.error && !toast.text.starts_with("Saved"), "{}", toast.text);
        assert_eq!(toast.action, Some(ToastAction::RetrySave));
        assert_eq!(toast.details.as_deref(), Some("No space left on device"));
        assert!(state.saved_until.is_none(), "no Saved face");
        assert_eq!(state.face(Instant::now()), Face::Saving);
        assert!(state.locked(), "inputs and a new take stay locked while the library still lists the take as active");
        assert!(dir.join("recording.mp4").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn retries_are_bounded_then_manual_and_a_confirmed_save_unlocks() {
        let ctx = egui::Context::default();
        let mut state = test_state();
        let (ok, calls) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
        state.host_override = Some(host(ok.clone(), calls.clone()));
        let dir = finished_worker_with_unsaved_take(&mut state, "bounded");
        assert!(state.completion_next.is_some());
        // Three automatic tries (2 s, 5 s, 15 s), each after its deadline, then nothing more.
        let mut delays = Vec::new();
        for _ in 0..3 {
            let due = state.completion_next.expect("a retry is scheduled");
            delays.push(due.saturating_duration_since(Instant::now()).as_secs());
            state.completion_next = Some(Instant::now());
            state.tick_completion(&ctx);
        }
        // Whole seconds remaining when read: just under 2, 5 and 15.
        assert_eq!(delays, vec![1, 4, 14]);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(state.completion_next.is_none(), "automatic retries stop after three");
        state.tick_completion(&ctx);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "no polling storm");
        assert_eq!(state.face(Instant::now()), Face::RetrySave);
        assert_eq!(state.toast.as_ref().map(|t| t.action), Some(Some(ToastAction::RetrySave)));
        // Pressing Record retries (not a new take), still failing: still pending, still no Saved.
        state.record_press(&ctx, granted());
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(state.completion.is_some() && state.worker.is_none() && state.countdown.is_none() && state.start.is_none());
        // The library recovers: the same press stores it and only then says Saved.
        ok.store(true, Ordering::SeqCst);
        state.record_press(&ctx, granted());
        assert!(state.completion.is_none() && !state.locked());
        assert!(state.toast.as_ref().unwrap().text.starts_with("Saved · 0:09"));
        assert_eq!(state.face(Instant::now()), Face::Saved);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn quit_is_refused_while_a_take_is_unconfirmed_and_allowed_once_it_is() {
        let ctx = egui::Context::default();
        let mut state = test_state();
        let (ok, calls) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
        state.host_override = Some(host(ok.clone(), calls));
        let dir = finished_worker_with_unsaved_take(&mut state, "quit");
        state.request_quit(&ctx);
        assert!(!state.quitting && state.completion.is_some());
        assert!(state.toast.as_ref().unwrap().text.starts_with("Couldn't quit"));
        ok.store(true, Ordering::SeqCst);
        state.request_quit(&ctx);
        assert!(state.quitting && state.completion.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_worker_that_died_leaves_a_media_preserving_abort_not_a_stuck_take() {
        let mut state = test_state();
        let dir = metadata_dir("panic");
        let placeholder = PendingCompletion::abort(serde_json::json!("take-7"), dir.clone(), "The recorder stopped unexpectedly. Available files have been preserved.".into());
        *state.pending_save.lock().unwrap() = Some(placeholder);
        state.finish_take(Err(Box::new("worker panicked")));
        assert!(state.completion.is_some(), "the take id survives the panic");
        let (ok, calls) = (Arc::new(AtomicBool::new(true)), Arc::new(AtomicUsize::new(0)));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let inner = host(ok, calls);
        state.host_override = Some(Arc::new(move |op, data| {
            recorded.lock().unwrap().push((op.to_owned(), data.clone()));
            inner(op, data)
        }));
        state.retry_completion(true);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "native-abort");
        assert_eq!(seen[0].1["id"], "take-7");
        assert!(state.completion.is_none());
        assert!(dir.join("recording.mp4").exists(), "media is preserved");
        let _ = Completed::Partial;
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn an_unconfirmed_begin_is_starting_not_saving_never_saved_and_blocks_a_new_take_until_settled() {
        let ctx = egui::Context::default();
        let mut state = test_state();
        *state.pending_save.lock().unwrap() = Some(PendingCompletion::begin("0123456789abcdef0123456789abcdef"));
        // The worker returned an error: capture never started (zero frames).
        state.finish_take(Ok(Err("The Library didn't confirm the recording start. Nothing was recorded.".into())));
        let toast = state.toast.as_ref().unwrap();
        assert!(toast.error && toast.text == "Couldn't start recording" && toast.action == Some(ToastAction::RetrySave), "{}", toast.text);
        assert!(state.saved_until.is_none());
        assert_eq!(state.face(Instant::now()), Face::Starting, "not Saving: nothing was captured");
        assert!(state.locked() && state.worker.is_none() && !state.recording);
        state.record_press(&ctx, granted());
        assert!(state.countdown.is_none() && state.start.is_none() && state.worker.is_none(), "no second take is started over an unsettled begin");
        // Quit waits for the host to settle it.
        let (cancelled, calls) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
        let (flag, counter) = (cancelled.clone(), calls.clone());
        state.host_override = Some(Arc::new(move |op, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            assert_eq!(op, "native-begin-resolve", "only a resolve is ever sent, never a new begin");
            if flag.load(Ordering::SeqCst) { Ok(serde_json::json!({"state":"cancelled"})) } else { Err("The library is unavailable. Your local recordings are safe.".into()) }
        }));
        state.request_quit(&ctx);
        assert!(!state.quitting && state.completion.is_some());
        assert!(state.toast.as_ref().unwrap().text.starts_with("Couldn't quit"));
        // Retries are bounded (2, 5, 15 s) and then manual, with a begin-specific message.
        for _ in 0..3 {
            state.completion_next = Some(Instant::now());
            state.tick_completion(&ctx);
        }
        assert!(state.completion_next.is_none());
        assert_eq!(state.face(Instant::now()), Face::RetrySave);
        assert!(state.toast.as_ref().unwrap().text.starts_with("Couldn't confirm the recording started"));
        // The host settles it: nothing was recorded, nothing is "Saved", everything is free again.
        cancelled.store(true, Ordering::SeqCst);
        state.record_press(&ctx, granted());
        assert!(state.completion.is_none() && !state.locked());
        assert_eq!(state.toast.as_ref().unwrap().text, "Nothing was recorded");
        assert!(state.saved_until.is_none() && state.face(Instant::now()) == Face::Ready);
        assert!(calls.load(Ordering::SeqCst) >= 5);
        state.request_quit(&ctx);
        assert!(state.quitting);
    }
    #[test]
    fn a_pending_begin_that_proves_active_is_closed_without_claiming_a_save() {
        let ctx = egui::Context::default();
        let mut state = test_state();
        let dir = metadata_dir("begin-active");
        *state.pending_save.lock().unwrap() = Some(PendingCompletion::begin("fedcba9876543210fedcba9876543210"));
        state.finish_take(Ok(Err("unconfirmed".into())));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (log, directory) = (seen.clone(), dir.clone());
        state.host_override = Some(Arc::new(move |op, data| {
            log.lock().unwrap().push(op.to_owned());
            match op {
                "native-begin-resolve" => Ok(serde_json::json!({"state":"active","record":{"id":"take-5","directory":directory}})),
                "native-abort" => {
                    assert_eq!(data["id"], "take-5");
                    Ok(serde_json::json!({"status":"interrupted"}))
                }
                other => panic!("unexpected {other}"),
            }
        }));
        state.retry_completion(true);
        assert_eq!(*seen.lock().unwrap(), vec!["native-begin-resolve", "native-abort"]);
        assert_eq!(state.toast.as_ref().unwrap().text, "Nothing was recorded");
        assert!(state.completion.is_none() && state.saved_until.is_none());
        assert!(dir.join("recording.mp4").exists());
        std::fs::remove_dir_all(dir).unwrap();
        let _ = ctx;
    }
    #[test]
    fn a_confirmed_save_and_a_failed_capture_without_pending_keep_their_old_behaviour() {
        let mut state = test_state();
        state.options.progress.fps.store(30, Ordering::Relaxed);
        state.options.progress.frames.store(30 * 4, Ordering::Relaxed);
        state.finish_take(Ok(Ok(())));
        assert_eq!(state.toast.as_ref().unwrap().text, "Saved · 0:04");
        assert!(state.saved_until.is_some() && state.completion.is_none());
        let mut failed = test_state();
        failed.finish_take(Ok(Err("raw failure".into())));
        assert_eq!(failed.toast.as_ref().unwrap().text, "Couldn't start recording");
        assert!(failed.completion.is_none() && failed.saved_until.is_none());
    }
}

#[cfg(test)]
mod preview_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn unchanged_settings_do_not_contact_host() {
        let confirmed = State::default().settings_values();
        let result = settings_transaction(&confirmed, &confirmed, |_| panic!("No-op contacted host"));
        assert_eq!(result.unwrap(), None);
    }

    /// A host that stores exactly what it was sent and echoes it back.
    fn echo(payload: &serde_json::Value, base: &SettingsValues) -> serde_json::Value {
        let mut settings = base.preferences();
        for (k, v) in payload["preferences"].as_object().into_iter().flatten() {
            settings[k] = v.clone();
        }
        serde_json::json!({"settings":settings,"root":payload["root"].as_str().unwrap_or("")})
    }

    #[test]
    fn settings_save_sends_only_changed_preferences() {
        let confirmed = State::default().settings_values();
        let mut draft = confirmed.clone();
        draft.theme = 2;
        draft.mirror = !confirmed.mirror;
        let mut calls = 0;
        let (saved, _) = settings_transaction(&confirmed, &draft, |payload| {
            calls += 1;
            assert_eq!(payload, serde_json::json!({"preferences":{"theme":"dark","mirror":draft.mirror}}));
            assert!(payload.get("root").is_none());
            assert!(payload["preferences"].get("cameraDeviceId").is_none());
            Ok(echo(&payload, &confirmed))
        })
        .unwrap()
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(saved, draft);
        assert_eq!(saved.size, confirmed.size);
    }

    #[test]
    fn new_preferences_use_the_names_the_host_will_read() {
        let confirmed = State::default().settings_values();
        let mut draft = confirmed.clone();
        draft.remember = false;
        draft.countdown = 0;
        draft.size = 224;
        draft.shape = SHAPE_WIDE;
        let _ = settings_transaction(&confirmed, &draft, |payload| {
            assert_eq!(
                payload["preferences"],
                serde_json::json!({"rememberInputs":false,"countdownSeconds":0,"cameraSize":224,"cameraShape":"wide"})
            );
            Ok(echo(&payload, &confirmed))
        })
        .unwrap();
    }

    #[test]
    fn a_host_that_drops_an_unknown_key_is_a_failure_not_a_silent_success() {
        let confirmed = State::default().settings_values();
        let mut draft = confirmed.clone();
        draft.countdown = 0;
        // An older library service accepts the request and ignores the key.
        let result = settings_transaction(&confirmed, &draft, |_| Ok(serde_json::json!({"settings":confirmed.preferences()})));
        assert_eq!(result.unwrap_err(), "The Library service didn't confirm “Countdown”. Nothing was changed.");
        // A reply without settings at all can't confirm anything either.
        assert!(settings_transaction(&confirmed, &draft, |_| Ok(serde_json::json!({}))).is_err());
    }

    #[test]
    fn rejected_settings_never_become_confirmed_and_path_only_save_has_no_preferences() {
        let confirmed = State::default().settings_values();
        let mut draft = confirmed.clone();
        draft.root = PathBuf::from("/new/recordings");
        let mut current = confirmed.clone();
        let result = settings_transaction(&confirmed, &draft, |payload| {
            assert_eq!(payload, serde_json::json!({"root":"/new/recordings"}));
            Err("Drive is unavailable".into())
        });
        if let Ok(Some((accepted, _))) = &result {
            current = accepted.clone();
        }
        assert_eq!(result.unwrap_err(), "Drive is unavailable");
        assert_eq!(current, confirmed);
        // The same proposal can be explicitly retried, then and only then applied.
        let (accepted, _) = settings_transaction(&confirmed, &draft, |p| Ok(echo(&p, &confirmed))).unwrap().unwrap();
        assert_eq!(accepted.root, draft.root);
        // An unconfirmed folder rolls back too.
        assert!(settings_transaction(&confirmed, &draft, |_| Ok(serde_json::json!({"root":"/elsewhere","settings":confirmed.preferences()}))).is_err());
    }

    #[test]
    fn a_rejected_save_rolls_the_live_value_back_and_says_so() {
        let mut state = test_state();
        state.prefs.countdown = 3;
        state.confirmed = state.prefs.clone();
        let mut draft = state.prefs.clone();
        draft.countdown = 0;
        // No connection: the write cannot be confirmed.
        state.commit_settings(&draft);
        assert_eq!(state.prefs, state.confirmed);
        assert_eq!(state.prefs.countdown, 3);
        assert!(state.settings_error && state.settings_status.starts_with("Couldn't save."));
    }

    #[test]
    fn host_active_or_unknown_state_prevents_quit_action() {
        for state in [serde_json::json!({"active":{"id":"pending-take"}}), serde_json::json!({})] {
            let mut operations = Vec::new();
            let result = confirm_host_quit(|op| {
                operations.push(op.to_owned());
                Ok(state.clone())
            });
            assert!(result.is_err());
            assert_eq!(operations, vec!["state"]);
        }
    }

    #[test]
    fn quit_requires_host_acceptance_after_saved_state() {
        let result = confirm_host_quit(|op| match op {
            "state" => Ok(serde_json::json!({"active":null})),
            "quit" => Err("Stop and save first".into()),
            _ => panic!("Unexpected operation"),
        });
        assert_eq!(result.unwrap_err(), "Stop and save first");
        assert!(confirm_host_quit(|op| Ok(if op == "state" { serde_json::json!({"active":null}) } else { serde_json::json!({"quitting":false}) })).is_err());
        let mut operations = Vec::new();
        assert!(confirm_host_quit(|op| {
            operations.push(op.to_owned());
            Ok(if op == "state" { serde_json::json!({"active":null}) } else { serde_json::json!({"quitting":true}) })
        })
        .is_ok());
        assert_eq!(operations, vec!["state", "quit"]);
    }

    #[test]
    fn preferences_restore_ids_and_intent_without_opening_inputs() {
        let mut state = State::default();
        state.preferences(&serde_json::json!({"camera":true,"microphone":true,"cameraDeviceId":"UID-A","microphoneDeviceId":"UID-M"}));
        assert_eq!(state.camera_device_id.as_deref(), Some("UID-A"));
        assert_eq!(state.microphone_device_id.as_deref(), Some("UID-M"));
        assert!(state.camera_intent && state.microphone_intent);
        assert!(state.camera.is_none() && state.microphone.is_none());
        assert!(state.camera_permission.is_none() && state.microphone_permission.is_none());
    }

    #[test]
    fn existing_preferences_survive_until_the_person_changes_them() {
        let mut state = State::default();
        state.preferences(&serde_json::json!({"cameraSize":200,"cameraShape":"square","quality":2560,"fps":60,"keepOriginals":false,"theme":"dark","mirror":false,"countdown":0}));
        assert_eq!((state.prefs.size, state.prefs.shape), (200, SHAPE_SQUARE), "legacy size and square are kept, not normalised");
        assert_eq!((state.prefs.quality, state.prefs.fps, state.prefs.originals, state.prefs.theme), (2560, 60, false, 2));
        assert_eq!(state.prefs.countdown, 0, "the older countdown key is still honoured");
        assert_eq!(state.prefs, state.confirmed);
        // Nothing here is a change: re-saving the same values writes nothing.
        let confirmed = state.confirmed.clone();
        assert_eq!(settings_transaction(&confirmed, &state.prefs, |_| panic!("wrote")).unwrap(), None);
        let mut wide = State::default();
        wide.preferences(&serde_json::json!({"cameraSize":224,"cameraShape":"wide","rememberInputs":false,"countdownSeconds":3}));
        assert_eq!((wide.prefs.size, wide.prefs.shape, wide.prefs.remember, wide.prefs.countdown), (224, SHAPE_WIDE, false, 3));
        // Defaults when nothing is stored.
        let mut fresh = State::default();
        fresh.preferences(&serde_json::json!({}));
        assert_eq!((fresh.prefs.size, fresh.prefs.shape, fresh.prefs.remember, fresh.prefs.countdown), (168, SHAPE_CIRCLE, true, 3));
    }

    #[test]
    fn layout_is_never_mirrored_even_when_the_preview_is() {
        let mut state = State::default();
        state.preferences(&serde_json::json!({"mirror":true,"cameraShape":"wide"}));
        assert!(state.preview_mirror.load(Ordering::Relaxed));
        let layout = state.layout.lock().unwrap();
        assert!(!layout.mirror, "the composited camera is unmirrored");
        assert_eq!(layout.shape, SHAPE_WIDE);
    }

    #[test]
    fn a_preferences_refresh_keeps_the_arming_a_person_just_made() {
        for remember in [false, true] {
            let ctx = egui::Context::default();
            let mut state = test_state();
            let stored = |camera: bool| serde_json::json!({"rememberInputs":remember,"camera":camera,"microphone":false,"cameraDeviceId":null});
            state.preferences(&stored(false));
            assert!(!state.camera_intent && !state.microphone_intent);
            // A device pick arms without opening a stream, then the host posts `preferences`.
            state.apply_device_choice(true, DeviceChoice::Device("UID-A".into()));
            assert!(state.camera_intent && state.camera.is_none());
            state.preferences(&stored(false));
            assert!(state.camera_intent, "remember={remember}: the refresh dropped the pick");
            assert_eq!(state.camera_device_id.as_deref(), Some("UID-A"), "a refresh never swaps the chosen device");
            // Hide, then an unrelated setting changes: still armed, still not live.
            state.hide(&ctx);
            state.preferences(&stored(false));
            state.preferences(&serde_json::json!({"rememberInputs":remember,"camera":false,"countdownSeconds":0}));
            assert!(state.camera_intent && state.camera.is_none() && state.camera_permission.is_none());
            assert_eq!(state.mark(true), Mark::Armed);
            // The stored value never *arms* mid-session either: an agent's settings write can't.
            state.microphone_intent = false;
            state.preferences(&serde_json::json!({"rememberInputs":remember,"camera":true,"microphone":true}));
            assert!(!state.microphone_intent && state.microphone.is_none());
            assert!(requests().is_empty());
        }
    }

    #[test]
    fn the_startup_reset_happens_once_and_only_when_remember_is_off() {
        let mut off = State::default();
        off.preferences(&serde_json::json!({"rememberInputs":false,"camera":true,"microphone":true}));
        assert!(!off.camera_intent && !off.microphone_intent && off.intent_loaded);
        let mut on = State::default();
        on.preferences(&serde_json::json!({"rememberInputs":true,"camera":true,"microphone":false}));
        assert!(on.camera_intent && !on.microphone_intent);
        // Turning remember off later is a preference change, not a reset of the live session.
        on.preferences(&serde_json::json!({"rememberInputs":false,"camera":true}));
        assert!(on.camera_intent && !on.prefs.remember);
    }

    #[test]
    fn remember_off_does_not_arm_and_does_not_write_intent() {
        let mut state = State::default();
        state.preferences(&serde_json::json!({"rememberInputs":false,"camera":true,"microphone":true,"cameraDeviceId":"UID-A"}));
        assert!(!state.camera_intent && !state.microphone_intent);
        assert_eq!(state.camera_device_id.as_deref(), Some("UID-A"), "the device preference is untouched");
    }

    #[test]
    fn record_slot_cancels_countdown_without_worker_or_files() {
        let ctx = egui::Context::default();
        let mut state = State { countdown: Some(Instant::now()), ..test_state() };
        state.record_press(&ctx, granted());
        assert!(state.countdown.is_none() && !state.countdown_closing);
        assert!(!state.recording && state.worker.is_none());
        // Also cancel the native-HUD removal phase before a worker can be created.
        state.countdown_closing = true;
        state.record_press(&ctx, granted());
        assert!(!state.countdown_closing && state.worker.is_none());
    }

    #[test]
    fn hide_discards_pending_input_opens_but_retains_preferences() {
        let ctx = egui::Context::default();
        let mut state = State {
            camera_permission: Some(Arc::new(AtomicI8::new(1))),
            microphone_permission: Some(Arc::new(AtomicI8::new(1))),
            camera_device_id: Some("remembered".into()),
            camera_intent: true,
            countdown: Some(Instant::now()),
            ..test_state()
        };
        state.hide(&ctx);
        assert!(state.hidden && state.countdown.is_none());
        assert!(state.camera_permission.is_none() && state.microphone_permission.is_none());
        assert_eq!(state.camera_device_id.as_deref(), Some("remembered"));
        assert!(state.camera_intent);
        assert!(state.worker.is_none());
    }

    #[test]
    fn hide_leaves_no_picker_or_pill_stranded() {
        let ctx = egui::Context::default();
        let mut state = test_state();
        state.panel = Some(Panel { kind: PanelKind::Source, tab: SourceTab::Display, search: String::new(), was_focused: true, size: egui::Vec2::ZERO });
        state.toast = Some(Toast::info("x"));
        state.deferred_toast = Some(Toast::info("y"));
        state.settings_open = true;
        state.setup_open = true;
        state.quit_confirmation = true;
        state.hide(&ctx);
        assert!(state.panel.is_none() && state.toast.is_none() && state.deferred_toast.is_none());
        assert!(!state.settings_open && !state.setup_open && !state.quit_confirmation);
    }

    #[test]
    fn countdown_waits_for_hud_removal_and_escape_cancels_without_capture() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let hud = vp(VP_COUNTDOWN);
        let mut raw = egui::RawInput::default();
        raw.viewports.insert(hud, egui::ViewportInfo::default());
        ctx.begin_pass(raw);
        ctx.show_viewport_deferred(hud, egui::ViewportBuilder::default().with_title("Countdown fixture"), |_, _| {});
        let mut state = State { countdown_closing: true, ..test_state() };
        state.countdown_ui(&ctx);
        assert!(state.countdown_closing && state.worker.is_none());
        let output = ctx.end_pass();
        assert!(output.viewport_output[&hud].commands.iter().any(|cmd| matches!(cmd, egui::ViewportCommand::Close)));
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        ctx.begin_pass(raw);
        state.countdown_ui(&ctx);
        assert!(!state.countdown_closing && state.countdown.is_none());
        assert!(state.worker.is_none() && !state.recording);
        let output = ctx.end_pass();
        assert_eq!(output.viewport_output.len(), 1);
        assert!(output.viewport_output.contains_key(&egui::ViewportId::ROOT));
    }

    #[test]
    fn countdown_keeps_last_fraction_of_each_second() {
        for (millis, expected) in [(3000, 3), (2001, 3), (2000, 2), (1001, 2), (1000, 1), (1, 1)] {
            assert_eq!(countdown_number(Duration::from_millis(millis)), expected);
        }
    }

    #[test]
    fn crop_preserves_scale_and_mirror() {
        let mut frame = vec![0; 1280 * 720 * 4];
        for y in 0..720 {
            for x in 0..1280 {
                let i = (y * 1280 + x) * 4;
                frame[i] = (x % 251) as u8;
                frame[i + 1] = (y % 251) as u8;
            }
        }
        let frame = super::super::camera_frame::Frame { width: 1280, height: 720, bgra: frame, sequence: 0 };
        let normal = preview_rgba(&frame, false);
        let mirrored = preview_rgba(&frame, true);
        assert_eq!(&normal[0..4], &[0, 0, 29, 255]);
        assert_eq!(&normal[(359 * 360) * 4..(359 * 360) * 4 + 4], &[0, 216, 29, 255]);
        assert_eq!(&mirrored[0..4], &normal[359 * 4..360 * 4]);
    }

    /// A frame whose pixels encode their own coordinates: blue = column / `cx`, green = row / `cy`,
    /// red constant. Output blue/green therefore reveal exactly which source pixel was sampled.
    fn coordinate_frame(w: usize, h: usize, cx: usize, cy: usize) -> super::super::camera_frame::Frame {
        let mut bgra = vec![0; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                bgra[i] = (x / cx) as u8;
                bgra[i + 1] = (y / cy) as u8;
                bgra[i + 2] = 77;
            }
        }
        super::super::camera_frame::Frame { width: w, height: h, bgra, sequence: 0 }
    }
    /// Expected source (column, row) for an output pixel, derived independently of `cover_rect`:
    /// uniform scale = max(dw / w, dh / h), centred, pixel centres mapped through that scale.
    fn expected_source(w: usize, h: usize, dw: usize, dh: usize, x: usize, y: usize, mirror: bool) -> (f64, f64) {
        let scale = (dw as f64 / w as f64).max(dh as f64 / h as f64);
        let (visible_w, visible_h) = (dw as f64 / scale, dh as f64 / scale);
        let (ox, oy) = ((w as f64 - visible_w) / 2., (h as f64 - visible_h) / 2.);
        let mx = if mirror { dw - 1 - x } else { x };
        (ox + (mx as f64 + 0.5) / scale, oy + (y as f64 + 0.5) / scale)
    }
    fn sampled(pixels: &[u8], dw: usize, x: usize, y: usize, cx: usize, cy: usize) -> (f64, f64) {
        let o = (y * dw + x) * 4;
        // Pixel value v covers source columns [v * cx, (v + 1) * cx): use its centre.
        ((pixels[o + 2] as f64 + 0.5) * cx as f64, (pixels[o + 1] as f64 + 0.5) * cy as f64)
    }

    #[test]
    fn wide_preview_crops_and_mirrors_the_source_pixels_an_independent_derivation_predicts() {
        let (dw, dh) = preview_dimensions(SHAPE_WIDE);
        assert_eq!((dw, dh), (480, 270));
        // (source size, value divisors): a 16:9 frame, a portrait frame (crops top and bottom), a near-square
        // frame and an ultra-wide frame (crops left and right). Asymmetric gradients in both axes.
        for (w, h, cx, cy) in [(1280usize, 720usize, 5usize, 3usize), (405, 720, 2, 3), (960, 800, 4, 4), (1000, 400, 4, 2)] {
            let frame = coordinate_frame(w, h, cx, cy);
            for mirror in [false, true] {
                let pixels = preview_cover(&frame, mirror, dw, dh);
                assert!(pixels.chunks(4).all(|p| p[3] == 255 && p[0] == 77), "{w}x{h}: every pixel is camera data, no padding bars");
                for (x, y) in [(0, 0), (dw - 1, 0), (0, dh - 1), (dw - 1, dh - 1), (dw / 2, dh / 2), (37, 201), (dw - 38, 64)] {
                    let (ex, ey) = expected_source(w, h, dw, dh, x, y, mirror);
                    let (sx, sy) = sampled(&pixels, dw, x, y, cx, cy);
                    // One source pixel plus the value quantisation of this fixture.
                    assert!((sx - ex).abs() <= cx as f64 + 1.5 && (sy - ey).abs() <= cy as f64 + 1.5,
                        "{w}x{h} mirror={mirror} ({x},{y}): sampled ({sx:.1},{sy:.1}) expected ({ex:.1},{ey:.1})");
                }
            }
            // Mirroring is exactly a left-right flip of the same crop.
            let plain = preview_cover(&frame, false, dw, dh);
            let flipped = preview_cover(&frame, true, dw, dh);
            for y in (0..dh).step_by(17) {
                for x in (0..dw).step_by(13) {
                    let (a, b) = ((y * dw + x) * 4, (y * dw + (dw - 1 - x)) * 4);
                    assert_eq!(&flipped[a..a + 4], &plain[b..b + 4], "{w}x{h} ({x},{y})");
                }
            }
            // The fixture really is asymmetric: left and right, top and bottom differ.
            assert_ne!(plain[2], plain[(dw - 1) * 4 + 2], "left and right edges must differ");
            assert_ne!(plain[1], plain[((dh - 1) * dw) * 4 + 1], "top and bottom edges must differ");
        }
        // A portrait source is cropped, never padded: the visible band is 405 x 227.8 centred in 720 rows.
        let portrait = coordinate_frame(405, 720, 2, 3);
        let pixels = preview_cover(&portrait, false, dw, dh);
        let top_row = sampled(&pixels, dw, 0, 0, 2, 3).1;
        assert!((top_row - (720. - 405. * 270. / 480.) / 2.).abs() < 4., "top row samples source row {top_row}");
        // Mismatched buffers produce an empty preview instead of reading out of bounds.
        let broken = super::super::camera_frame::Frame { width: 10, height: 10, bgra: vec![0; 8], sequence: 0 };
        assert!(preview_cover(&broken, false, 4, 4).iter().all(|b| *b == 0));
    }
}
pub fn start() -> Result<(), Box<dyn std::error::Error>> {
    super::event_log::event(
        "ui_start",
        serde_json::json!({"version":env!("CARGO_PKG_VERSION")}),
    );
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
    let connection = super::connection::Connection::load()?;
    let default_destination = PathBuf::from(home)
        .join(if cfg!(target_os = "macos") {
            "Movies"
        } else {
            "Videos"
        })
        .join("OpenRecorder");
    let destination = connection
        .as_ref()
        .and_then(|c| c.action("state", serde_json::json!({})).ok())
        .and_then(|s| s["root"].as_str().map(PathBuf::from))
        .unwrap_or(default_destination);
    let args: Vec<String> = std::env::args().collect();
    let benchmark = args
        .iter()
        .position(|a| a == "--benchmark-ui")
        .and_then(|index| {
            args.get(index + 1)
                .cloned()
                .zip(args.get(index + 2).cloned())
        });
    let idle = args
        .iter()
        .position(|a| a == "--idle-ui")
        .and_then(|index| args.get(index + 1))
        .and_then(|s| s.parse::<u64>().ok());
    let failed = Arc::new(AtomicBool::new(false));
    let result_flag = failed.clone();
    // The floating toolbar is retired. Neither the host's launch (`--desktop --config`), the old `--toolbar` spelling
    // nor the measurement modes bring it back; only the explicit development switch does.
    let toolbar_mode = toolbar_requested(&args, std::env::var("OPENRECORDER_DEV_TOOLBAR").ok().as_deref());
    let root = if toolbar_mode {
        egui::ViewportBuilder::default()
            .with_inner_size([TOOLBAR_W, TOOLBAR_H])
            .with_min_inner_size([TOOLBAR_W, TOOLBAR_H])
            .with_max_inner_size([TOOLBAR_W, TOOLBAR_H])
            .with_transparent(true)
            .with_resizable(false)
            .with_title("OpenRecorder")
            .with_decorations(false)
            .with_always_on_top()
    } else {
        // One transparent pixel that hosts the bubble, countdown and pickers. It is never painted, takes no
        // clicks and has no taskbar entry.
        egui::ViewportBuilder::default()
            .with_inner_size([1., 1.])
            .with_transparent(true)
            .with_resizable(false)
            .with_title("OpenRecorder")
            .with_decorations(false)
            .with_taskbar(false)
            .with_mouse_passthrough(true)
    };
    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        // Wayland may withhold frame callbacks for an inactive/off screen. A
        // blocking EGL swap must never prevent Stop, recovery or shutdown.
        vsync: !cfg!(target_os = "linux"),
        viewport: root,
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        }));
    }
    eframe::run_native(
        "OpenRecorder",
        options,
        Box::new(move |creation| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "Inter".into(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../../assets/fonts/Inter.ttf"
                ))),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "Inter".into());
            creation.egui_ctx.set_fonts(fonts);
            configure_native_styles(&creation.egui_ctx);
            creation.egui_ctx.set_theme(egui::ThemePreference::System);
            let _native_material = super::material::apply(creation);
            let mut state = State {
                styles_configured: true,
                screen_ready: scap::has_permission(),
                setup_open: connection.as_ref().is_some_and(|c| !c.setup_complete)
                    && benchmark.is_none()
                    && idle.is_none(),
                connection,
                destination,
                benchmark_failed: failed,
                ..Default::default()
            };
            state.prefs.root = state.destination.clone();
            state.confirmed.root = state.destination.clone();
            if let Some(conn) = &state.connection {
                let (command, human) = super::connection::commands(conn.clone(), creation.egui_ctx.clone());
                (state.command, state.human) = (Some(command), Some(human));
            }
            state.toolbar_mode = toolbar_mode;
            state.launched_ms = super::connection::now_ms();
            state.launch = new_launch_nonce();
            state.capabilities = super::capability::Capabilities::load(&state.capabilities_path());
            if benchmark.is_none() {
                match super::tray::Tray::new(creation.egui_ctx.clone()) {
                    Ok(tray) => state.tray = Some(tray),
                    Err(error) => {
                        super::event_log::event("tray_unavailable", serde_json::json!({"error":error}));
                        state.tray_error = Some(error);
                    }
                }
                // Home is opened when no tray can be used, at launch or after a later loss (see `home_fallback`).
                state.fallback_enabled = !toolbar_mode && idle.is_none();
            }
            if let Some(conn) = &state.connection {
                if let Ok(snapshot) = conn.action("state", serde_json::json!({})) {
                    state.preferences(&snapshot["settings"]);
                }
            }
            if let Some((directory, seconds)) = benchmark {
                let pixels = Arc::new(Mutex::new(super::camera_frame::Frame {width:1280,height:720,bgra:vec![0;1280*720*4],sequence:0}));
                let preview = pixels.clone();
                state.camera = Some(Camera {
                    child: None,
                    error: Arc::new(Mutex::new(None)),
                    pixels,
                    ready: Arc::new(AtomicBool::new(true)),
                    last_frame: None,
                    opened: Instant::now(),
                });
                state.benchmark = true;
                state.recording = true;
                state.worker = Some(thread::spawn(move || {
                    super::synthetic(
                        &["core".into(), "--synthetic".into(), directory, seconds],
                        Some(preview),
                    )
                    .map_err(|e| e.to_string())
                }));
            }
            if let Some(seconds) = idle {
                state.idle_deadline = Some(Instant::now() + Duration::from_secs(seconds));
            }
            if let Some(conn) = &state.connection {
                let _ = conn.action(
                    "native-window-ready",
                    serde_json::json!({"pid":std::process::id()}),
                );
            }
            Ok(Box::new(state))
        }),
    )
    .map_err(|e| e.to_string())?;
    if result_flag.load(Ordering::Relaxed) {
        return Err("Native UI benchmark failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod menu_tests {
    use super::fixtures::*;
    use super::super::{
        connection::{HumanAction, HumanCommand},
        controls::{Gate, Stage},
        tray::{self, testing, Action, IconKind},
    };
    use super::*;

    /// The menu-driven app: no toolbar, a tray drawn into memory, no host, no devices.
    fn menu_state() -> (State, Arc<Mutex<Vec<tray::View>>>) {
        let mut state = test_state();
        state.toolbar_mode = false;
        let (tray, drawn) = testing::tray();
        state.tray = Some(tray);
        // Device and display listing is exercised by its own tests; keep this one passive.
        state.devices_checked = Some(Instant::now());
        state.launched_ms = 1_000;
        state.launch = "launch-test-0001".into();
        (state, drawn)
    }
    fn last(drawn: &Arc<Mutex<Vec<tray::View>>>) -> tray::View {
        drawn.lock().unwrap().last().cloned().expect("the tray was drawn")
    }
    fn labels(view: &tray::View) -> Vec<String> {
        fn walk(items: &[tray::Item], out: &mut Vec<String>) {
            for i in items {
                out.push(i.label.clone());
                if let tray::Kind::Submenu(inner) = &i.kind {
                    walk(inner, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&view.items, &mut out);
        out
    }
    fn recording_for(state: &mut State, seconds: u64) {
        state.recording = true;
        state.generation = 3;
        state.options.progress.phase.store(1, Ordering::Relaxed);
        state.options.progress.fps.store(30, Ordering::Relaxed);
        state.started_at = Some(Instant::now() - Duration::from_secs(seconds));
    }
    static STOP: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn every_way_a_take_ends_clears_the_timer_the_square_and_every_label_in_the_same_update() {
        let ctx = egui::Context::default();
        for ending in ["stopped and saved", "failed", "cancelled before the first frame", "worker died"] {
            let (mut state, drawn) = menu_state();
            recording_for(&mut state, 46);
            state.tray_update(&ctx);
            let during = last(&drawn);
            assert_eq!((during.icon, during.title.as_str()), (IconKind::Square, "0:46"), "{ending}");
            assert!(labels(&during).contains(&"Stop Recording (0:46)".to_string()));
            // The worker is gone (whatever the reason): the next update must show nothing of the take.
            state.recording = false;
            state.worker = None;
            if ending == "failed" || ending == "worker died" {
                state.capture_notice = Some(super::super::controls::failure_notice("disk failed", 30, 1920, 46, 1380, 3));
            }
            state.tray_update(&ctx);
            let after = last(&drawn);
            let text = format!("{:?}", (labels(&after), &after.title, &after.tooltip, &after.spoken));
            for stale in ["0:46", "Stop Recording", "Saving", "Recording Stopped", "— Recording", ", recording,"] {
                assert!(!text.replace("Last recording stopped", "").contains(stale), "{ending}: {stale} survived: {text}");
            }
            assert_eq!((after.icon, after.title.as_str()), (IconKind::Disc, ""), "{ending}");
            assert_eq!(labels(&after)[0], "Record", "{ending}");
            assert!(state.started_at.is_none(), "{ending}: the timer itself is gone, not just hidden");
        }
    }
    #[test]
    fn stopping_shows_the_disc_and_saving_with_no_timer_and_nothing_to_press() {
        let ctx = egui::Context::default();
        let (mut state, drawn) = menu_state();
        recording_for(&mut state, 12);
        state.options.progress.phase.store(2, Ordering::Relaxed);
        state.tray_update(&ctx);
        let view = last(&drawn);
        assert_eq!((view.icon, view.title.as_str()), (IconKind::Disc, "Saving…"));
        assert_eq!(view.items[0].label, "Saving…");
        assert!(!view.items[0].enabled);
        assert!(state.started_at.is_none(), "capture has ended, so the clock has too");
        assert!(!labels(&view).iter().any(|l| l.contains("0:12")));
    }
    #[test]
    fn the_stage_follows_the_machine_and_the_record_press_starts_a_new_generation_with_a_clean_notice() {
        let (mut state, _) = menu_state();
        let now = Instant::now();
        assert_eq!(state.stage(now), Stage::Idle);
        state.countdown = Some(now + Duration::from_secs(2));
        assert!(matches!(state.stage(now), Stage::Countdown(2)));
        state.countdown = None;
        state.start = Some(StartAttempt { presence: None, opened_camera: false, opened_microphone: false, opened: false });
        assert_eq!(state.stage(now), Stage::Starting);
        state.start = None;
        recording_for(&mut state, 1);
        assert_eq!(state.stage(now), Stage::Recording);
        state.options.progress.phase.store(0, Ordering::Relaxed);
        assert_eq!(state.stage(now), Stage::Starting, "the encoder is still starting");
        state.options.progress.phase.store(2, Ordering::Relaxed);
        assert_eq!(state.stage(now), Stage::Stopping);
        state.recording = false;
        // A take that is finished but unconfirmed is Saving; waiting for a manual retry is idle with a notice.
        let dir = std::env::temp_dir();
        let mut pending = super::super::connection::PendingCompletion::finish(serde_json::json!("t"), dir);
        pending.last_error = "No space left on device".into();
        state.completion = Some(pending);
        state.completion_next = Some(now);
        assert_eq!(state.stage(now), Stage::Saving);
        state.completion_next = None;
        assert_eq!(state.stage(now), Stage::Idle);
        assert!(state.current_notice().unwrap().text.contains("isn't saved yet"));
        state.completion = None;
        // An accepted press is a new generation and clears the previous failure's notice.
        state.capture_notice = Some(super::super::controls::failure_notice("x", 30, 1920, 5, 150, 3));
        let before = state.generation;
        let ctx = egui::Context::default();
        state.record_press(&ctx, GrantSnapshot { screen: true, camera: Grant::SystemManaged, microphone: Grant::SystemManaged });
        assert_eq!(state.generation, before + 1);
        assert!(state.capture_notice.is_none());
        assert_eq!(state.stage(Instant::now()), Stage::Starting);
    }
    #[test]
    fn the_source_and_devices_are_named_by_what_the_system_reported_and_never_a_placeholder() {
        let (mut state, _drawn) = menu_state();
        state.options.selection = None;
        state.targets = vec![(false, 1, "Built-in Retina Display".into()), (false, 2, "LG UltraFine".into()), (true, 9, "Notes".into())];
        let report = progress_json_of(&state);
        assert_eq!(report["sources"].as_array().unwrap().len(), 2, "windows are not in the display list");
        // An explicit choice of a listed display is called by that display's own name, on every platform.
        state.options.selection = Some((false, 2));
        assert_eq!(progress_json_of(&state)["source"], serde_json::json!({"kind":"display","name":"LG UltraFine"}));
        state.options.selection = None;
        if cfg!(target_os = "linux") {
            // A desktop portal chooses the real source (a display or any window) on every take and never says which, so
            // the default is "Desktop picker" whatever the system lists: no display name is claimed for it.
            for listing in [vec![(false, 1, "Built-in Retina Display".to_string())], vec![(false, 1, String::new())], vec![]] {
                state.targets = listing;
                assert_eq!(progress_json_of(&state)["source"], serde_json::json!({"kind":"display","name":"Desktop picker"}));
            }
        } else {
            // The default is the first display the system lists, called by its own name.
            assert_eq!(report["source"], serde_json::json!({"kind":"display","name":"Built-in Retina Display"}), "the default is the first listed display, by its name");
            state.targets = vec![(false, 1, String::new())];
            assert_eq!(progress_json_of(&state)["source"]["name"], "Display", "a blank name is not made up");
            state.targets.clear();
            assert_eq!(progress_json_of(&state)["source"]["name"], "Display", "no listing, no name; never \"Main display\"");
        }
        // A camera that is chosen but not listed has no name here; a listed one is named; the microphone is its own name.
        state.camera_device_id = Some("UID-GONE".into());
        state.camera_names.clear();
        state.microphone_device_id = Some("MacBook Pro Microphone".into());
        let report = progress_json_of(&state);
        assert!(report["inputs"]["camera"].get("name").is_none(), "no fabricated camera name");
        assert_eq!(report["inputs"]["microphone"]["name"], "MacBook Pro Microphone");
        state.camera_names = vec![super::super::devices::CameraDevice { id: "UID-GONE".into(), name: "Studio Camera".into() }];
        assert_eq!(progress_json_of(&state)["inputs"]["camera"]["name"], "Studio Camera");
        // The status words are the one vocabulary and "armed" is never reported as live.
        state.camera_intent = true;
        assert_eq!(progress_json_of(&state)["inputs"]["camera"]["mark"], "armed");
        assert!(state.camera.is_none(), "choosing and arming opened nothing");
    }
    fn progress_json_of(state: &State) -> serde_json::Value {
        super::super::controls::progress_json(&state.snapshot(super::super::controls::Stage::Idle))
    }
    #[test]
    fn a_failed_take_leaves_one_plain_notice_with_the_raw_message_only_behind_details() {
        let (mut state, drawn) = menu_state();
        state.confirmed.quality = 2560;
        state.options.progress.fps.store(60, Ordering::Relaxed);
        state.options.progress.frames.store(100, Ordering::Relaxed);
        state.recording = false;
        state.generation = 5;
        state.finish_take(Ok(Err("Recording cannot keep up. Choose a lower resolution or frame rate.".into())));
        let notice = state.capture_notice.clone().unwrap();
        assert_eq!(notice.text, "Couldn't keep up at 60 fps, 2560 × 1440. Originals are kept.");
        assert!(notice.kept && notice.generation == 5);
        let ctx = egui::Context::default();
        state.tray_update(&ctx);
        let view = last(&drawn);
        assert!(labels(&view).contains(&"Open Last Recording".to_string()) && labels(&view).contains(&"Recording stopped early".to_string()));
        assert!(!labels(&view).iter().any(|l| l.contains("Choose a lower") || l.contains("keep up") || l == "Use 30 fps" || l == "Dismiss"), "the raw text stays in Home's technical details");
        // Opening the recording clears it from every surface (there is no Dismiss or frame-rate advice in the menu).
        testing::choose(state.tray.as_ref().unwrap(), Action::OpenLastRecording);
        state.tray_update(&ctx);
        assert!(!labels(&last(&drawn)).contains(&"Recording stopped early".to_string()));
        // A saved take leaves no failure notice at all.
        state.finish_take(Ok(Ok(())));
        assert!(state.capture_notice.is_none());
    }
    fn summary(status: &str, error: Option<&str>, out: Option<f64>, camera: Option<(u64, u64)>, audio: bool, originals: bool) -> serde_json::Value {
        serde_json::json!({"status":status,"error":error,"fps":60,"width":2560,"height":1440,"camera":camera.is_some(),
            "cameraWidth":camera.map(|c| c.0),"cameraHeight":camera.map(|c| c.1),"cadence":{"outputFps":out},
            "workload":{"quality":2560,"audio":audio,"originals":originals}})
    }
    /// A take is over: its capture.json is where the app learns what it measured.
    fn end_take(state: &mut State, summary: serde_json::Value, result: Result<(), String>) {
        state.options.progress.frames.store(100, Ordering::Relaxed);
        // The capture writes the target it was told to record; these tests stand in for it with the take's own.
        let mut summary = summary;
        if summary["workload"].get("source").is_none() {
            summary["workload"]["source"] = serde_json::json!(match state.take_source.as_str() { "unresolved" => "default", s => s });
        }
        *state.options.progress.summary.lock().unwrap() = Some(summary);
        state.finish_take(Ok(result));
    }

    // The macOS/Windows resolver: `source_key`/`prepare_take` name a real display id. A Linux desktop portal
    // chooses the source per take, so its State-level behaviour is covered by the portal tests below.
    #[test]
    #[cfg(not(target_os = "linux"))]
    fn sixty_is_learned_per_workload_and_a_lost_attempt_never_locks_the_choice_out() {
        use super::super::capability::Status;
        let (mut state, _) = menu_state();
        std::env::set_var("OPENRECORDER_HOME", std::env::temp_dir().join(format!("openrecorder-menu-caps-{}", std::process::id())));
        state.confirmed.fps = 60;
        state.confirmed.fps_chosen = true;
        state.confirmed.quality = 2560;
        state.confirmed.originals = true;
        state.take_source = "display:1".into();
        state.options.selection = Some((false, 1));
        let gate = "Recording cannot keep up. Choose a lower resolution or frame rate.";
        end_take(&mut state, summary("failed", Some(gate), None, None, false, false), Err(gate.into()));
        assert_eq!(state.fps60_entry().status, Status::FailedLast);
        assert_eq!(state.fps_in_use(), 60, "the person's choice is never lowered behind their back");
        end_take(&mut state, summary("completed", None, Some(59.9), None, false, false), Ok(()));
        assert_eq!(state.fps60_entry().status, Status::Qualified, "the screen-only take qualified the screen-only workload");
        // An unchosen default may be promoted for that workload; a stored 24 or an explicit 30 never.
        state.confirmed.fps = 30;
        state.confirmed.fps_chosen = false;
        assert_eq!(state.fps_in_use(), 60);
        // The same display with the microphone armed is a heavier, different workload: no promotion.
        state.microphone_intent = true;
        assert_eq!(state.fps60_entry().status, Status::Unknown);
        assert_eq!(state.fps_in_use(), 30, "audio was never measured");
        state.microphone_intent = false;
        // So is the camera (whose size has not been seen), and another display.
        state.camera_intent = true;
        state.camera_device_id = Some("cam-A".into());
        assert_eq!(state.fps_in_use(), 30, "a camera of unknown size can't borrow the screen-only result");
        state.camera_intent = false;
        state.options.selection = Some((false, 2));
        assert_eq!(state.fps_in_use(), 30);
        state.options.selection = Some((false, 1));
        assert_eq!(state.fps_in_use(), 60);
        state.confirmed.fps = 24;
        assert_eq!(state.fps_in_use(), 24);
        state.confirmed.fps = 30;
        state.confirmed.fps_chosen = true;
        assert_eq!(state.fps_in_use(), 30, "an explicit 30 is not raised, however good the evidence");
    }
    #[test]
    fn an_idle_app_with_everything_off_schedules_its_own_publication_and_the_host_stays_fresh() {
        // Permissions allowed, no inputs, no windows open, no other event: only the scheduled wakeups run.
        let (mut state, _) = menu_state();
        state.connection = Some(super::super::connection::Connection::for_test("http://127.0.0.1:1".into()));
        state.screen_ready = true;
        let mut now = 1_000_000u64;
        let mut gaps = vec![];
        let mut last_publish = None;
        let mut wakeups = 0;
        for _ in 0..200 {
            let before = state.heartbeat_for_test();
            let wake = state.publish_capture(Stage::Idle, now).expect("a host to tell");
            if state.heartbeat_for_test() != before {
                gaps.extend(last_publish.map(|t| now - t));
                last_publish = Some(now);
            }
            assert!((250..=4_000).contains(&wake), "a bounded, non-zero wakeup: {wake}");
            wakeups += 1;
            now += wake;
            if now > 1_000_000 + 120_000 {
                break;
            }
        }
        assert!(!gaps.is_empty() && gaps.iter().all(|g| *g <= 4_000), "published on its own, never silent for more than 4 s: {gaps:?}");
        assert!(wakeups <= 40, "about one wakeup per period, not a busy loop: {wakeups}");
        // The caller asks the toolkit for exactly that wakeup, and nothing else is needed to keep it fresh.
        let source = include_str!("desktop.rs");
        assert!(source.contains("if let Some(wake) = self.publish_capture(stage, super::connection::now_ms()) {\n            ctx.request_repaint_after(Duration::from_millis(wake));"));
        // No connection: nothing to tell, and no wakeup is scheduled for it.
        state.connection = None;
        assert_eq!(state.publish_capture(Stage::Idle, now), None);
    }
    thread_local! {
        /// The "operating system" for the transition tests: which display is main right now.
        static OS_MAIN: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(Some(1)) };
    }
    fn fake_two_displays() -> (Vec<(u32, egui::Rect)>, Option<u32>) {
        let rect = |x: f32| egui::Rect::from_min_size(egui::pos2(x, 0.), egui::vec2(2560., 1440.));
        (vec![(1, rect(0.)), (2, rect(2560.))], OS_MAIN.get())
    }
    /// What the capture writes about a take, built only from the options the capture was handed.
    fn capture_json_for(options: &super::super::capture::RecordOptions, status: &str, out: f64) -> serde_json::Value {
        serde_json::json!({"status":status,"error":null,"fps":options.fps,"width":2560,"height":1440,"camera":false,
            "cadence":{"outputFps":out},
            "workload":{"quality":options.quality,"audio":false,"originals":false,"source":super::super::capture::target_name(options.selection)}})
    }

    // The macOS/Windows resolver: `source_key`/`prepare_take` name a real display id. A Linux desktop portal
    // chooses the source per take, so its State-level behaviour is covered by the portal tests below.
    #[test]
    #[cfg(not(target_os = "linux"))]
    fn a_take_resolves_its_display_fresh_and_capture_key_and_metadata_all_name_that_one_display() {
        use super::super::capability::Status;
        let (mut state, _) = menu_state();
        std::env::set_var("OPENRECORDER_HOME", std::env::temp_dir().join(format!("openrecorder-menu-caps4-{}", std::process::id())));
        state.os.displays = fake_two_displays;
        state.confirmed.quality = 2560;
        state.confirmed.fps = 30;
        state.confirmed.fps_chosen = false;
        state.options.selection = None; // the person's preference stays "default" between takes
        // Display 1 is main; a take on it completes at 60 and qualifies display 1 (default is promoted from then on).
        OS_MAIN.set(Some(1));
        state.main_display = Some(1);
        let first = state.prepare_take().unwrap();
        assert_eq!((first.options.selection, first.source.as_str(), first.options.fps), (Some((false, 1)), "display:1", 30));
        state.take_source = first.source.clone();
        let mut done = capture_json_for(&first.options, "completed", 59.9);
        done["fps"] = serde_json::json!(60);
        end_take(&mut state, done, Ok(()));
        assert_eq!(state.prepare_take().unwrap().options.fps, 60, "same main display, same geometry: applicable");
        // The main display switches to 2 (same geometry) and the person presses Record before the 2 s cache refresh:
        // the cache still says 1, the system says 2.
        OS_MAIN.set(Some(2));
        assert_eq!(state.main_display, Some(1), "the cache is stale on purpose");
        assert_eq!(state.source_key(), "display:1", "what the UI would still show");
        let second = state.prepare_take().unwrap();
        assert_eq!(second.source, "display:2", "resolved fresh at Record, not read from the cache");
        assert_eq!(second.options.selection, Some((false, 2)), "and handed to the capture as an explicit target");
        assert_eq!(second.options.fps, 30, "display 1's qualification is not borrowed for display 2");
        assert_eq!(state.main_display, Some(2), "the cache is refreshed too");
        // The main display flips back before the capture adapter starts. The options already name display 2, so the
        // adapter can't resolve "main" to anything else; the metadata it writes names display 2; the key is display 2.
        OS_MAIN.set(Some(1));
        assert_eq!(second.options.selection, Some((false, 2)));
        let mut written = capture_json_for(&second.options, "completed", 30.0);
        assert_eq!(written["workload"]["source"], "display:2", "the metadata agrees with the target and the key");
        written["fps"] = serde_json::json!(60); // had it been a 60 fps take
        written["cadence"]["outputFps"] = serde_json::json!(60.0);
        state.take_source = second.source.clone();
        end_take(&mut state, written, Ok(()));
        // Attributed to display 2 alone: display 1 keeps its own record, and display 2 now has one.
        state.options.selection = Some((false, 2));
        assert_eq!(state.fps60_entry().status, Status::Qualified);
        state.options.selection = Some((false, 1));
        assert_eq!(state.fps60_entry().status, Status::Qualified);
        // A result whose metadata names a different display than the one the app keyed it on is not attributed at all.
        let mismatched = {
            let mut m = capture_json_for(&first.options, "completed", 60.0);
            m["fps"] = serde_json::json!(60);
            m
        };
        let mut caps = super::super::capability::Capabilities::default();
        caps.record_take(&mismatched, "display:2", None, super::super::codec(), 1);
        assert_eq!(caps.status(&caps.plan(60, 2560, super::super::codec(), "display:2", None, false, false)).status, Status::Unknown, "attributed to nobody");
        assert_eq!(caps.status(&caps.plan(60, 2560, super::super::codec(), "display:1", None, false, false)).status, Status::Unknown, "and not to display 1 either");
    }
    // macOS/Windows: the system lists displays, so a chosen one that has gone can be detected and refused. (A Linux
    // portal picks the source itself and has nothing to check it against; see the portal test below.)
    #[test]
    #[cfg(not(target_os = "linux"))]
    fn a_chosen_display_that_has_gone_is_refused_not_replaced() {
        let (mut state, _) = menu_state();
        state.os.displays = fake_two_displays;
        OS_MAIN.set(Some(1));
        // The person chose display 3; it is not attached any more. No take starts and nothing falls back to main.
        state.options.selection = Some((false, 3));
        assert!(state.prepare_take().err().unwrap().contains("isn't available"));
        state.begin_recording();
        assert!(!state.recording && state.worker.is_none(), "nothing was started");
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("isn't available")));
        // The default when the system reports no main display: the adapter's choice stands, unresolved, never promoted.
        state.options.selection = None;
        OS_MAIN.set(None);
        let unknown = state.prepare_take().unwrap();
        assert_eq!((unknown.options.selection, unknown.source.as_str()), (None, "unresolved"));
        // A display that is listed is used as chosen.
        OS_MAIN.set(Some(1));
        state.options.selection = Some((false, 2));
        let chosen = state.prepare_take().unwrap();
        assert_eq!((chosen.options.selection, chosen.source.as_str()), (Some((false, 2)), "display:2"));
    }
    /// Explicit rates are carried through a take's preparation untouched, on every platform.
    #[test]
    fn explicit_rates_pass_through_take_preparation_on_every_platform() {
        let (mut state, _) = menu_state();
        state.os.displays = fake_two_displays;
        OS_MAIN.set(Some(1));
        state.confirmed.fps_chosen = true;
        for rate in [24, 30, 60] {
            state.confirmed.fps = rate;
            assert_eq!(state.prepare_take().unwrap().options.fps, rate, "an explicit {rate} is honoured");
        }
    }
    /// The pure rules, identical on every platform (the platform only chooses the `portal` argument).
    #[test]
    fn take_source_rules_for_windows_portals_and_listed_displays() {
        assert_eq!(resolve_take_source(false, Some((true, 9)), &[1, 2], Some(1)).unwrap().key, "window");
        assert_eq!(resolve_take_source(true, None, &[1, 2], Some(1)).unwrap().key, "unresolved", "a portal chooses per take");
        assert_eq!(resolve_take_source(true, None, &[1, 2], Some(1)).unwrap().selection, None, "and is not told a display");
        for selection in [Some((false, 2)), Some((true, 9)), Some((false, 3))] {
            assert_eq!(resolve_take_source(true, selection, &[1, 2], Some(1)).unwrap().key, "unresolved", "{selection:?}: even an explicit choice is unresolved under a portal");
        }
        assert_eq!(resolve_take_source(false, Some((false, 2)), &[1, 2], Some(1)).unwrap().key, "display:2");
        assert_eq!(resolve_take_source(false, Some((false, 7)), &[], None).unwrap().key, "display:7", "unlisted systems keep the person's choice");
        assert!(resolve_take_source(false, Some((false, 3)), &[1, 2], Some(1)).is_err(), "a listed system refuses a display that has gone");
        assert!(resolve_take_source(true, Some((false, 3)), &[1, 2], Some(1)).is_ok(), "a portal has nothing to check it against");
    }
    // Linux: the desktop portal chooses the real source (any display or window) on every take and never says which,
    // so State-level, every target is unresolved, nothing is ever promoted to 60, and nothing is taken away.
    #[test]
    #[cfg(target_os = "linux")]
    fn on_a_portal_desktop_every_target_is_unresolved_and_no_take_is_promoted_to_60() {
        use super::super::capability::Status;
        let (mut state, _) = menu_state();
        std::env::set_var("OPENRECORDER_HOME", std::env::temp_dir().join(format!("openrecorder-menu-caps5-{}", std::process::id())));
        state.os.displays = fake_two_displays;
        state.confirmed.quality = 2560;
        state.confirmed.fps = 30;
        state.confirmed.fps_chosen = false;
        // Default, whatever the system says is main, and whatever the (stale) cache says.
        for (os_main, cache) in [(Some(1), Some(1)), (Some(2), Some(1)), (None, Some(1)), (Some(1), None)] {
            OS_MAIN.set(os_main);
            state.main_display = cache;
            state.options.selection = None;
            let take = state.prepare_take().unwrap();
            assert_eq!((take.options.selection, take.source.as_str(), take.options.fps), (None, "unresolved", 30), "default, main {os_main:?}");
            assert_eq!(state.source_key(), "unresolved");
        }
        // An explicit display, an unlisted display and a window: the portal decides, so none of them is a known source.
        for selection in [Some((false, 1)), Some((false, 2)), Some((false, 3)), Some((true, 9))] {
            state.options.selection = selection;
            let take = state.prepare_take().unwrap();
            assert_eq!((take.options.selection, take.source.as_str(), take.options.fps), (selection, "unresolved", 30), "{selection:?}");
        }
        // Takes complete at a full 60 on every one of those, with the metadata the capture would write: nothing qualifies.
        for selection in [None, Some((false, 1)), Some((true, 9))] {
            state.options.selection = selection;
            let take = state.prepare_take().unwrap();
            state.take_source = take.source.clone();
            let mut done = capture_json_for(&take.options, "completed", 60.0);
            done["fps"] = serde_json::json!(60);
            end_take(&mut state, done, Ok(()));
        }
        for selection in [None, Some((false, 1)), Some((false, 2)), Some((true, 9))] {
            state.options.selection = selection;
            assert_eq!(state.fps60_entry().status, Status::Unknown, "{selection:?}");
            assert_eq!(state.prepare_take().unwrap().options.fps, 30, "no automatic 60 for {selection:?}");
            assert_eq!(state.capabilities.publish_fps60(&state.planned_workload())["status"], "unknown", "Home is told unknown");
        }
        // Nothing is taken away: explicit rates are honoured as chosen.
        state.confirmed.fps_chosen = true;
        for rate in [24, 30, 60] {
            state.confirmed.fps = rate;
            assert_eq!(state.prepare_take().unwrap().options.fps, rate);
        }
        // A portal never refuses a take for a display it cannot check; the portal decides.
        state.options.selection = Some((false, 3));
        assert!(state.prepare_take().is_ok());
    }
    #[test]
    fn source_identity_is_a_real_display_id_or_unresolved_and_never_invented() {
        // macOS/Windows: a chosen display, the main display behind the default, a window, an unknown main display.
        assert_eq!(source_identity(false, Some((false, 7)), Some(1)), "display:7");
        assert_eq!(source_identity(false, None, Some(1)), "display:1");
        assert_eq!(source_identity(false, None, Some(2)), "display:2", "a changed main display is a different source");
        assert_eq!(source_identity(false, None, None), "unresolved", "no invented name for an unknown main display");
        assert_eq!(source_identity(false, Some((true, 42)), Some(1)), "window");
        // Linux: the portal picks the real source on every take (a display or any window) without saying which.
        for selection in [None, Some((false, 7)), Some((true, 42))] {
            assert_eq!(source_identity(true, selection, Some(1)), "unresolved", "{selection:?}");
        }
    }
    // The macOS/Windows resolver: `source_key`/`prepare_take` name a real display id. A Linux desktop portal
    // chooses the source per take, so its State-level behaviour is covered by the portal tests below.
    #[test]
    #[cfg(not(target_os = "linux"))]
    fn a_default_portal_window_or_changed_main_display_cannot_reuse_a_sixty_qualification() {
        use super::super::capability::Status;
        let (mut state, _) = menu_state();
        std::env::set_var("OPENRECORDER_HOME", std::env::temp_dir().join(format!("openrecorder-menu-caps3-{}", std::process::id())));
        state.confirmed.quality = 2560;
        state.confirmed.fps = 30;
        state.confirmed.fps_chosen = false;
        // The default source on a machine whose main display is 1 completes a 60 fps take.
        state.options.selection = None;
        state.main_display = Some(1);
        state.take_source = state.source_key();
        assert_eq!(state.take_source, "display:1");
        end_take(&mut state, summary("completed", None, Some(59.9), None, false, false), Ok(()));
        assert_eq!(state.fps_in_use(), 60, "the same main display, same geometry: applicable");
        // The main display changes to another with the very same geometry: not the same source.
        state.main_display = Some(2);
        assert_eq!(state.fps60_entry().status, Status::Unknown);
        assert_eq!(state.fps_in_use(), 30);
        // The main display is not reported at all.
        state.main_display = None;
        assert_eq!(state.fps_in_use(), 30);
        // The same display chosen explicitly is the same source again.
        state.options.selection = Some((false, 1));
        assert_eq!(state.fps_in_use(), 60);
        // A window of any size is never covered.
        state.options.selection = Some((true, 5));
        assert_eq!(state.fps_in_use(), 30);
        // A take whose source was unresolved qualifies nothing, even if it completes at full rate.
        state.take_source = "unresolved".into();
        end_take(&mut state, summary("completed", None, Some(60.0), None, false, false), Ok(()));
        state.options.selection = None;
        state.main_display = None;
        assert_eq!(state.fps_in_use(), 30);
        // Nothing is taken away: an explicit 60 is still allowed, and an explicit 30 or 24 is still honoured.
        state.confirmed.fps = 60;
        state.confirmed.fps_chosen = true;
        assert_eq!(state.fps_in_use(), 60);
        state.confirmed.fps = 24;
        assert_eq!(state.fps_in_use(), 24);
    }
    // The macOS/Windows resolver: `source_key`/`prepare_take` name a real display id. A Linux desktop portal
    // chooses the source per take, so its State-level behaviour is covered by the portal tests below.
    #[test]
    #[cfg(not(target_os = "linux"))]
    fn the_synthetic_check_and_the_heavy_take_that_failed_cannot_be_traded_for_each_other() {
        use super::super::capability::{CheckOutcome, Status};
        let (mut state, _) = menu_state();
        std::env::set_var("OPENRECORDER_HOME", std::env::temp_dir().join(format!("openrecorder-menu-caps2-{}", std::process::id())));
        state.confirmed.quality = 2560;
        state.confirmed.originals = true;
        state.options.selection = Some((false, 1));
        state.take_source = "display:1".into();
        state.capabilities.record_check(60, 2560, super::super::codec(), &CheckOutcome::Passed { output_fps: 60.0 }, 1);
        // The camera + microphone + originals configuration of the failed ac1552 take.
        state.camera_intent = true;
        state.camera_device_id = Some("cam-A".into());
        state.microphone_intent = true;
        state.take_camera = Some("cam-A".into());
        let heavy_gate = "Recording cannot keep up. Choose a lower resolution or frame rate.";
        end_take(&mut state, summary("failed", Some(heavy_gate), None, Some((900, 720)), true, true), Err(heavy_gate.into()));
        assert_eq!(state.fps60_entry().status, Status::FailedLast, "the heavy workload failed on its own evidence");
        assert_eq!(state.fps_in_use(), 30, "and the passing encoder check does not override that, or raise anything");
        let published = serde_json::json!(state.capabilities.publish_fps60(&state.planned_workload()));
        assert_eq!((published["status"].as_str(), published["encoder"]["status"].as_str()), (Some("failed-last"), Some("qualified")));
    }

    use super::super::connection::now_ms;
    fn command(state: &State, action: HumanAction, generation: Option<u64>, nonce: &str, age_ms: u64, ttl_ms: u64) -> HumanCommand {
        let issued_at = now_ms() - age_ms;
        HumanCommand { id: 1, issued_at, expires_at: issued_at + ttl_ms, launch: state.launch.clone(), nonce: nonce.into(), generation, action }
    }
    fn inbox_of(state: &mut State) -> super::super::connection::HumanInbox {
        let inbox: super::super::connection::HumanInbox = Default::default();
        state.human = Some(inbox.clone());
        inbox
    }

    #[test]
    fn a_relayed_stop_needs_the_running_take_and_a_late_or_early_command_changes_nothing() {
        let _serial = STOP.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        let inbox = inbox_of(&mut state);
        super::super::STOP_REQUEST.store(false, Ordering::Relaxed);
        let mut push = |c: HumanCommand| inbox.lock().unwrap().push_back(c);
        // Idle: Stop does nothing.
        push(command(&state, HumanAction::Stop, Some(3), "nonce-idle", 100, 4000));
        state.human_commands(&ctx);
        assert!(!super::super::STOP_REQUEST.load(Ordering::Relaxed));
        recording_for(&mut state, 5);
        // Another take's Stop, one issued before this process started, one for another launch, one with no take named.
        push(command(&state, HumanAction::Stop, Some(2), "nonce-old1", 100, 4000));
        let mut early = command(&state, HumanAction::Stop, Some(3), "nonce-old2", 100, 4000);
        early.issued_at = 999;
        push(early);
        let mut other = command(&state, HumanAction::Stop, Some(3), "nonce-old3", 100, 4000);
        other.launch = "launch-someone-else".into();
        push(other);
        push(command(&state, HumanAction::Stop, None, "nonce-old4", 100, 4000));
        state.human_commands(&ctx);
        assert!(!super::super::STOP_REQUEST.load(Ordering::Relaxed), "stale generation, pre-launch, other launch or unnamed take must not stop this one");
        push(command(&state, HumanAction::Stop, Some(3), "nonce-good", 100, 4000));
        state.human_commands(&ctx);
        assert!(super::super::STOP_REQUEST.load(Ordering::Relaxed));
        super::super::STOP_REQUEST.store(false, Ordering::Relaxed);
        assert!(inbox.lock().unwrap().is_empty(), "every command is consumed once");
        // The same request delivered again is a replay.
        push(command(&state, HumanAction::Stop, Some(3), "nonce-good", 100, 4000));
        state.human_commands(&ctx);
        assert!(!super::super::STOP_REQUEST.load(Ordering::Relaxed), "a replayed request number does nothing");
    }
    #[test]
    fn a_command_the_polling_thread_accepted_in_time_is_dropped_if_it_has_expired_by_execution() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        let inbox = inbox_of(&mut state);
        // Taken off the wire fresh, then the UI stalled: by the time it runs, its 4 s are long gone.
        inbox.lock().unwrap().push_back(command(&state, HumanAction::Record, Some(0), "nonce-stall", 10_000, 4000));
        state.human_commands(&ctx);
        assert!(state.start.is_none() && state.countdown.is_none() && !state.recording && state.generation == 0);
        assert!(requests().is_empty(), "nothing was requested of the OS");
        assert_eq!(state.consumed_len(), 0, "an expired control is refused before it is even entered");
    }
    #[test]
    fn a_relayed_record_never_starts_during_a_take_after_a_restart_or_for_an_older_take() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        let inbox = inbox_of(&mut state);
        let mut push = |c: HumanCommand| inbox.lock().unwrap().push_back(c);
        // Queued before this process started, or for a previous launch: dropped; nothing begins.
        let mut early = command(&state, HumanAction::Record, Some(0), "nonce-rec0", 100, 4000);
        early.issued_at = 500;
        push(early);
        let mut previous = command(&state, HumanAction::Record, Some(0), "nonce-rec1", 100, 4000);
        previous.launch = "launch-before-restart".into();
        push(previous);
        push(command(&state, HumanAction::Record, Some(7), "nonce-rec2", 100, 4000));
        push(command(&state, HumanAction::Record, None, "nonce-rec3", 100, 4000));
        state.human_commands(&ctx);
        assert!(state.start.is_none() && state.countdown.is_none() && !state.recording && state.generation == 0);
        assert!(requests().is_empty());
        // During a take a Record is busy, not queued behind it.
        recording_for(&mut state, 3);
        push(command(&state, HumanAction::Record, Some(3), "nonce-rec4", 100, 4000));
        state.human_commands(&ctx);
        assert!(state.start.is_none() && state.generation == 3);
    }
    #[test]
    fn menu_choices_that_are_not_record_or_a_toggle_open_nothing() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        state.menu_action(&ctx, Action::CameraDevice(Some("UID-A".into())));
        assert_eq!(state.camera_device_id.as_deref(), Some("UID-A"));
        assert!(state.camera_intent, "a pick arms the camera");
        assert!(state.camera.is_none() && state.camera_permission.is_none(), "but opens nothing");
        state.menu_action(&ctx, Action::MicDevice(None));
        state.menu_action(&ctx, Action::Display(7));
        assert_eq!(state.options.selection, Some((false, 7)));
        assert!(requests().is_empty(), "no permission or device request came from choosing");
        assert!(state.capture_notice.is_none());
    }
    #[test]
    fn shape_size_and_mirror_stay_live_while_recording_including_portrait() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        recording_for(&mut state, 9);
        state.menu_action(&ctx, Action::Shape("portrait"));
        state.menu_action(&ctx, Action::Size(224));
        state.menu_action(&ctx, Action::Mirror(false));
        assert_eq!((state.prefs.shape, state.prefs.size, state.prefs.mirror), (SHAPE_PORTRAIT, 224, false));
        assert!(state.bubble_dirty, "saved once the take is finished");
        assert_eq!(state.layout.lock().unwrap().shape, SHAPE_PORTRAIT);
        // Inputs and sources are not live: the relayed commands are refused by the stage.
        let stage = state.stage(Instant::now());
        let device = command(&state, HumanAction::Device { camera: true, uid: None }, Some(3), "nonce-dev0", 10, 4000);
        let session = super::super::controls::Session { launch: &state.launch, generation: 3, launched_at: 1_000, now: now_ms() };
        assert_eq!(super::super::controls::gate(&device, stage, &session, &mut Default::default()), Gate::Busy);
    }
    #[test]
    fn the_live_appearance_during_a_take_is_reported_reversible_and_saved_only_at_the_boundary() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        let look = |s: &State| progress_json_of(s)["appearance"].clone();
        let saved = look(&state);
        assert_eq!(saved, serde_json::json!({"shape":"circle","size":168,"mirror":true,"showCamera":true}));
        recording_for(&mut state, 9);
        // Hide, then show; mirror off, then on: each report says what is applied now, and the next press can reverse it.
        state.menu_action(&ctx, Action::ShowCamera(false));
        assert_eq!(look(&state)["showCamera"], false);
        state.menu_action(&ctx, Action::ShowCamera(true));
        assert_eq!(look(&state)["showCamera"], true);
        state.menu_action(&ctx, Action::Mirror(false));
        assert_eq!(look(&state)["mirror"], false);
        state.menu_action(&ctx, Action::Mirror(true));
        assert_eq!(look(&state)["mirror"], true);
        // The same through Home's relayed controls.
        for (n, (action, key, want)) in [(HumanAction::ShowCamera(false), "showCamera", serde_json::json!(false)), (HumanAction::Mirror(false), "mirror", serde_json::json!(false)), (HumanAction::ShowCamera(true), "showCamera", serde_json::json!(true)), (HumanAction::Mirror(true), "mirror", serde_json::json!(true))].into_iter().enumerate() {
            let c = command(&state, action.clone(), Some(3), &format!("nonce-look{n}"), 10, 4000);
            let launch = state.launch.clone();
            let session = super::super::controls::Session { launch: &launch, generation: 3, launched_at: 1_000, now: now_ms() };
            assert_eq!(super::super::controls::gate(&c, state.stage(Instant::now()), &session, &mut Default::default()), Gate::Run);
            match action {
                HumanAction::ShowCamera(on) => state.live_show_camera(on),
                HumanAction::Mirror(on) => state.live_look(None, None, Some(on)),
                _ => unreachable!(),
            }
            assert_eq!(look(&state)[key], want, "{key} after {action:?}");
        }
        // Shape and size too, and now several changes are pending at once.
        state.menu_action(&ctx, Action::Shape("portrait"));
        state.menu_action(&ctx, Action::Size(224));
        state.menu_action(&ctx, Action::ShowCamera(false));
        state.menu_action(&ctx, Action::Mirror(false));
        assert_eq!(look(&state), serde_json::json!({"shape":"portrait","size":224,"mirror":false,"showCamera":false}));
        // Nothing was saved during the take: the confirmed (saved) preferences are untouched and the change is only pending.
        assert_eq!((state.confirmed.shape, state.confirmed.size, state.confirmed.mirror, state.confirmed.show_camera), (SHAPE_CIRCLE, 168, true, true));
        assert!(state.bubble_dirty);
        state.flush_bubble_settings();
        assert!(state.bubble_dirty, "still pending while recording: no write mid-take");
        // At the boundary exactly those four values are what gets written, once.
        let mut sent = Vec::new();
        let written = settings_transaction(&state.confirmed.clone(), &state.prefs.clone(), |payload| {
            sent.push(payload.clone());
            Ok(serde_json::json!({"settings":payload["preferences"].clone()}))
        })
        .unwrap();
        assert!(written.is_some());
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["preferences"], serde_json::json!({"cameraShape":"portrait","cameraSize":224,"mirror":false,"showCamera":false}));
        // The publication key changes with the look, so Home hears of it at once and not on the next heartbeat.
        let key_of = |s: &State| format!("{:?}", s.snapshot(Stage::Recording).appearance);
        let before = key_of(&state);
        state.menu_action(&ctx, Action::Mirror(true));
        assert_ne!(before, key_of(&state));
        // And the real scheduler agrees: a change is published at once, an unchanged state is not republished early.
        state.connection = Some(super::super::connection::Connection::for_test("http://127.0.0.1:1".into()));
        let stage = state.stage(Instant::now());
        assert_eq!(stage, Stage::Recording);
        let published = |s: &State| s.heartbeat_for_test();
        state.publish_capture(stage, 1_000_000);
        let first = published(&state);
        state.publish_capture(stage, 1_000_050);
        assert_eq!(published(&state), first, "unchanged: nothing is sent early");
        state.menu_action(&ctx, Action::ShowCamera(true));
        state.publish_capture(stage, 1_000_100);
        assert_ne!(published(&state), first, "a look change is published at once");
    }
    thread_local! {
        static STATUS_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    fn status_read(grants: GrantSnapshot) -> GrantSnapshot {
        STATUS_READS.with(|c| c.set(c.get() + 1));
        grants
    }
    /// The real startup: no tray, `grants` unknown (None), the first update, an OS whose passive status read answers
    /// `grants`. Returns (camera listed, microphone listed) and checks nothing was requested, opened or armed.
    fn first_update_without_tray(snapshot: fn() -> GrantSnapshot) -> (bool, bool) {
        STATUS_READS.with(|c| c.set(0));
        let mut state = test_state();
        state.toolbar_mode = false;
        state.os.snapshot = snapshot;
        assert!(state.tray.is_none() && state.grants.is_none(), "the startup state: unknown permissions, no tray");
        state.tray_update(&egui::Context::default());
        for _ in 0..300 {
            state.poll_discovery();
            if state.camera_discovery.is_none() && state.microphone_discovery.is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(state.grants.is_some(), "the passive status was read before any discovery decision");
        assert_eq!(STATUS_READS.with(|c| c.get()), 1, "one silent status read");
        assert!(requests().is_empty(), "no permission request");
        assert!(state.camera.is_none() && state.microphone.is_none() && state.camera_permission.is_none() && state.microphone_permission.is_none() && state.screen_permission.is_none() && state.start.is_none(), "no device opened, no request future");
        assert!(!state.camera_intent && !state.microphone_intent, "nothing was switched on for the person");
        (state.camera_choices.is_some(), state.microphone_choices.is_some())
    }
    #[test]
    fn at_startup_with_no_tray_and_unknown_permissions_nothing_is_listed_until_the_passive_status_allows_it() {
        // (The display listing needs screen access; it is off here so no real display is queried.)
        fn screen_off(camera: Grant, microphone: Grant) -> GrantSnapshot {
            GrantSnapshot { screen: false, camera, microphone }
        }
        // macOS: access denied, restricted or not yet asked for both: the unknown-permission stand-in must not list.
        assert_eq!(first_update_without_tray(|| status_read(screen_off(Grant::Denied, Grant::Denied))), (false, false), "denied");
        assert_eq!(first_update_without_tray(|| status_read(screen_off(Grant::Restricted, Grant::Restricted))), (false, false), "restricted");
        assert_eq!(first_update_without_tray(|| status_read(screen_off(Grant::NotAsked, Grant::NotAsked))), (false, false), "not asked");
        // Each input follows its own status.
        assert_eq!(first_update_without_tray(|| status_read(screen_off(Grant::Denied, Grant::Allowed))), (false, true));
        assert_eq!(first_update_without_tray(|| status_read(screen_off(Grant::Allowed, Grant::NotAsked))), (true, false));
        // Granted: metadata only, and still nothing opened or requested (checked inside the helper).
        assert_eq!(first_update_without_tray(|| status_read(screen_off(Grant::Allowed, Grant::Allowed))), (true, true), "granted");
        // Windows and Linux report system-managed access and keep listing.
        assert_eq!(first_update_without_tray(|| status_read(screen_off(Grant::SystemManaged, Grant::SystemManaged))), (true, true), "system managed");
    }
    #[test]
    fn with_no_tray_the_idle_app_still_lists_granted_devices_for_homes_pickers_and_asks_for_nothing() {
        let list = |state: &mut State| {
            for _ in 0..300 {
                state.poll_discovery();
                if state.camera_discovery.is_none() && state.microphone_discovery.is_none() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        };
        let devices = |state: &State| progress_json_of(state)["devices"].clone();
        // No tray at all, access already granted, an idle app. (The display listing needs `screen_ready`; keep it off so
        // no real display is queried here.)
        let mut state = test_state();
        state.toolbar_mode = false;
        state.screen_ready = false;
        state.grants = Some(granted());
        assert!(state.tray.is_none());
        state.tray_update(&egui::Context::default());
        list(&mut state);
        assert_eq!(devices(&state), serde_json::json!({"camera":[{"uid":"UID-A","name":"Studio Camera","selected":false}],"microphone":[{"uid":"Built-in","name":"Built-in","selected":false}]}));
        // Listing opened nothing, asked for nothing and armed nothing.
        assert!(requests().is_empty(), "no permission or device request");
        assert!(state.camera.is_none() && state.microphone.is_none() && state.camera_permission.is_none() && state.microphone_permission.is_none() && state.start.is_none());
        assert!(!state.camera_intent && !state.microphone_intent, "nothing was switched on for the person");
        // The permission gate still holds: with camera access not yet asked, only the microphone is listed.
        let mut state = test_state();
        state.toolbar_mode = false;
        state.screen_ready = false;
        state.grants = Some(GrantSnapshot { camera: Grant::NotAsked, ..granted() });
        state.tray_update(&egui::Context::default());
        list(&mut state);
        let listed = devices(&state);
        assert_eq!(listed["camera"], serde_json::json!([]), "no camera listing without access, and never a request");
        assert_eq!(listed["microphone"].as_array().unwrap().len(), 1);
        assert!(requests().is_empty());
        // And the idle gate: during a take nothing is listed.
        let mut state = test_state();
        state.toolbar_mode = false;
        state.screen_ready = false;
        state.grants = Some(granted());
        recording_for(&mut state, 5);
        state.tray_update(&egui::Context::default());
        assert!(state.camera_discovery.is_none() && state.microphone_discovery.is_none());
    }
    #[test]
    fn show_camera_hides_only_the_bubble_live_and_settings_open_home_not_a_native_window() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        recording_for(&mut state, 9);
        assert!(state.layout.lock().unwrap().visible && state.prefs.show_camera, "shown by default");
        let (intent, microphone) = (state.camera_intent, state.microphone_intent);
        state.menu_action(&ctx, Action::ShowCamera(false));
        assert!(!state.prefs.show_camera && !state.layout.lock().unwrap().visible, "the recording stops drawing the bubble at once");
        assert!(state.bubble_dirty, "saved once the take is finished");
        assert_eq!((state.camera_intent, state.microphone_intent), (intent, microphone), "the camera's own on/off is not part of it");
        assert!(state.camera.is_none() && state.camera_permission.is_none(), "and nothing was opened");
        state.menu_action(&ctx, Action::ShowCamera(true));
        assert!(state.prefs.show_camera && state.layout.lock().unwrap().visible);
        // The host's saved choice is read back, defaulting to shown, and written with the other preferences.
        assert!(SettingsValues::from_host(&serde_json::json!({}), PathBuf::new()).show_camera);
        let hidden = SettingsValues::from_host(&serde_json::json!({"showCamera":false}), PathBuf::new());
        assert!(!hidden.show_camera && hidden.preferences()["showCamera"] == false);
        // Settings… is Home's Settings; it never opens a separate native window.
        let (mut idle, _) = menu_state();
        idle.menu_action(&ctx, Action::Settings);
        assert!(!idle.settings_open, "no native Settings window");
    }
    #[test]
    fn settings_say_where_capture_is_controlled_and_a_missing_tray_is_never_hidden() {
        assert_eq!(controls_words(false, false), ("Home", "No tray on this desktop — capture controls are here in Home."));
        assert_eq!(controls_words(false, true).0, "Home");
        assert_eq!(controls_words(true, true).0, "Menu bar");
        assert_eq!(controls_words(true, false).0, "Tray");
    }
    #[test]
    fn the_countdown_appears_in_the_middle_of_the_display_being_recorded() {
        let second = egui::Rect::from_min_size(egui::pos2(1920., 0.), egui::vec2(2560., 1440.));
        assert_eq!(hud_position(Some(second), Some(egui::vec2(1920., 1080.))), egui::pos2(1920. + 1280. - 70., 720. - 70.));
        assert_eq!(hud_position(None, Some(egui::vec2(1440., 900.))), egui::pos2(650., 380.));
        assert_eq!(hud_position(None, None), egui::pos2(200., 200.));
    }
    #[test]
    fn the_hosts_own_launch_arguments_never_bring_the_toolbar_back() {
        let launch = |extra: &[&str]| std::iter::once("OpenRecorder").chain(extra.iter().copied()).map(String::from).collect::<Vec<_>>();
        // How the library host starts the app now, how an earlier build did, and a double-click.
        assert!(!toolbar_requested(&launch(&["--desktop", "--config", "/c"]), None));
        assert!(!toolbar_requested(&launch(&["--toolbar", "--config", "/c"]), None));
        assert!(!toolbar_requested(&launch(&[]), None));
        assert!(!toolbar_requested(&launch(&["--desktop"]), Some("0")));
        // The measurement modes run the same menu-driven app.
        assert!(!toolbar_requested(&launch(&["--desktop", "--idle-ui", "5"]), None));
        assert!(!toolbar_requested(&launch(&["--desktop", "--benchmark-ui", "/o", "5"]), None));
        // Only the explicit development switches do.
        assert!(toolbar_requested(&launch(&["--dev-toolbar"]), None));
        assert!(toolbar_requested(&launch(&["--desktop", "--dev-toolbar", "--config", "/c"]), None));
        assert!(toolbar_requested(&launch(&[]), Some("1")));
    }
    #[test]
    fn without_a_toolbar_hide_is_a_no_op_and_a_remote_show_goes_to_home() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        state.camera_intent = true;
        state.hide(&ctx);
        assert!(!state.hidden, "there is no toolbar to hide, and hiding must not release inputs or the bubble");
        state.show_toolbar(&ctx);
        // Home was asked for (no library in this test, so the one honest notice appears) and nothing was opened.
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("Library is unavailable")), "{:?}", state.toast.as_ref().map(|t| &t.text));
        assert!(state.camera.is_none() && state.camera_permission.is_none() && state.start.is_none());
        assert!(requests().is_empty());
        // Pickers and notices hang from the display, not from a toolbar.
        let (rect, display) = state.toolbar_rect(&ctx);
        assert_eq!(rect.height(), 0.);
        assert!(display.is_some_and(|d| d.contains(rect.min)));
    }
    #[test]
    fn a_tray_lost_after_startup_opens_home_once_when_it_is_closed_and_not_in_a_stream() {
        use super::super::tray::Availability;
        let (mut state, _) = menu_state();
        state.fallback_enabled = true;
        let availability = state.tray.as_ref().unwrap().availability();
        let opened = |s: &State| s.toast.is_some();
        // Present at launch: nothing happens, however long it stays that way.
        state.home_fallback_at(0);
        state.home_fallback_at(60_000);
        assert!(!opened(&state) && !state.fallback.pending());
        // The watcher goes away later with Home closed (no connection here means "Home never read the state").
        availability.lock().unwrap().read(super::super::tray::Seen::No, super::super::tray::Seen::Unknown);
        state.home_fallback_at(100_000);
        assert!(state.fallback.pending() && !opened(&state), "a short grace first");
        state.home_fallback_at(102_000);
        assert!(opened(&state), "then Home is asked for");
        // The same loss does not ask again.
        state.toast = None;
        state.home_fallback_at(110_000);
        state.home_fallback_at(300_000);
        assert!(!opened(&state));
        // Home was opened at 102 000. The tray returns, then is lost again inside the minute (host gone, watcher
        // still there). That loss is deferred, not discarded: it opens Home once the cooldown ends, and not before.
        *availability.lock().unwrap() = Availability::PRESENT;
        state.home_fallback_at(110_000);
        availability.lock().unwrap().read(super::super::tray::Seen::Yes, super::super::tray::Seen::No);
        state.home_fallback_at(120_000);
        state.home_fallback_at(130_000);
        state.home_fallback_at(161_999);
        assert!(!opened(&state) && state.fallback.pending(), "inside the cooldown");
        state.home_fallback_at(162_000);
        assert!(opened(&state), "the continuing loss opens Home when the cooldown ends");
        state.toast = None;
        state.home_fallback_at(400_000);
        assert!(!opened(&state) && !state.fallback.pending(), "and only once");
        // A loss that recovers before it is acted on opens nothing at all.
        *availability.lock().unwrap() = Availability::PRESENT;
        state.home_fallback_at(401_000);
        availability.lock().unwrap().read(super::super::tray::Seen::No, super::super::tray::Seen::Unknown);
        state.home_fallback_at(410_000);
        *availability.lock().unwrap() = Availability::PRESENT;
        state.home_fallback_at(415_000);
        state.home_fallback_at(500_000);
        assert!(!opened(&state) && !state.fallback.pending());
        // The development toolbar and the measurement modes never use the fallback.
        let (mut quiet, _) = menu_state();
        quiet.fallback_enabled = false;
        quiet.tray.as_ref().unwrap().availability().lock().unwrap().read(super::super::tray::Seen::No, super::super::tray::Seen::Unknown);
        quiet.home_fallback_at(0);
        quiet.home_fallback_at(10_000);
        assert!(!opened(&quiet) && !quiet.fallback.pending());
    }
    #[test]
    fn a_missing_tray_is_published_as_such_and_the_untouched_state_publishes_no_timer() {
        let ctx = egui::Context::default();
        let (mut state, _) = menu_state();
        state.tray = None;
        state.tray_error = Some("No StatusNotifierHost exists".into());
        let snapshot = state.snapshot(Stage::Idle);
        assert!(!snapshot.tray_available);
        let json = super::super::controls::progress_json(&snapshot);
        assert_eq!((json["trayAvailable"].as_bool(), json["trayNote"].as_str()), (Some(false), Some("No StatusNotifierHost exists")));
        assert!(json.get("elapsedMs").is_none());
        // Without a tray the update still runs and draws nothing.
        state.tray_update(&ctx);
        assert_eq!(json["phase"], "idle");
    }
    #[test]
    fn the_menu_and_the_published_state_come_from_the_same_stage_and_generation() {
        let ctx = egui::Context::default();
        let (mut state, drawn) = menu_state();
        recording_for(&mut state, 30);
        state.tray_update(&ctx);
        let stage = state.stage(Instant::now());
        let json = super::super::controls::progress_json(&state.snapshot(stage));
        assert_eq!((json["phase"].as_str(), json["generation"].as_u64()), (Some("recording"), Some(3)));
        let elapsed = json["elapsedMs"].as_u64().unwrap();
        assert!((30_000..32_000).contains(&elapsed), "{elapsed}");
        assert_eq!(last(&drawn).title, "0:30");
    }
}
