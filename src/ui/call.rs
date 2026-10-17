//! The call UI, drawn the way WhatsApp's desktop app draws it: a call that rings is a card in the
//! bottom-left corner of the main window, with who is calling and the answers, and a call that is
//! up (answered, or placed from here) gets a small window of its own in the top-right corner of the
//! screen. A call that rings while the main window is not in front rings in that window instead.
//!
//! A voice call's window holds who is on the line, how long the call has been up, bars that move
//! with the other side's voice, and the controls. A video call's window shows the other side's
//! picture over most of it, with our own camera in a corner, and grows to make room for it; a group
//! call's window holds a tile for each person on it. The window can be resized and expanded, but
//! the call never takes over the main window.
//!
//! Nothing here decides what a call is doing. The phase, the point the duration counts from, the
//! mute state and the device selections all come from [`crate::calls::CallUpdate`], which the
//! worker fills from the protocol's own events. This module only draws them, and asks the app for
//! something when the user presses a button.
//!
//! The call window is a separate egui viewport with its own event loop turn, so it keeps drawing
//! while the main window is minimized. It cannot borrow the [`App`]: the app hands it a
//! [`CallView`] and the newest [`Frames`] each frame through [`CallWindow`], and reads its
//! [`CallRequest`]s back.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Layout, Rect, Sense, TextureOptions, Vec2, pos2, vec2};

use crate::app::{App, CallPerson, CallTexture};
use crate::calls::{CallOutcome, CallPhase, CallUpdate, DeviceList, PeerAudio, SilenceReason};
use crate::i18n::{Locale, gettext};
use crate::model::Action;
use crate::theme::{self, Icon, Palette};
use crate::ui::widgets;

/// The ringing card's area, so it keeps one place in the layer order for the life of the call.
const CARD: &str = "zapfast-call-card";
/// The call window's viewport.
const WINDOW: &str = "zapfast-call-window";
/// How long a new call window waits for KWin to be ready to place it.
const PLACE_WAIT: Duration = Duration::from_secs(1);
/// The ringing card's width, including its margins, and with a third answer for a video call.
const CARD_WIDTH: f32 = 340.0;
const CARD_WIDTH_VIDEO: f32 = 392.0;
/// The ringing card's round buttons.
const CARD_CONTROL: f32 = 42.0;
/// The call window's width, in points, for a voice call.
const WINDOW_WIDTH: f32 = 300.0;
/// The call window's size when a video call opens it or turns a voice call's window into one, and
/// the least it may be resized to.
const VIDEO_SIZE: Vec2 = vec2(420.0, 600.0);
const VIDEO_MIN: Vec2 = vec2(300.0, 420.0);
/// The same for a group call's tiles.
const GROUP_SIZE: Vec2 = vec2(640.0, 520.0);
const GROUP_MIN: Vec2 = vec2(340.0, 380.0);
/// The most a resizable call window is held to: in effect, no limit.
const NO_LIMIT: Vec2 = vec2(16_384.0, 16_384.0);
/// Space between the call window's edges and what it draws.
const MARGIN: f32 = 14.0;
/// The avatar in the call window.
const AVATAR: f32 = 72.0;
/// The call window's round controls, at most.
const CONTROL: f32 = 52.0;
/// The least gap between them.
const CONTROL_GAP: f32 = 8.0;
/// The height of the bar the controls sit in under a video call's picture.
const CONTROL_BAR: f32 = 52.0 + 2.0 * 12.0;
/// The device pickers' width in the call window, and the height each one takes.
const PICKER_WIDTH: f32 = 260.0;
const PICKER_HEIGHT: f32 = 52.0;
/// The level bars under the status: how many, how tall, and how often they move.
const BARS: usize = 25;
const BARS_HEIGHT: f32 = 24.0;
const LEVEL_STEP: Duration = Duration::from_millis(50);
/// The height of one warning line under the status.
const WARNING_LINE: f32 = 18.0;
/// The buttons in the call window's top corner: the pin, and expand.
const PIN: f32 = 26.0;
/// How much of a video call's picture our own camera's corner may take.
const PREVIEW_SHARE: f32 = 0.3;
/// The plate under words drawn over a video picture: dark in either theme, because the picture is
/// not themed.
const VIDEO_SCRIM: Color32 = Color32::from_black_alpha(150);
/// The most tiles a group call lays out; the rest are counted on the last.
const MAX_TILES: usize = 12;
/// The gap between tiles.
const TILE_GAP: f32 = 8.0;
/// The shape a tile is laid out at, wide as a call's picture.
const TILE_ASPECT: f32 = 4.0 / 3.0;
/// How many faces a ringing group call shows before it counts the rest, and their size.
const FACES: usize = 5;
const FACE: f32 = 30.0;

/// What the call window shows, as the app last handed it over.
#[derive(Clone, Debug, PartialEq)]
pub struct CallView {
    pub call: CallUpdate,
    /// The call's own name: "Locked chat" where the app would not say more.
    pub peer: String,
    pub picture: Option<PathBuf>,
    /// Who rang a group call, by name.
    pub caller: Option<String>,
    /// The people on a group call.
    pub people: Vec<CallPerson>,
    /// Ourselves, as our own tile on a group call shows us.
    pub me: Option<CallPerson>,
    pub palette: Palette,
    pub locale: Locale,
    pub devices: DeviceList,
    /// Whether the window should stay above other windows.
    pub on_top: bool,
    /// Whether a ringing call offers Decline and Accept: not on an account in anonymous mode.
    pub answerable: bool,
}

/// The newest pictures of a video call, as the app last handed them over: the other side's, our
/// own camera's, and each group member's whose camera is on.
#[derive(Clone, Default)]
pub struct Frames {
    pub remote: Option<Arc<egui::ColorImage>>,
    pub local: Option<Arc<egui::ColorImage>>,
    pub members: HashMap<String, Arc<egui::ColorImage>>,
}

impl Frames {
    /// Whether `other` holds the very same pictures: a new frame is a new allocation, so this tells
    /// a frame that arrived from one that did not without comparing pixels.
    pub fn same(&self, other: &Frames) -> bool {
        let same =
            |a: &Option<Arc<egui::ColorImage>>, b: &Option<Arc<egui::ColorImage>>| match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
        same(&self.remote, &other.remote)
            && same(&self.local, &other.local)
            && self.members.len() == other.members.len()
            && self.members.iter().all(|(id, frame)| {
                other
                    .members
                    .get(id)
                    .is_some_and(|known| Arc::ptr_eq(known, frame))
            })
    }
}

/// What the call window asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum CallRequest {
    /// An action the app applies as if a view in the main window had pushed it.
    Action(Action),
    /// Keep the window above other windows, or stop.
    OnTop(bool),
    /// The window was closed: the app forgets the call it showed.
    Dismiss,
    /// The window of a call that is still ringing was closed: the call keeps ringing, and goes back
    /// to the card in the main window.
    Hide,
}

/// How the call window lays a call out, which also sets how big it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// Who is on the line and the controls, in a small window of a fixed size: a voice call, and
    /// any call while it rings.
    Voice,
    /// A one-to-one video call: the other side's picture fills the window.
    Video,
    /// A group call: a tile for each person on it.
    Group,
}

impl Shape {
    fn of(view: &CallView) -> Self {
        let call = &view.call;
        if call.phase == CallPhase::Incoming {
            Shape::Voice
        } else if call.group.is_some() && !view.people.is_empty() {
            Shape::Group
        } else if call.video && call.group.is_none() {
            Shape::Video
        } else {
            Shape::Voice
        }
    }
}

/// The textures the call window draws a video call's pictures through, updated in place rather
/// than made afresh for every frame.
#[derive(Default)]
pub struct Textures {
    pub remote: Option<CallTexture>,
    pub local: Option<CallTexture>,
    pub members: HashMap<String, CallTexture>,
}

/// The state the app and the call window share.
#[derive(Default)]
pub struct CallWindow {
    /// What to draw; `None` once there is no call to show, which hides the window.
    pub view: Option<CallView>,
    /// The call's newest pictures.
    pub frames: Frames,
    /// What the window asked for since the app last looked.
    pub requests: Vec<CallRequest>,
    /// Whether the window shows its device pickers.
    pub devices_open: bool,
    /// The textures the pictures are drawn through.
    pub textures: Textures,
    /// The peer's loudness, newest last, one sample per [`LEVEL_STEP`].
    levels: VecDeque<f32>,
    sampled: Option<Instant>,
    /// Whether the window was last asked to stay above the others; `None` for a new window.
    applied_on_top: Option<bool>,
    /// The shape the window was last given, and the height of a voice window.
    shape: Option<Shape>,
    height: f32,
    /// KWin being asked to place the new window as it appears.
    placing: Option<Placing>,
}

/// KWin being asked to place a new call window as it appears, on KDE Plasma under Wayland.
#[derive(Debug)]
struct Placing {
    since: Instant,
    /// Set once KWin waits for the window (`true`), or could not be asked (`false`).
    placed: Arc<OnceLock<bool>>,
}

impl CallWindow {
    /// Forgets what belonged to the window of the call that ended, so the next call's window starts
    /// fresh.
    pub fn reset(&mut self) {
        self.levels.clear();
        self.sampled = None;
        self.applied_on_top = None;
        self.shape = None;
        self.height = 0.0;
        self.placing = None;
        self.devices_open = false;
        self.textures = Textures::default();
        self.frames = Frames::default();
    }
}

/// The call window's state, shared between the app and the window's own viewport.
pub type SharedCallWindow = Arc<Mutex<CallWindow>>;

/// The call window's viewport id, so the app can ask it to repaint.
pub fn window_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of(WINDOW)
}

/// Draws the call UI, if there is a call: the ringing card in this window, or the call window.
pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(call) = app.call.clone() else {
        return;
    };
    if app.call_in_window() {
        open_window(app, ctx);
    } else {
        card(app, ctx, &call);
    }
}

/// What a ringing call says it is: a one-to-one call its kind, a group call also who is calling,
/// as WhatsApp's own prompt does. The desktop notification says the same.
pub(crate) fn incoming_text(locale: Locale, call: &CallUpdate, caller: Option<&str>) -> String {
    match (call.group.is_some(), caller, call.video) {
        (false, _, false) => gettext(locale, "Incoming voice call").into_owned(),
        (false, _, true) => gettext(locale, "Incoming video call").into_owned(),
        (true, Some(name), false) => {
            gettext(locale, "Group voice call from {name}").replace("{name}", name)
        }
        (true, Some(name), true) => {
            gettext(locale, "Group video call from {name}").replace("{name}", name)
        }
        (true, None, false) => gettext(locale, "Incoming group voice call").into_owned(),
        (true, None, true) => gettext(locale, "Incoming group video call").into_owned(),
    }
}

/// The card in the bottom-left corner: who is calling, and Decline and Accept (with or without
/// video, for a video call). A call that rang and was never answered keeps the card for a moment
/// to say how it ended.
fn card(app: &mut App, ctx: &egui::Context, call: &CallUpdate) {
    let palette = app.palette;
    let locale = app.locale;
    let peer = app.call_name();
    let picture = app.call_avatar();
    let caller = app.call_caller_name();
    let ringing = call.phase == CallPhase::Incoming;
    // Anonymous, the card only says who is calling.
    let answerable = ringing && app.call_answerable();
    let video = answerable && call.video;
    let card_width = if video { CARD_WIDTH_VIDEO } else { CARD_WIDTH };
    let mut actions = Vec::new();
    egui::Area::new(egui::Id::new(CARD))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::LEFT_BOTTOM, vec2(16.0, -16.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(if palette.dark {
                    Color32::from_rgb(28, 32, 36)
                } else {
                    palette.panel
                })
                .stroke(egui::Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(14))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 4],
                    blur: 18,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(ui, |ui| {
                    let width = card_width - 28.0;
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        if call.group.as_ref().is_some_and(|group| group.link) {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                            paint_link_avatar(ui, &palette, rect);
                        } else {
                            widgets::avatar(
                                ui,
                                &palette,
                                &peer,
                                &call.chat,
                                44.0,
                                picture.as_deref(),
                            );
                        }
                        ui.add_space(10.0);
                        let count = match (answerable, video) {
                            (false, _) => 0.0,
                            (true, false) => 2.0,
                            (true, true) => 3.0,
                        };
                        let buttons = count * CARD_CONTROL + (count - 1.0).max(0.0) * 10.0 + 8.0;
                        let text = (width - 54.0 - buttons).max(60.0);
                        ui.allocate_ui_with_layout(
                            vec2(text, CARD_CONTROL),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(text);
                                theme::text(ui, &peer, theme::semibold(15.0), palette.text);
                                let (line, color) = if ringing && !answerable {
                                    (
                                        gettext(locale, "Anonymous mode: the call is only logged")
                                            .into_owned(),
                                        palette.secondary,
                                    )
                                } else if ringing {
                                    (
                                        incoming_text(locale, call, caller.as_deref()),
                                        palette.secondary,
                                    )
                                } else {
                                    (status(locale, call), status_color(call, &palette))
                                };
                                theme::text(ui, line, theme::medium(12.5), color);
                            },
                        );
                        if answerable {
                            ui.add_space(8.0);
                            ui.spacing_mut().item_spacing.x = 10.0;
                            answers(ui, &palette, locale, call.video, CARD_CONTROL, &mut actions);
                        }
                    });
                });
        });
    app.actions.extend(actions);
}

/// Decline and Accept, as round buttons in a row; a video call is accepted with video or with
/// voice only.
fn answers(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: Locale,
    video: bool,
    diameter: f32,
    actions: &mut Vec<Action>,
) {
    let tip = gettext(locale, "Decline").into_owned();
    if control(
        ui,
        Icon::PhoneOff,
        diameter,
        palette.danger,
        Color32::WHITE,
        &tip,
        true,
    )
    .clicked()
    {
        actions.push(Action::DeclineCall);
    }
    if video {
        let tip = gettext(locale, "Accept with voice only").into_owned();
        if control(
            ui,
            Icon::Phone,
            diameter,
            palette.surface_active,
            palette.text,
            &tip,
            true,
        )
        .clicked()
        {
            actions.push(Action::AnswerCallWithoutVideo);
        }
        let tip = gettext(locale, "Accept with video").into_owned();
        if control(
            ui,
            Icon::Video,
            diameter,
            palette.accent,
            palette.on_accent,
            &tip,
            true,
        )
        .clicked()
        {
            actions.push(Action::AnswerCall);
        }
    } else {
        let tip = gettext(locale, "Accept").into_owned();
        if control(
            ui,
            Icon::Phone,
            diameter,
            palette.accent,
            palette.on_accent,
            &tip,
            true,
        )
        .clicked()
        {
            actions.push(Action::AnswerCall);
        }
    }
}

/// Keeps the call window open. Its contents come from [`CallWindow`], which the app fills in
/// [`App::background_frame`], so this only has to say that the window exists and what it is called.
fn open_window(app: &App, ctx: &egui::Context) {
    let shared = Arc::clone(&app.call_window);
    let waker = app.waker();
    let title = window_title(app);
    if !placement_ready(app, Some(ctx), &title) {
        return;
    }
    let monitor = app.main_monitor;
    ctx.show_viewport_deferred(window_id(), viewport(app, title), move |ui, class| {
        window(
            ui,
            &shared,
            &waker,
            monitor,
            class == egui::ViewportClass::EmbeddedWindow,
        );
    });
}

/// The call window's title: the call's own name, as the app last handed it over.
fn window_title(app: &App) -> String {
    app.call_window
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .view
        .as_ref()
        .map(|view| view.peer.clone())
        .unwrap_or_default()
}

/// How the call window opens: a voice call's size, its icon, and the top-right corner of the
/// screen. A video or group call's window grows from there on its first frame.
fn viewport(app: &App, title: String) -> egui::ViewportBuilder {
    let size = [WINDOW_WIDTH, base_height(false)];
    // Its size is set from what it shows, through the minimum and the maximum: a window made
    // unresizable on Wayland keeps the size it opened with, which would leave no room for the device
    // pickers or a video call's picture.
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id(std::env::var("FLATPAK_ID").unwrap_or_else(|_| "zapfast".to_owned()))
        .with_inner_size(size)
        .with_min_inner_size(size)
        .with_max_inner_size(size)
        .with_icon(window_icon());
    // The top-right corner of the screen the main window is on. Wayland ignores a position, and
    // KDE's is set by the KWin script instead.
    match app.main_monitor {
        Some(monitor) if crate::keep_above::method() == crate::keep_above::Method::WindowLevel => {
            builder.with_position(corner(monitor, WINDOW_WIDTH))
        }
        _ => builder,
    }
}

/// Where a call window `width` wide goes on `monitor`: its top-right corner.
fn corner(monitor: Rect, width: f32) -> egui::Pos2 {
    pos2(
        monitor.right() - width - crate::keep_above::EDGE,
        monitor.top() + crate::keep_above::EDGE,
    )
}

/// The call window as a window of its own, for a call that rings while the main window is in the
/// tray: there is no main window to hold it, so it opens in the main window's place.
pub fn standalone_viewport(app: &App) -> egui::ViewportBuilder {
    viewport(app, window_title(app))
}

/// Whether the standalone call window may open yet; see [`placement_ready`]. The app runs without
/// a window then, and asks again on its next tick.
pub fn standalone_ready(app: &App) -> bool {
    placement_ready(app, None, &window_title(app))
}

/// One frame of the standalone call window.
pub fn standalone(ui: &mut egui::Ui, app: &App) {
    window(ui, &app.call_window, &app.waker(), app.main_monitor, false);
}

/// How long the standalone call window may go without being drawn before the main window opens in
/// its place. A compositor that never shows a new window gives it no frames, and eframe then only
/// runs its logic: the call would ring with nothing on screen to answer it.
const UNDRAWN_WAIT: Duration = Duration::from_secs(3);

/// What the standalone call window does after a frame of the app's logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standalone {
    /// The call is still shown here.
    Stay,
    /// ZapFast quits.
    Quit,
    /// The main window was asked for, or this one was never drawn: it opens in this one's place.
    Main,
    /// The call is no longer shown here: back to the tray.
    Hide,
}

/// The standalone call window's own state between frames.
#[derive(Debug)]
pub struct StandaloneWindow {
    opened: Instant,
    drawn: bool,
    closing: Option<Standalone>,
    shown: Option<(Option<bool>, Option<bool>)>,
}

impl Default for StandaloneWindow {
    fn default() -> Self {
        Self {
            opened: Instant::now(),
            drawn: false,
            closing: None,
            shown: None,
        }
    }
}

impl StandaloneWindow {
    /// Runs the app's logic for one frame of the standalone call window and closes the window when
    /// it has no more reason to be open. Closing takes one more frame, which the close asks for.
    pub fn logic(&mut self, app: &mut App, ctx: &egui::Context) -> Standalone {
        app.background_frame(ctx);
        let shown = ctx.input(|input| (input.viewport().minimized, input.viewport().occluded));
        if self.shown != Some(shown) {
            log::info!(
                "[WINDOW] call window minimized={:?} occluded={:?} drawn={}",
                shown.0,
                shown.1,
                self.drawn
            );
            self.shown = Some(shown);
        }
        // Its close button closes only this window (the call goes as the button says, in `window`):
        // ZapFast stays in the tray.
        if ctx.input(|input| input.viewport().close_requested()) && !app.quit_requested {
            app.hide_intent = true;
        }
        let next = self.decide(app, Instant::now());
        if next == Standalone::Stay {
            // Events reach the app between frames: look again soon, as the tray would.
            ctx.request_repaint_after(Duration::from_millis(250));
        } else {
            if self.closing != Some(next) {
                log::info!("[WINDOW] closing the call window: {next:?}");
                self.closing = Some(next);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        next
    }

    /// A window opened at `opened`, for tests that need it to have waited.
    #[cfg(test)]
    pub(crate) fn opened_at(opened: Instant) -> Self {
        Self {
            opened,
            ..Self::default()
        }
    }

    /// Records that the window drew a frame.
    pub fn drawn(&mut self) {
        self.drawn = true;
    }

    fn decide(&self, app: &mut App, now: Instant) -> Standalone {
        if app.quit_requested {
            Standalone::Quit
        } else if app.wants_show {
            app.reopen_window();
            Standalone::Main
        } else if !app.call_in_window() {
            app.hide_intent = true;
            Standalone::Hide
        } else if !self.drawn && now.duration_since(self.opened) >= UNDRAWN_WAIT {
            if self.closing.is_none() {
                log::warn!(
                    "[WINDOW] the call window was not drawn in {UNDRAWN_WAIT:?}; opening the main window"
                );
            }
            // The main window keeps the call then, drawn or not.
            app.call_window_gave_way = true;
            app.wants_show = true;
            app.reopen_window();
            Standalone::Main
        } else {
            Standalone::Stay
        }
    }
}

/// Whether the call window may open. On KDE Plasma under Wayland a new one waits until KWin is
/// ready to move it to its corner as it appears, so it opens there instead of in the middle of the
/// screen and then jumping; after [`PLACE_WAIT`] it opens anyway, and is moved once it is there.
fn placement_ready(app: &App, ctx: Option<&egui::Context>, title: &str) -> bool {
    if crate::keep_above::method() != crate::keep_above::Method::KWin {
        return true;
    }
    let mut state = app.call_window.lock().unwrap_or_else(|p| p.into_inner());
    let placing = state.placing.get_or_insert_with(|| {
        let placed = Arc::new(OnceLock::new());
        let told = Arc::clone(&placed);
        let waker = app.waker();
        crate::keep_above::kwin_await(
            title.to_owned(),
            app.settings.call_window_on_top,
            move |ready| {
                let _ = told.set(ready);
                waker.wake();
            },
        );
        Placing {
            since: Instant::now(),
            placed,
        }
    });
    if placing.placed.get().is_some() {
        return true;
    }
    let waited = placing.since.elapsed();
    if waited >= PLACE_WAIT {
        return true;
    }
    if let Some(ctx) = ctx {
        ctx.request_repaint_after(PLACE_WAIT - waited);
    }
    false
}

/// ZapFast's icon for the call window, made once.
fn window_icon() -> Arc<egui::IconData> {
    static ICON: OnceLock<Arc<egui::IconData>> = OnceLock::new();
    Arc::clone(ICON.get_or_init(|| {
        const SIZE: usize = 128;
        Arc::new(egui::IconData {
            rgba: crate::util::app_icon_rgba(SIZE),
            width: SIZE as u32,
            height: SIZE as u32,
        })
    }))
}

/// A voice call window's height with nothing under the status and the device pickers closed;
/// a group call's ringing prompt also shows who is on it already.
fn base_height(faces: bool) -> f32 {
    let faces = if faces { FACE + 10.0 } else { 0.0 };
    MARGIN + AVATAR + 8.0 + 24.0 + 2.0 + 18.0 + 8.0 + BARS_HEIGHT + faces + 14.0 + CONTROL + MARGIN
}

/// The lines under the status: devices that went away, the peer's audio, and the peer asking for
/// video.
fn warnings(call: &CallUpdate) -> usize {
    call.lost_devices.len()
        + usize::from(call.peer_audio.is_some())
        + usize::from(call.video_requested)
}

/// How many device pickers the window shows: the camera's too on a video call.
fn pickers(call: &CallUpdate) -> usize {
    if call.video { 3 } else { 2 }
}

/// A voice call window's height for what it shows now.
fn voice_height(view: &CallView, devices_open: bool) -> f32 {
    let faces = view.call.phase == CallPhase::Incoming && view.people.iter().any(|p| p.joined);
    let devices = if devices_open {
        pickers(&view.call) as f32 * (PICKER_HEIGHT + 6.0) + 8.0
    } else {
        0.0
    };
    base_height(faces) + warnings(&view.call) as f32 * WARNING_LINE + devices
}

/// Closes the call window as its close button does. Closing the window is hanging up: the call has
/// nowhere else to be shown. A call that is still ringing only stops being shown here, and goes
/// back to the card.
pub(crate) fn close_window(state: &mut CallWindow) {
    let Some(view) = state.view.take() else {
        return;
    };
    state
        .requests
        .push(if view.call.phase == CallPhase::Incoming {
            CallRequest::Hide
        } else if view.call.phase.is_live() {
            // Hanging up also lets the call go.
            CallRequest::Action(Action::HangupCall)
        } else {
            CallRequest::Dismiss
        });
}

/// One frame of the call window, in its own viewport. `embedded` is a window drawn inside the main
/// one, where there is no viewport of its own to close, size or place (egui does this where a
/// backend has only one window, as the tests do).
fn window(
    ui: &mut egui::Ui,
    shared: &SharedCallWindow,
    waker: &crate::backend::Waker,
    monitor: Option<Rect>,
    embedded: bool,
) {
    let ctx = ui.ctx().clone();
    let (view, frames, mut devices_open, levels, mut textures) = {
        let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
        let Some(view) = state.view.clone() else {
            // The call is gone and the main window has not taken the viewport down yet (it may be
            // minimized and not drawing), so the window gets out of the way on its own.
            if !embedded {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            return;
        };
        if !embedded && ctx.input(|input| input.viewport().close_requested()) {
            close_window(&mut state);
            drop(state);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            waker.wake();
            return;
        }
        if !embedded {
            keep_above(&ctx, &mut state, &view);
        }
        if view.call.phase == CallPhase::Active {
            let now = Instant::now();
            if state
                .sampled
                .is_none_or(|sampled| now.duration_since(sampled) >= LEVEL_STEP)
            {
                state.sampled = Some(now);
                state.levels.push_back(crate::call_audio::peer_level());
                while state.levels.len() > BARS / 2 + 1 {
                    state.levels.pop_front();
                }
            }
        } else {
            state.levels.clear();
        }
        if !embedded {
            fit_window(&ctx, &mut state, &view, monitor);
        }
        (
            view,
            state.frames.clone(),
            state.devices_open,
            state.levels.clone(),
            std::mem::take(&mut state.textures),
        )
    };
    let mut requests = Vec::new();
    {
        let mut body = Body {
            view: &view,
            frames: &frames,
            textures: &mut textures,
            levels: &levels,
            devices_open: &mut devices_open,
            requests: &mut requests,
        };
        match Shape::of(&view) {
            Shape::Voice => body.voice(ui),
            Shape::Video => body.video(ui),
            Shape::Group => body.group(ui),
        }
    }
    {
        let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
        state.devices_open = devices_open;
        state.textures = textures;
        if !requests.is_empty() {
            state.requests.extend(requests);
            waker.wake();
        }
    }
    if view.call.phase == CallPhase::Active {
        // The bars move with the peer's voice, and the duration with the clock.
        ctx.request_repaint_after(LEVEL_STEP);
    } else if view.call.phase.is_live() {
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

/// Gives the window the size its shape calls for. A voice call's window is held to the height of
/// what it shows; a video or group call's grows to its own size once, when it becomes one, and is
/// the user's to resize from there. A window that grew is moved back into its corner.
fn fit_window(ctx: &egui::Context, state: &mut CallWindow, view: &CallView, monitor: Option<Rect>) {
    let shape = Shape::of(view);
    let changed = state.shape != Some(shape);
    let first = state.shape.is_none();
    state.shape = Some(shape);
    let size = match shape {
        Shape::Voice => {
            let height = voice_height(view, state.devices_open);
            if !changed && (height - state.height).abs() <= 0.5 {
                return;
            }
            state.height = height;
            let size = vec2(WINDOW_WIDTH, height);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            size
        }
        Shape::Video | Shape::Group => {
            if !changed {
                return;
            }
            let (size, min) = if shape == Shape::Video {
                (VIDEO_SIZE, VIDEO_MIN)
            } else {
                (GROUP_SIZE, GROUP_MIN)
            };
            // No taller than the screen leaves room for.
            let size = monitor.map_or(size, |monitor| {
                size.min(monitor.size() - Vec2::splat(4.0 * crate::keep_above::EDGE))
                    .max(min)
            });
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(NO_LIMIT));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            size
        }
    };
    if first || !changed {
        return;
    }
    // A window that changed size keeps to its corner rather than growing off the screen's edge.
    match crate::keep_above::method() {
        crate::keep_above::Method::WindowLevel => {
            if let Some(monitor) = monitor {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(corner(
                    monitor, size.x,
                )));
            }
        }
        crate::keep_above::Method::KWin => crate::keep_above::kwin_arrange(
            view.peer.clone(),
            true,
            view.on_top,
            Duration::from_millis(150),
        ),
        crate::keep_above::Method::Unsupported => {}
    }
}

/// Asks the desktop to keep the window above the others, or to stop, when that changed.
fn keep_above(ctx: &egui::Context, state: &mut CallWindow, view: &CallView) {
    use crate::keep_above::Method;
    if state.applied_on_top == Some(view.on_top) {
        return;
    }
    let new = state.applied_on_top.is_none();
    state.applied_on_top = Some(view.on_top);
    // KWin placed the new window and kept it above as it appeared.
    let placed = state
        .placing
        .as_ref()
        .is_some_and(|placing| placing.placed.get() == Some(&true));
    if new && placed {
        return;
    }
    match crate::keep_above::method() {
        Method::WindowLevel => {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if view.on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }))
        }
        // A new window is moved to its corner as well, once it has had a moment to be mapped and
        // KWin can find it.
        Method::KWin => crate::keep_above::kwin_arrange(
            view.peer.clone(),
            new,
            view.on_top,
            Duration::from_millis(if new { 700 } else { 0 }),
        ),
        Method::Unsupported => {}
    }
}

/// One frame of the call window's contents, with what it draws from and where its requests go.
struct Body<'a> {
    view: &'a CallView,
    frames: &'a Frames,
    textures: &'a mut Textures,
    levels: &'a VecDeque<f32>,
    devices_open: &'a mut bool,
    requests: &'a mut Vec<CallRequest>,
}

impl Body<'_> {
    fn push(&mut self, action: Action) {
        self.requests.push(CallRequest::Action(action));
    }

    /// A voice call, or any call while it rings: who, how far along, how loud the peer is, and the
    /// controls.
    fn voice(&mut self, ui: &mut egui::Ui) {
        let view = self.view;
        let CallView {
            call,
            peer,
            picture,
            palette,
            locale,
            ..
        } = view;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, surface_color(palette));
        self.corner_buttons(ui, rect, false);
        let inner = rect.shrink(MARGIN);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if call.group.as_ref().is_some_and(|group| group.link) {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(AVATAR), Sense::hover());
                    paint_link_avatar(ui, palette, rect);
                } else {
                    widgets::avatar(ui, palette, peer, &call.chat, AVATAR, picture.as_deref());
                }
                ui.add_space(8.0);
                theme::text(ui, peer, theme::bold(18.0), palette.text);
                ui.add_space(2.0);
                let (line, color) = if call.phase == CallPhase::Incoming {
                    (
                        incoming_text(*locale, call, view.caller.as_deref()),
                        palette.secondary,
                    )
                } else {
                    (status(*locale, call), status_color(call, palette))
                };
                theme::text(ui, line, theme::medium(13.0), color);
                ui.add_space(8.0);
                if call.phase == CallPhase::Incoming {
                    let joined: Vec<&CallPerson> =
                        view.people.iter().filter(|person| person.joined).collect();
                    if !joined.is_empty() {
                        faces(ui, palette, &joined);
                        ui.add_space(10.0);
                    }
                }
                level_bars(ui, self.levels, palette);
                notes(ui, *locale, call, palette, palette.warning);
                ui.add_space(14.0);
                if call.phase == CallPhase::Incoming {
                    ui.horizontal(|ui| self.ringing_controls(ui));
                    return;
                }
                if *self.devices_open {
                    self.devices(ui);
                    ui.add_space(8.0);
                }
                ui.horizontal(|ui| self.controls(ui));
            },
        );
    }

    /// A one-to-one video call: the other side's picture over the window, our own camera in its
    /// corner, and the controls in a bar along the bottom.
    fn video(&mut self, ui: &mut egui::Ui) {
        let view = self.view;
        let CallView {
            call,
            peer,
            picture,
            palette,
            locale,
            ..
        } = view;
        let rect = ui.max_rect();
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - CONTROL_BAR), rect.max);
        let stage = rect.with_max_y(bar.top());
        ui.painter().rect_filled(bar, 0.0, surface_color(palette));
        let remote = (call.remote_video)
            .then(|| self.frames.remote.clone())
            .flatten();
        if let Some(image) = &remote {
            // A peer's picture is letterboxed in black, as any video is.
            ui.painter().rect_filled(stage, 0.0, Color32::BLACK);
            let texture = call_texture(ui, &mut self.textures.remote, "zapfast-call-remote", image);
            let at = fit_inside(stage, image_size(image));
            egui::Image::new((texture, at.size())).paint_at(ui, at);
            // The name and how long the call has been up, on a plate of their own at the top.
            let name =
                ui.painter()
                    .layout_no_wrap(peer.to_owned(), theme::semibold(14.0), Color32::WHITE);
            let line = ui.painter().layout_no_wrap(
                status(*locale, call),
                theme::medium(12.5),
                Color32::WHITE,
            );
            let inner = vec2(
                name.size().x.max(line.size().x),
                name.size().y + line.size().y,
            );
            let plate = Rect::from_min_size(
                pos2(stage.left() + 10.0, stage.top() + 10.0),
                inner + vec2(20.0, 10.0),
            );
            let plate = plate.intersect(
                stage
                    .shrink2(vec2(10.0, 0.0))
                    .with_max_x(stage.right() - 2.0 * (PIN + 8.0) - 10.0),
            );
            ui.painter()
                .rect_filled(plate, CornerRadius::same(10), VIDEO_SCRIM);
            let clip = ui.painter().with_clip_rect(plate.shrink2(vec2(10.0, 0.0)));
            clip.galley(
                pos2(plate.left() + 10.0, plate.top() + 5.0),
                name.clone(),
                Color32::WHITE,
            );
            clip.galley(
                pos2(plate.left() + 10.0, plate.top() + 5.0 + name.size().y),
                line,
                Color32::WHITE,
            );
            // What moved under the call, under the plate, readable over any picture.
            let lines = video_notes(*locale, call);
            let mut top = plate.bottom() + 6.0;
            for line in lines {
                let galley = ui.painter().layout(
                    line,
                    theme::medium(12.0),
                    Color32::WHITE,
                    stage.width() - 40.0,
                );
                let back = Rect::from_min_size(
                    pos2(stage.left() + 10.0, top),
                    galley.size() + vec2(16.0, 6.0),
                );
                ui.painter()
                    .rect_filled(back, CornerRadius::same(8), VIDEO_SCRIM);
                ui.painter()
                    .galley(back.min + vec2(8.0, 3.0), galley, Color32::WHITE);
                top = back.bottom() + 4.0;
            }
        } else {
            // No picture from the other side yet, or their camera is off: who is on the line, as a
            // voice call shows it.
            ui.painter().rect_filled(stage, 0.0, surface_color(palette));
            let inner = stage.shrink(MARGIN);
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(Layout::top_down(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space((inner.height() * 0.12).min(48.0));
                    let size = (inner.height() * 0.28).clamp(AVATAR, 140.0);
                    widgets::avatar(ui, palette, peer, &call.chat, size, picture.as_deref());
                    ui.add_space(10.0);
                    theme::text(ui, peer, theme::bold(19.0), palette.text);
                    ui.add_space(2.0);
                    theme::text(
                        ui,
                        status(*locale, call),
                        theme::medium(13.0),
                        status_color(call, palette),
                    );
                    if call.phase == CallPhase::Active && !call.remote_video {
                        ui.add_space(4.0);
                        theme::text(
                            ui,
                            gettext(*locale, "Their camera is off"),
                            theme::medium(12.5),
                            palette.secondary,
                        );
                    }
                    notes(ui, *locale, call, palette, palette.warning);
                },
            );
        }
        self.own_preview(ui, stage);
        self.corner_buttons(ui, rect, remote.is_some());
        if *self.devices_open && call.phase.is_live() {
            self.devices_panel(ui, stage);
        }
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(vec2(MARGIN, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| self.controls(ui),
        );
    }

    /// Our own camera, in the bottom corner of the picture at its own shape.
    fn own_preview(&mut self, ui: &mut egui::Ui, stage: Rect) {
        let call = &self.view.call;
        if !(call.video && call.camera_on) {
            return;
        }
        let Some(image) = self.frames.local.clone() else {
            return;
        };
        let bounds = stage.size() * PREVIEW_SHARE;
        let size = fit_inside(Rect::from_min_size(stage.min, bounds), image_size(&image)).size();
        let at = Rect::from_min_size(
            pos2(
                stage.right() - size.x - 10.0,
                stage.bottom() - size.y - 10.0,
            ),
            size,
        );
        let texture = call_texture(ui, &mut self.textures.local, "zapfast-call-local", &image);
        egui::Image::new((texture, size))
            .corner_radius(CornerRadius::same(10))
            .paint_at(ui, at);
        ui.painter().rect_stroke(
            at,
            CornerRadius::same(10),
            egui::Stroke::new(1.0, Color32::from_white_alpha(90)),
            egui::StrokeKind::Outside,
        );
    }

    /// A group call: the group's name and how the call is going on top, a tile for each person on
    /// it, and the controls along the bottom.
    fn group(&mut self, ui: &mut egui::Ui) {
        let view = self.view;
        let CallView {
            call,
            peer,
            palette,
            locale,
            ..
        } = view;
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, surface_color(palette));
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), 52.0))
            .shrink2(vec2(2.0 * (PIN + 8.0) + 8.0, 0.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(header), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                theme::text(ui, peer, theme::bold(17.0), palette.text);
                ui.add_space(2.0);
                theme::text(
                    ui,
                    status(*locale, call),
                    theme::medium(13.0),
                    status_color(call, palette),
                );
            });
        });
        self.corner_buttons(ui, rect, false);
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - CONTROL_BAR), rect.max);
        let mut bottom = bar.top() - 4.0;
        let lines = warnings(call);
        if lines > 0 {
            let height = lines as f32 * WARNING_LINE + 4.0;
            let room = Rect::from_min_max(
                pos2(rect.left() + MARGIN, bottom - height),
                pos2(rect.right() - MARGIN, bottom),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    notes(ui, *locale, call, palette, palette.warning);
                });
            });
            bottom -= height + 4.0;
        }
        let grid = Rect::from_min_max(
            pos2(rect.left() + MARGIN, header.bottom() + 8.0),
            pos2(rect.right() - MARGIN, bottom.max(header.bottom() + 60.0)),
        );
        self.tiles(ui, grid);
        if *self.devices_open && call.phase.is_live() {
            self.devices_panel(ui, rect.with_max_y(bar.top()));
        }
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(vec2(MARGIN, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| self.controls(ui),
        );
    }

    /// The group call's tiles: those in the call first, then ourselves, then those only asked to
    /// it; past the most a grid holds, the last tile counts the rest.
    fn tiles(&mut self, ui: &mut egui::Ui, grid: Rect) {
        let view = self.view;
        let CallView {
            call,
            people,
            me,
            palette,
            locale,
            ..
        } = view;
        let camera = (call.video && call.camera_on)
            .then(|| self.frames.local.clone())
            .flatten();
        let mut tiles: Vec<Tile<'_>> = people
            .iter()
            .filter(|person| person.joined)
            .map(Tile::Person)
            .collect();
        // Our own tile, in a voice call as in a video one: our camera while it runs, else our photo.
        match (camera, me.as_ref()) {
            (Some(image), _) => tiles.push(Tile::Me(image)),
            (None, Some(me)) => tiles.push(Tile::MyPhoto(me)),
            (None, None) => {}
        }
        tiles.extend(
            people
                .iter()
                .filter(|person| !person.joined)
                .map(Tile::Person),
        );
        if tiles.len() > MAX_TILES {
            let rest = tiles.len() - (MAX_TILES - 1);
            tiles.truncate(MAX_TILES - 1);
            tiles.push(Tile::More(rest));
        }
        let rects = tile_grid(tiles.len(), grid, TILE_GAP);
        let you = gettext(*locale, "You").into_owned();
        for (tile, rect) in tiles.into_iter().zip(rects) {
            match tile {
                // A member whose camera is on fills their tile with it; anyone else keeps their
                // photo, so nobody leaves the grid by turning it off.
                Tile::Person(person) => match person
                    .video
                    .then(|| self.frames.members.get(&person.id).cloned())
                    .flatten()
                {
                    Some(image) => {
                        let mut kept = self.textures.members.remove(&person.id);
                        let texture = call_texture(ui, &mut kept, "zapfast-call-member", &image);
                        if let Some(kept) = kept {
                            self.textures.members.insert(person.id.clone(), kept);
                        }
                        video_tile(ui, &person.name, texture, image_size(&image), rect);
                    }
                    None => person_tile(ui, *locale, palette, person, rect),
                },
                Tile::Me(image) => {
                    let texture =
                        call_texture(ui, &mut self.textures.local, "zapfast-call-local", &image);
                    video_tile(ui, &you, texture, image_size(&image), rect);
                    if call.muted {
                        muted_badge(ui, palette, rect);
                    }
                }
                Tile::MyPhoto(me) => {
                    person_tile(ui, *locale, palette, me, rect);
                    if call.muted {
                        muted_badge(ui, palette, rect);
                    }
                }
                Tile::More(rest) => {
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(12), tile_fill(palette));
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("+{rest}"),
                        theme::bold(22.0),
                        palette.secondary,
                    );
                }
            }
        }
        // Members who have left the call keep no texture.
        let shown: Vec<&str> = people.iter().map(|person| person.id.as_str()).collect();
        self.textures
            .members
            .retain(|member, _| shown.contains(&member.as_str()));
    }

    /// The pin that keeps the window above the others and, on a video or group call, the button
    /// that expands the window, in its top-right corner. Over a picture they sit on a dark disc.
    fn corner_buttons(&mut self, ui: &mut egui::Ui, rect: Rect, over_picture: bool) {
        let view = self.view;
        let CallView {
            palette,
            locale,
            on_top,
            ..
        } = view;
        let mut right = rect.right() - 8.0;
        if crate::keep_above::method() != crate::keep_above::Method::Unsupported {
            let pin = Rect::from_min_size(pos2(right - PIN, rect.top() + 8.0), Vec2::splat(PIN));
            right = pin.left() - 8.0;
            let tip = if *on_top {
                gettext(
                    *locale,
                    "The call window stays above other windows. Click to stop.",
                )
            } else {
                gettext(*locale, "Keep the call window above other windows")
            }
            .into_owned();
            let (fill, tint) = match (*on_top, over_picture) {
                (true, _) => (palette.accent, palette.on_accent),
                (false, true) => (VIDEO_SCRIM, Color32::WHITE),
                (false, false) => (palette.surface_active, palette.secondary),
            };
            let response = ui
                .scope_builder(egui::UiBuilder::new().max_rect(pin), |ui| {
                    control(ui, Icon::Pin, PIN, fill, tint, &tip, true)
                })
                .inner;
            if response.clicked() {
                self.requests.push(CallRequest::OnTop(!*on_top));
            }
        }
        if Shape::of(view) == Shape::Voice {
            return;
        }
        let expand = Rect::from_min_size(pos2(right - PIN, rect.top() + 8.0), Vec2::splat(PIN));
        let maximized = ui
            .ctx()
            .input(|input| input.viewport().maximized)
            .unwrap_or(false);
        let (icon, tip) = if maximized {
            (Icon::Minimize, gettext(*locale, "Restore the call window"))
        } else {
            (Icon::Maximize, gettext(*locale, "Expand the call window"))
        };
        let (fill, tint) = if over_picture {
            (VIDEO_SCRIM, Color32::WHITE)
        } else {
            (palette.surface_active, palette.secondary)
        };
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(expand), |ui| {
                control(ui, icon, PIN, fill, tint, &tip, true)
            })
            .inner;
        if response.clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
    }

    /// The call window's round buttons: mute, the camera, switching cameras, the devices, and hang
    /// up, centred in the row.
    fn controls(&mut self, ui: &mut egui::Ui) {
        let view = self.view;
        let CallView {
            call,
            palette,
            locale,
            devices,
            ..
        } = view;
        let locale = *locale;
        let connected = call.phase.is_connected();
        let live = call.phase.is_live();
        // A group call takes video only when it began as a video call: the library refuses to
        // turn a group's voice call into one, so no button offers it.
        let camera = call.group.is_none() || call.video;
        let switch = call.video && call.camera_on && devices.cameras.len() > 1;
        let count = 3 + usize::from(camera) + usize::from(switch);
        let n = count as f32;
        let available = ui.available_width();
        let diameter = CONTROL.min((available - (n - 1.0) * CONTROL_GAP) / n);
        let spacing = ((available - n * diameter) / (n - 1.0)).clamp(CONTROL_GAP, 22.0);
        let width = n * diameter + (n - 1.0) * spacing;
        ui.spacing_mut().item_spacing.x = spacing;
        ui.add_space(((available - width) / 2.0).max(0.0));

        // The engine's own mute, which also tells the peer.
        let (icon, tip, fill, tint) = if call.muted {
            (
                Icon::MicOff,
                gettext(locale, "Unmute"),
                palette.danger,
                Color32::WHITE,
            )
        } else {
            (
                Icon::Mic,
                gettext(locale, "Mute"),
                palette.surface_active,
                palette.text,
            )
        };
        if control(ui, icon, diameter, fill, tint, &tip, connected).clicked() {
            self.push(Action::SetCallMuted(!call.muted));
        }

        // The camera: starts video on a voice call, and stops or starts sending our picture on a
        // video call. On a voice call the peer asked to turn into a video call, it answers them.
        if camera {
            let (icon, tip, fill, tint) = if !call.video {
                (
                    Icon::Video,
                    gettext(locale, "Start video"),
                    if call.video_requested {
                        palette.accent
                    } else {
                        palette.surface_active
                    },
                    if call.video_requested {
                        palette.on_accent
                    } else {
                        palette.text
                    },
                )
            } else if call.camera_on {
                (
                    Icon::Video,
                    gettext(locale, "Turn the camera off"),
                    palette.surface_active,
                    palette.text,
                )
            } else {
                (
                    Icon::VideoOff,
                    gettext(locale, "Turn the camera on"),
                    palette.danger,
                    Color32::WHITE,
                )
            };
            if control(ui, icon, diameter, fill, tint, &tip, connected).clicked() {
                self.push(Action::SetCallCamera(!call.camera_on));
            }
        }

        // The next camera the machine has, after the one in use.
        if switch {
            let tip = gettext(locale, "Switch camera");
            if control(
                ui,
                Icon::SwitchCamera,
                diameter,
                palette.surface_active,
                palette.text,
                &tip,
                connected,
            )
            .clicked()
            {
                self.push(Action::SetCallCameraDevice(next_camera(
                    devices,
                    call.camera.as_deref(),
                )));
            }
        }

        // The speaker button shows and hides the device pickers.
        let tip = gettext(locale, "Speaker and devices").into_owned();
        let (fill, tint) = if *self.devices_open {
            (palette.accent, palette.on_accent)
        } else {
            (palette.surface_active, palette.text)
        };
        if control(ui, Icon::Volume2, diameter, fill, tint, &tip, live).clicked() {
            *self.devices_open = !*self.devices_open;
        }

        let tip = gettext(locale, "Hang up").into_owned();
        if control(
            ui,
            Icon::PhoneOff,
            diameter,
            palette.danger,
            Color32::WHITE,
            &tip,
            live,
        )
        .clicked()
        {
            self.push(Action::HangupCall);
        }
    }

    /// Decline and Accept, for a call that is still ringing, so the window can answer it without
    /// the main window. Anonymous, the window only says who is calling.
    fn ringing_controls(&mut self, ui: &mut egui::Ui) {
        let CallView {
            call,
            palette,
            locale,
            answerable,
            ..
        } = self.view;
        if !answerable {
            ui.vertical_centered(|ui| {
                theme::text(
                    ui,
                    gettext(*locale, "Anonymous mode: the call is only logged"),
                    theme::regular(13.0),
                    palette.secondary,
                );
            });
            return;
        }
        let count = if call.video { 3.0 } else { 2.0 };
        let spacing = if call.video { 24.0 } else { 44.0 };
        let width = count * CONTROL + (count - 1.0) * spacing;
        ui.spacing_mut().item_spacing.x = spacing;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        let mut actions = Vec::new();
        answers(ui, palette, *locale, call.video, CONTROL, &mut actions);
        for action in actions {
            self.push(action);
        }
    }

    /// The device pickers, one above the other. Each one rebinds a live stream, so what it shows is
    /// what is in use.
    fn devices(&mut self, ui: &mut egui::Ui) {
        let CallView {
            call,
            locale,
            devices,
            ..
        } = self.view;
        let locale = *locale;
        let default_label = gettext(locale, "Default device").into_owned();
        let list = |devices: Vec<(&String, &String)>| -> Vec<(String, String)> {
            devices
                .into_iter()
                .map(|(id, label)| (id.clone(), label.clone()))
                .collect()
        };
        let microphones = list(
            devices
                .microphones
                .iter()
                .map(|device| (&device.id, &device.label))
                .collect(),
        );
        let speakers = list(
            devices
                .speakers
                .iter()
                .map(|device| (&device.id, &device.label))
                .collect(),
        );
        let cameras = list(
            devices
                .cameras
                .iter()
                .map(|device| (&device.id, &device.label))
                .collect(),
        );
        let connected = call.phase.is_connected();
        let width = PICKER_WIDTH.min(ui.available_width());
        let size = vec2(width, PICKER_HEIGHT);
        let mut actions = Vec::new();
        ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
            Picker {
                salt: "call-microphone",
                icon: Icon::Mic,
                title: &gettext(locale, "Microphone"),
                default_label: &default_label,
                current: call.microphone.as_deref(),
                devices: &microphones,
                enabled: connected,
                width,
            }
            .show(ui, &mut actions, Action::SetCallMicrophone);
        });
        ui.add_space(6.0);
        ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
            Picker {
                salt: "call-speaker",
                icon: Icon::Volume2,
                title: &gettext(locale, "Speaker"),
                default_label: &default_label,
                current: call.speaker.as_deref(),
                devices: &speakers,
                enabled: connected,
                width,
            }
            .show(ui, &mut actions, Action::SetCallSpeaker);
        });
        if call.video {
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                Picker {
                    salt: "call-camera",
                    icon: Icon::Video,
                    title: &gettext(locale, "Camera"),
                    default_label: &gettext(locale, "First camera"),
                    current: call.camera.as_deref(),
                    devices: &cameras,
                    enabled: connected,
                    width,
                }
                .show(ui, &mut actions, Action::SetCallCameraDevice);
            });
        }
        for action in actions {
            self.push(action);
        }
    }

    /// The device pickers on a panel of their own along the bottom of `area`, over a video call's
    /// picture or a group call's tiles.
    fn devices_panel(&mut self, ui: &mut egui::Ui, area: Rect) {
        let palette = self.view.palette;
        let count = pickers(&self.view.call) as f32;
        let height = count * (PICKER_HEIGHT + 6.0) + 2.0 * 10.0;
        let width = (PICKER_WIDTH + 2.0 * 14.0).min(area.width() - 16.0);
        let panel = Rect::from_min_size(
            pos2(area.center().x - width / 2.0, area.bottom() - height - 8.0),
            vec2(width, height),
        );
        ui.painter()
            .rect_filled(panel, CornerRadius::same(12), surface_color(&palette));
        ui.painter().rect_stroke(
            panel,
            CornerRadius::same(12),
            egui::Stroke::new(1.0, palette.outline),
            egui::StrokeKind::Inside,
        );
        // Clicks on the panel stay on it.
        ui.interact(panel, egui::Id::new(WINDOW).with("devices"), Sense::click());
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(panel.shrink2(vec2(14.0, 10.0)))
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.devices(ui);
            },
        );
    }
}

/// The camera after `current` in the machine's list, round to the first; `None` (the first camera)
/// when there is nothing to switch between.
fn next_camera(devices: &DeviceList, current: Option<&str>) -> Option<String> {
    let cameras = &devices.cameras;
    if cameras.is_empty() {
        return None;
    }
    let at = current
        .and_then(|current| cameras.iter().position(|camera| camera.id == current))
        .unwrap_or(0);
    Some(cameras[(at + 1) % cameras.len()].id.clone())
}

/// The peer's loudness as bars that spread from the middle: the newest sample in the centre, older
/// ones outwards, so the row moves like an equalizer while the peer talks and lies flat while they
/// are quiet.
fn level_bars(ui: &mut egui::Ui, levels: &VecDeque<f32>, palette: &Palette) {
    const WIDTH: f32 = 3.0;
    const GAP: f32 = 3.0;
    let total = BARS as f32 * (WIDTH + GAP) - GAP;
    let (rect, _) = ui.allocate_exact_size(vec2(total, BARS_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let middle = BARS / 2;
    for bar in 0..BARS {
        let age = bar.abs_diff(middle);
        let level = levels
            .len()
            .checked_sub(1 + age)
            .and_then(|index| levels.get(index))
            .copied()
            .unwrap_or(0.0);
        let height = (WIDTH + level * (BARS_HEIGHT - WIDTH)).min(BARS_HEIGHT);
        let x = rect.left() + bar as f32 * (WIDTH + GAP);
        let bar_rect =
            Rect::from_center_size(pos2(x + WIDTH / 2.0, rect.center().y), vec2(WIDTH, height));
        let color = if level > 0.05 {
            palette.accent
        } else {
            palette.secondary.gamma_multiply(0.5)
        };
        ui.painter()
            .rect_filled(bar_rect, CornerRadius::same(2), color);
    }
}

/// The backdrop: a window colour.
fn surface_color(palette: &Palette) -> Color32 {
    if palette.dark {
        Color32::from_rgb(18, 21, 24)
    } else {
        palette.window
    }
}

/// The line under the peer's name, from the backend's phase and nothing else.
pub(crate) fn status(locale: Locale, call: &CallUpdate) -> String {
    match call.phase {
        // The button was pressed and the offer is still on its way out.
        CallPhase::Placing => gettext(locale, "Connecting…").into_owned(),
        // The offer is out; no device of the peer's has said it rings yet.
        CallPhase::Dialing => gettext(locale, "Calling…").into_owned(),
        // The peer's phone rings: its preaccept arrived.
        CallPhase::Ringing => gettext(locale, "Ringing…").into_owned(),
        // Answered, and the media is still coming up.
        CallPhase::Connecting | CallPhase::Accepted => gettext(locale, "Connecting…").into_owned(),
        // The length runs from when the media came up, not from the button.
        CallPhase::Active => call
            .started
            .map(elapsed)
            .unwrap_or_else(|| gettext(locale, "Connecting…").into_owned()),
        CallPhase::Ended | CallPhase::Failed => call
            .outcome
            .map(|outcome| outcome_text(locale, outcome, call.phase))
            .unwrap_or_else(|| gettext(locale, "Call ended").into_owned()),
        CallPhase::Incoming => incoming_text(locale, call, None),
    }
}

/// How a call that is over reads.
fn outcome_text(locale: Locale, outcome: CallOutcome, phase: CallPhase) -> String {
    match outcome {
        CallOutcome::Answered => gettext(locale, "Call ended").into_owned(),
        CallOutcome::Missed => gettext(locale, "Missed call").into_owned(),
        CallOutcome::Declined => gettext(locale, "Declined").into_owned(),
        CallOutcome::Busy => gettext(locale, "Busy").into_owned(),
        CallOutcome::Failed => gettext(locale, "Could not connect").into_owned(),
        CallOutcome::NoAnswer => gettext(locale, "No answer").into_owned(),
        CallOutcome::ConnectionLost if phase == CallPhase::Ended => {
            gettext(locale, "Connection lost").into_owned()
        }
        CallOutcome::ConnectionLost => gettext(locale, "Could not connect").into_owned(),
        CallOutcome::AnsweredElsewhere => {
            gettext(locale, "Answered on another device").into_owned()
        }
        CallOutcome::DeclinedElsewhere => {
            gettext(locale, "Declined on another device").into_owned()
        }
    }
}

fn status_color(call: &CallUpdate, palette: &Palette) -> Color32 {
    match call.phase {
        CallPhase::Failed => palette.danger,
        CallPhase::Ended => palette.secondary,
        _ => palette.accent,
    }
}

/// The lines that say what moved under the call: a device that went away, the peer's audio not
/// arriving, and the peer asking to turn the call into a video call.
fn note_lines(locale: Locale, call: &CallUpdate) -> Vec<String> {
    let mut lines: Vec<String> = call
        .lost_devices
        .iter()
        .map(|lost| {
            let template = match lost.kind {
                crate::calls::DeviceKind::Microphone => gettext(
                    locale,
                    "Microphone “{name}” is not available; using the system default",
                ),
                crate::calls::DeviceKind::Speaker => gettext(
                    locale,
                    "Speaker “{name}” is not available; using the system default",
                ),
                crate::calls::DeviceKind::Camera => {
                    gettext(locale, "Camera “{name}” is not available")
                }
            };
            template.replace("{name}", &lost.name)
        })
        .collect();
    if let Some(audio) = call.peer_audio {
        lines.push(peer_audio_text(locale, audio));
    }
    if call.video_requested {
        // The camera button answers it.
        lines.push(
            gettext(
                locale,
                "Wants to switch to video: turn on your camera to answer",
            )
            .into_owned(),
        );
    }
    lines
}

/// The notes over a video call's picture: the same, and the peer's camera being off.
fn video_notes(locale: Locale, call: &CallUpdate) -> Vec<String> {
    note_lines(locale, call)
}

/// The notes under the status, in `color`.
fn notes(ui: &mut egui::Ui, locale: Locale, call: &CallUpdate, palette: &Palette, color: Color32) {
    let lines = note_lines(locale, call);
    if lines.is_empty() {
        return;
    }
    ui.add_space(4.0);
    for (index, line) in lines.into_iter().enumerate() {
        // The request for video is an invitation, not a warning.
        let color = if call.video_requested && index + 1 == warnings(call) {
            palette.accent
        } else {
            color
        };
        theme::text(ui, line, theme::medium(12.5), color);
    }
}

/// What the engine says about the peer's audio, in the reader's language.
fn peer_audio_text(locale: Locale, audio: PeerAudio) -> String {
    match audio {
        PeerAudio::Stalled => gettext(locale, "No audio is arriving from this call").into_owned(),
        PeerAudio::Silent(SilenceReason::NoDecoder) => {
            gettext(locale, "This call's audio cannot be decoded").into_owned()
        }
        PeerAudio::Silent(
            SilenceReason::AuthenticationFailing
            | SilenceReason::UnexpectedPayloadType
            | SilenceReason::CodecRejectingFrames
            | SilenceReason::CodecFlapping
            | SilenceReason::Unknown,
        ) => gettext(locale, "The other side's audio is not getting through").into_owned(),
    }
}

/// `m:ss`, or `h:mm:ss` past an hour.
fn elapsed(started: Instant) -> String {
    crate::util::call_duration(started.elapsed().as_secs())
}

/// One round control. A disabled control still reads as a control rather than disappearing.
fn control(
    ui: &mut egui::Ui,
    icon: Icon,
    diameter: f32,
    fill: Color32,
    tint: Color32,
    tooltip: &str,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), sense);
    if enabled {
        theme::reveal_focus(&response);
        theme::focus_outline(ui, response.id, rect, diameter / 2.0);
    }
    if ui.is_rect_visible(rect) {
        let hovered = enabled && (response.hovered() || response.has_focus());
        let fill = if !enabled {
            fill.gamma_multiply(0.6)
        } else if hovered {
            theme::blend(fill, tint, 0.12)
        } else {
            fill
        };
        let tint = if enabled {
            tint
        } else {
            tint.gamma_multiply(0.6)
        };
        ui.painter()
            .circle_filled(rect.center(), diameter / 2.0, fill);
        theme::paint_icon(ui, icon, rect, diameter * 0.42, tint);
    }
    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    // A painted control is invisible to a screen reader unless it says what it is; the shared icon
    // buttons register the same way, and a disabled one is reported as disabled rather than missing.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));
    response.on_hover_text(tooltip)
}

/// One device picker: a title with its icon, and a list of what the machine really has.
struct Picker<'a> {
    salt: &'a str,
    icon: Icon,
    title: &'a str,
    default_label: &'a str,
    current: Option<&'a str>,
    devices: &'a [(String, String)],
    enabled: bool,
    width: f32,
}

impl Picker<'_> {
    fn show(
        &self,
        ui: &mut egui::Ui,
        actions: &mut Vec<Action>,
        action: impl Fn(Option<String>) -> Action,
    ) {
        let label = self
            .current
            .and_then(|id| {
                self.devices
                    .iter()
                    .find(|(known, _)| known == id)
                    .map(|(_, label)| label.clone())
            })
            .or_else(|| self.current.map(str::to_owned))
            .unwrap_or_else(|| self.default_label.to_owned());
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                theme::icon(ui, self.icon, 13.0, ui.visuals().weak_text_color());
                theme::text(
                    ui,
                    self.title,
                    theme::medium(11.5),
                    ui.visuals().weak_text_color(),
                );
            });
            ui.add_enabled_ui(self.enabled, |ui| {
                egui::ComboBox::from_id_salt(self.salt)
                    .selected_text(label)
                    .width(self.width)
                    .truncate()
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.current.is_none(), self.default_label)
                            .clicked()
                        {
                            actions.push(action(None));
                        }
                        for (id, name) in self.devices {
                            if ui
                                .selectable_label(self.current == Some(id.as_str()), name)
                                .clicked()
                            {
                                actions.push(action(Some(id.clone())));
                            }
                        }
                    });
            });
        });
    }
}

/// A row of the pictures of the people on a group call, overlapping as WhatsApp draws them, and
/// "+N" for those past the first few.
fn faces(ui: &mut egui::Ui, palette: &Palette, people: &[&CallPerson]) {
    let shown = people.len().min(FACES);
    let extra = people.len() - shown;
    let step = FACE * 0.72;
    let count = shown + usize::from(extra > 0);
    let width = FACE + step * (count.saturating_sub(1)) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, FACE), Sense::hover());
    let ring = surface_color(palette);
    for (index, person) in people.iter().take(shown).enumerate() {
        let at = Rect::from_min_size(
            pos2(rect.left() + step * index as f32, rect.top()),
            Vec2::splat(FACE),
        );
        // A ring of the window's colour keeps overlapping faces apart.
        ui.painter()
            .circle_filled(at.center(), FACE / 2.0 + 2.0, ring);
        widgets::paint_avatar(
            ui,
            palette,
            at,
            &person.name,
            &person.id,
            person.picture.as_deref(),
        );
    }
    if extra > 0 {
        let at = Rect::from_min_size(
            pos2(rect.left() + step * shown as f32, rect.top()),
            Vec2::splat(FACE),
        );
        ui.painter()
            .circle_filled(at.center(), FACE / 2.0 + 2.0, ring);
        ui.painter()
            .circle_filled(at.center(), FACE / 2.0, palette.surface_active);
        ui.painter().text(
            at.center(),
            egui::Align2::CENTER_CENTER,
            format!("+{extra}"),
            theme::semibold(12.0),
            palette.text,
        );
    }
}

/// Lays out `count` tiles of [`TILE_ASPECT`] in `area`, as large as they fit: the column count
/// that leaves the biggest tile wins, the grid sits in the middle, and a short last row is centred
/// under the others.
pub(crate) fn tile_grid(count: usize, area: Rect, gap: f32) -> Vec<Rect> {
    if count == 0 || area.width() <= 0.0 || area.height() <= 0.0 {
        return Vec::new();
    }
    let mut best: Option<(usize, Vec2)> = None;
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        let cell = vec2(
            (area.width() - gap * (columns - 1) as f32) / columns as f32,
            (area.height() - gap * (rows - 1) as f32) / rows as f32,
        );
        if cell.x <= 0.0 || cell.y <= 0.0 {
            continue;
        }
        let tile = if cell.x / cell.y > TILE_ASPECT {
            vec2(cell.y * TILE_ASPECT, cell.y)
        } else {
            vec2(cell.x, cell.x / TILE_ASPECT)
        };
        if best.is_none_or(|(_, size)| tile.x * tile.y > size.x * size.y) {
            best = Some((columns, tile));
        }
    }
    let Some((columns, tile)) = best else {
        return Vec::new();
    };
    let rows = count.div_ceil(columns);
    let height = rows as f32 * tile.y + gap * (rows - 1) as f32;
    let top = area.center().y - height / 2.0;
    (0..count)
        .map(|index| {
            let (row, column) = (index / columns, index % columns);
            let in_row = if row == rows - 1 {
                count - row * columns
            } else {
                columns
            };
            let width = in_row as f32 * tile.x + gap * (in_row - 1) as f32;
            let left = area.center().x - width / 2.0;
            Rect::from_min_size(
                pos2(
                    left + column as f32 * (tile.x + gap),
                    top + row as f32 * (tile.y + gap),
                ),
                tile,
            )
        })
        .collect()
}

/// One tile of a group call's grid.
enum Tile<'a> {
    Person(&'a CallPerson),
    /// Our own camera's picture.
    Me(Arc<egui::ColorImage>),
    /// Ourselves while our camera is off, or on a voice call.
    MyPhoto(&'a CallPerson),
    /// How many more are on the call than the grid holds.
    More(usize),
}

/// The sign a muted microphone wears in the corner of our own tile: only ours, since the library
/// says nothing of anyone else's.
fn muted_badge(ui: &egui::Ui, palette: &Palette, tile: Rect) {
    let size = 26.0;
    let at = Rect::from_min_size(
        pos2(tile.right() - size - 8.0, tile.top() + 8.0),
        Vec2::splat(size),
    );
    ui.painter()
        .circle_filled(at.center(), size / 2.0, palette.danger);
    theme::paint_icon(ui, Icon::MicOff, at, 14.0, Color32::WHITE);
}

/// A call link's picture: a link on a plain disc, where a chat would have its photo.
pub(crate) fn paint_link_avatar(ui: &egui::Ui, palette: &Palette, rect: Rect) {
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, palette.surface_active);
    theme::paint_icon(ui, Icon::Link, rect, rect.width() * 0.45, palette.text);
}

/// A tile's ground: a shade off the window in either theme.
fn tile_fill(palette: &Palette) -> Color32 {
    theme::blend(
        surface_color(palette),
        palette.text,
        if palette.dark { 0.08 } else { 0.06 },
    )
}

/// A member's tile while their camera is on: the picture fills it, cropped to its shape as
/// WhatsApp's grid does, with their name on a plate in the corner.
fn video_tile(ui: &mut egui::Ui, name: &str, texture: egui::TextureId, size: Vec2, rect: Rect) {
    egui::Image::new((texture, rect.size()))
        .uv(cover_uv(size, rect.size()))
        .corner_radius(CornerRadius::same(12))
        .paint_at(ui, rect);
    let label = ui
        .painter()
        .layout_no_wrap(name.to_owned(), theme::semibold(12.5), Color32::WHITE);
    let width = label.size().x.min(rect.width() - 32.0).max(0.0);
    let plate = Rect::from_min_size(
        pos2(
            rect.left() + 8.0,
            rect.bottom() - 8.0 - label.size().y - 6.0,
        ),
        vec2(width + 14.0, label.size().y + 6.0),
    );
    ui.painter()
        .rect_filled(plate, CornerRadius::same(7), VIDEO_SCRIM);
    let clip = ui.painter().with_clip_rect(plate.shrink2(vec2(7.0, 0.0)));
    clip.galley(
        pos2(plate.left() + 7.0, plate.top() + 3.0),
        label,
        Color32::WHITE,
    );
}

/// The part of a picture of `image` size that covers a tile of `tile` size: the middle of it, cut
/// down on whichever side is too long.
fn cover_uv(image: Vec2, tile: Vec2) -> Rect {
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if image.x <= 0.0 || image.y <= 0.0 || tile.x <= 0.0 || tile.y <= 0.0 {
        return full;
    }
    let (picture, frame) = (image.x / image.y, tile.x / tile.y);
    if picture > frame {
        let keep = frame / picture;
        Rect::from_min_max(pos2((1.0 - keep) / 2.0, 0.0), pos2((1.0 + keep) / 2.0, 1.0))
    } else {
        let keep = picture / frame;
        Rect::from_min_max(pos2(0.0, (1.0 - keep) / 2.0), pos2(1.0, (1.0 + keep) / 2.0))
    }
}

/// One person's tile on a group call: their picture and name, dimmed while they are only asked to
/// the call rather than in it.
fn person_tile(
    ui: &mut egui::Ui,
    locale: Locale,
    palette: &Palette,
    person: &CallPerson,
    rect: Rect,
) {
    ui.painter()
        .rect_filled(rect, CornerRadius::same(12), tile_fill(palette));
    let label_height = if person.joined { 20.0 } else { 36.0 };
    let size = (rect.height() - label_height - 20.0)
        .min(rect.width() * 0.5)
        .clamp(24.0, 110.0);
    let block = size + 6.0 + label_height;
    let top = rect.center().y - block / 2.0;
    let face = Rect::from_center_size(pos2(rect.center().x, top + size / 2.0), Vec2::splat(size));
    widgets::paint_avatar(
        ui,
        palette,
        face,
        &person.name,
        &person.id,
        person.picture.as_deref(),
    );
    if !person.joined {
        // Asked, not in yet: the face waits behind a veil.
        ui.painter().circle_filled(
            face.center(),
            size / 2.0,
            tile_fill(palette).gamma_multiply(0.55),
        );
    }
    let label = Rect::from_min_size(
        pos2(rect.left() + 8.0, face.bottom() + 6.0),
        vec2(rect.width() - 16.0, label_height),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(label), |ui| {
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let color = if person.joined {
                palette.text
            } else {
                palette.secondary
            };
            theme::text(ui, &person.name, theme::semibold(13.0), color);
            if !person.joined {
                theme::text(
                    ui,
                    gettext(locale, "Invited"),
                    theme::medium(11.5),
                    palette.dim,
                );
            }
        });
    });
}

/// A picture's size as a vector.
fn image_size(image: &egui::ColorImage) -> Vec2 {
    vec2(image.width() as f32, image.height() as f32)
}

/// The largest rect with `size`'s shape that fits inside `area`, centred.
fn fit_inside(area: Rect, size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return area;
    }
    let scale = (area.width() / size.x).min(area.height() / size.y);
    Rect::from_center_size(area.center(), size * scale)
}

/// Draws `image` through the kept texture, uploaded only when a new frame arrived and then in
/// place: the window also repaints for its timer, and a texture per repaint would churn GPU memory.
fn call_texture(
    ui: &egui::Ui,
    kept: &mut Option<CallTexture>,
    name: &str,
    image: &Arc<egui::ColorImage>,
) -> egui::TextureId {
    match kept {
        Some(kept) if Arc::ptr_eq(&kept.frame, image) => kept.texture.id(),
        Some(kept) => {
            kept.texture.set(
                egui::ImageData::from((**image).clone()),
                TextureOptions::LINEAR,
            );
            kept.frame = Arc::clone(image);
            kept.texture.id()
        }
        None => {
            let texture = ui
                .ctx()
                .load_texture(name, (**image).clone(), TextureOptions::LINEAR);
            let id = texture.id();
            *kept = Some(CallTexture {
                texture,
                frame: Arc::clone(image),
            });
            id
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_fill_the_room_without_overlapping() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 600.0));
        for count in 1..=MAX_TILES {
            let tiles = tile_grid(count, area, TILE_GAP);
            assert_eq!(tiles.len(), count);
            for (index, tile) in tiles.iter().enumerate() {
                assert!(
                    area.expand(0.5).contains_rect(*tile),
                    "{count} tiles: {tile:?} leaves the room"
                );
                assert!((tile.width() / tile.height() - TILE_ASPECT).abs() < 0.01);
                for other in &tiles[index + 1..] {
                    assert!(
                        !tile.shrink(0.5).intersects(other.shrink(0.5)),
                        "{count} tiles overlap"
                    );
                }
            }
        }
        // Two people on a wide window sit side by side, as large as fits.
        let two = tile_grid(2, area, TILE_GAP);
        assert!((two[0].center().y - two[1].center().y).abs() < 0.5);
        // Five are three over two, the short row centred.
        let five = tile_grid(5, area, TILE_GAP);
        let middle = (five[3].left() + five[4].right()) / 2.0;
        assert!((middle - area.center().x).abs() < 0.5);
        assert!(tile_grid(0, area, TILE_GAP).is_empty());
    }

    #[test]
    fn a_members_picture_covers_their_tile_from_its_middle() {
        // A wide picture in a 4:3 tile loses its sides.
        let wide = cover_uv(vec2(1600.0, 900.0), vec2(400.0, 300.0));
        assert!((wide.top() - 0.0).abs() < 1e-6 && (wide.bottom() - 1.0).abs() < 1e-6);
        assert!((wide.center().x - 0.5).abs() < 1e-6);
        assert!((wide.width() - (4.0 / 3.0) / (16.0 / 9.0)).abs() < 1e-4);
        // A portrait one loses its top and bottom.
        let tall = cover_uv(vec2(720.0, 1280.0), vec2(400.0, 300.0));
        assert!((tall.width() - 1.0).abs() < 1e-6);
        assert!((tall.center().y - 0.5).abs() < 1e-6);
        assert!(tall.height() < 1.0);
    }

    #[test]
    fn switching_cameras_goes_round_the_machines_list() {
        let camera = |id: &str| crate::calls::CameraDevice {
            id: id.to_owned(),
            label: id.to_owned(),
        };
        let devices = DeviceList {
            cameras: vec![camera("front"), camera("back")],
            ..DeviceList::default()
        };
        assert_eq!(next_camera(&devices, None).as_deref(), Some("back"));
        assert_eq!(
            next_camera(&devices, Some("back")).as_deref(),
            Some("front")
        );
        assert_eq!(next_camera(&DeviceList::default(), None), None);
    }

    #[test]
    fn a_new_picture_is_told_from_the_same_one() {
        let image = Arc::new(egui::ColorImage::filled([2, 2], Color32::BLACK));
        let one = Frames {
            remote: Some(Arc::clone(&image)),
            ..Frames::default()
        };
        let same = Frames {
            remote: Some(Arc::clone(&image)),
            ..Frames::default()
        };
        assert!(one.same(&same));
        let newer = Frames {
            remote: Some(Arc::new((*image).clone())),
            ..Frames::default()
        };
        assert!(!one.same(&newer), "an equal picture in a new frame is new");
        assert!(!one.same(&Frames::default()));
    }
}
