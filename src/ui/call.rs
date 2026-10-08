//! The call UI, drawn the way WhatsApp's desktop app draws it: a call that rings is a card in the
//! bottom-left corner of the main window, with who is calling and the two answers, and a call that
//! is up (answered, or placed from here) gets a small window of its own.
//!
//! Nothing here decides what a call is doing. The phase, the point the duration counts from, the
//! mute state and the device selections all come from [`crate::calls::CallUpdate`], which the worker
//! fills from the backend's own events. This module only draws them, and asks the app for something
//! when the user presses a button.
//!
//! The call window is a separate egui viewport with its own event loop turn, so it keeps drawing
//! while the main window is minimized. It cannot borrow the [`App`]: the app hands it a
//! [`CallView`] each frame through [`CallWindow`] and reads its [`CallRequest`]s back.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Layout, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
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
/// The ringing card's width, including its margins.
const CARD_WIDTH: f32 = 340.0;
/// The ringing card's round buttons.
const CARD_CONTROL: f32 = 42.0;
/// The call window's width, in points.
const WINDOW_WIDTH: f32 = 300.0;
/// Space between the call window's edges and what it draws.
const MARGIN: f32 = 14.0;
/// The avatar in the call window.
const AVATAR: f32 = 72.0;
/// The call window's round controls.
const CONTROL: f32 = 52.0;
/// The device pickers' width in the call window.
const PICKER_WIDTH: f32 = 260.0;
/// The height the two stacked device pickers take.
const DEVICES_HEIGHT: f32 = 112.0;
/// The level bars under the status: how many, how tall, and how often they move.
const BARS: usize = 25;
const BARS_HEIGHT: f32 = 24.0;
const LEVEL_STEP: Duration = Duration::from_millis(50);
/// The height of one warning line under the status.
const WARNING_LINE: f32 = 18.0;
/// The pin in the call window's corner.
const PIN: f32 = 26.0;

/// What the call window shows, as the app last handed it over.
#[derive(Clone, Debug, PartialEq)]
pub struct CallView {
    pub call: CallUpdate,
    /// The call's own name: "Locked chat" or "Unknown caller" where the app would not say more.
    pub peer: String,
    pub picture: Option<PathBuf>,
    pub palette: Palette,
    pub locale: Locale,
    pub devices: DeviceList,
    /// Whether the window should stay above other windows.
    pub on_top: bool,
}

/// What the call window asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallRequest {
    Mute(bool),
    Hangup,
    Microphone(Option<String>),
    Speaker(Option<String>),
    /// Keep the window above other windows, or stop.
    OnTop(bool),
    /// The window was closed: the app forgets the call it showed.
    Dismiss,
    /// Pick up the call that is ringing.
    Answer,
    /// Turn down the call that is ringing.
    Decline,
    /// The window of a call that is still ringing was closed: the call keeps ringing, and goes back
    /// to the card in the main window.
    Hide,
}

/// The state the app and the call window share.
#[derive(Debug, Default)]
pub struct CallWindow {
    /// What to draw; `None` once there is no call to show, which hides the window.
    pub view: Option<CallView>,
    /// What the window asked for since the app last looked.
    pub requests: Vec<CallRequest>,
    /// Whether the window shows its device pickers.
    pub devices_open: bool,
    /// The peer's loudness, newest last, one sample per [`LEVEL_STEP`].
    levels: std::collections::VecDeque<f32>,
    sampled: Option<Instant>,
    /// Whether the window was last asked to stay above the others; `None` for a new window.
    applied_on_top: Option<bool>,
    /// The height the window was last given.
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
        self.height = 0.0;
        self.placing = None;
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

/// The card in the bottom-left corner: who is calling, and Decline and Accept. A call that rang
/// and was never answered keeps the card for a moment to say how it ended.
fn card(app: &mut App, ctx: &egui::Context, call: &CallUpdate) {
    let palette = app.palette;
    let locale = app.locale;
    let peer = app.call_name(&call.chat);
    let picture = app.call_avatar(&call.chat);
    let ringing = call.phase == CallPhase::Incoming;
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
                    let width = CARD_WIDTH - 28.0;
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        widgets::avatar(ui, &palette, &peer, &call.chat, 44.0, picture.as_deref());
                        ui.add_space(10.0);
                        let buttons = if ringing {
                            2.0 * CARD_CONTROL + 18.0
                        } else {
                            0.0
                        };
                        let text = (width - 54.0 - buttons).max(60.0);
                        ui.allocate_ui_with_layout(
                            vec2(text, CARD_CONTROL),
                            Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(text);
                                theme::text(ui, &peer, theme::semibold(15.0), palette.text);
                                let (line, color) = if ringing {
                                    (
                                        gettext(locale, "Incoming voice call").into_owned(),
                                        palette.secondary,
                                    )
                                } else {
                                    (status(locale, call), status_color(call, &palette))
                                };
                                theme::text(ui, line, theme::medium(12.5), color);
                            },
                        );
                        if ringing {
                            ui.add_space(8.0);
                            let decline = gettext(locale, "Decline").into_owned();
                            let response = control(
                                ui,
                                Icon::Phone,
                                CARD_CONTROL,
                                palette.danger,
                                Color32::WHITE,
                                &decline,
                                true,
                            );
                            if response.clicked() {
                                app.actions.push(Action::DeclineCall);
                            }
                            ui.add_space(10.0);
                            let accept = gettext(locale, "Accept").into_owned();
                            let response = control(
                                ui,
                                Icon::Phone,
                                CARD_CONTROL,
                                palette.accent,
                                palette.on_accent,
                                &accept,
                                true,
                            );
                            if response.clicked() {
                                app.actions.push(Action::AnswerCall);
                            }
                        }
                    });
                });
        });
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
    ctx.show_viewport_deferred(window_id(), viewport(app, title), move |ui, _class| {
        window(ui, &shared, &waker);
    });
}

/// The call window's title: the call's own name.
fn window_title(app: &App) -> String {
    app.call
        .as_ref()
        .map(|call| app.call_name(&call.chat))
        .unwrap_or_default()
}

/// How the call window opens: its size, its icon, and the top-right corner of the screen.
fn viewport(app: &App, title: String) -> egui::ViewportBuilder {
    let size = [WINDOW_WIDTH, base_height()];
    // Its size is set from what it shows, through the minimum and the maximum: a window made
    // unresizable on Wayland keeps the size it opened with, which would leave no room for the device
    // pickers.
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id(std::env::var("FLATPAK_ID").unwrap_or_else(|_| "zapfast".to_owned()))
        .with_inner_size(size)
        .with_min_inner_size(size)
        .with_max_inner_size(size)
        .with_maximize_button(false)
        .with_icon(window_icon());
    // The top-right corner of the screen the main window is on. Wayland ignores a position, and
    // KDE's is set by the KWin script instead.
    match app.main_monitor {
        Some(monitor) if crate::keep_above::method() == crate::keep_above::Method::WindowLevel => {
            builder.with_position(pos2(
                monitor.right() - WINDOW_WIDTH - crate::keep_above::EDGE,
                monitor.top() + crate::keep_above::EDGE,
            ))
        }
        _ => builder,
    }
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
    window(ui, &app.call_window, &app.waker());
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

/// The call window's height with nothing under the status and the device pickers closed.
fn base_height() -> f32 {
    MARGIN + AVATAR + 8.0 + 24.0 + 2.0 + 18.0 + 8.0 + BARS_HEIGHT + 14.0 + CONTROL + MARGIN
}

/// The call window's height for what it shows now.
fn window_height(view: &CallView, devices_open: bool) -> f32 {
    let warnings = view.call.lost_devices.len() + usize::from(view.call.peer_audio.is_some());
    let devices = if devices_open {
        DEVICES_HEIGHT + 14.0
    } else {
        0.0
    };
    base_height() + warnings as f32 * WARNING_LINE + devices
}

/// One frame of the call window, in its own viewport.
fn window(ui: &mut egui::Ui, shared: &SharedCallWindow, waker: &crate::backend::Waker) {
    let ctx = ui.ctx().clone();
    let (view, mut devices_open, levels) = {
        let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
        let Some(view) = state.view.clone() else {
            // The call is gone and the main window has not taken the viewport down yet (it may be
            // minimized and not drawing), so the window gets out of the way on its own.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        };
        // Closing the window is hanging up: the call has nowhere else to be shown. A call that is
        // still ringing only stops being shown here.
        if ctx.input(|input| input.viewport().close_requested()) {
            if view.call.phase == CallPhase::Incoming {
                state.requests.push(CallRequest::Hide);
            } else {
                if view.call.phase.is_live() {
                    state.requests.push(CallRequest::Hangup);
                }
                state.requests.push(CallRequest::Dismiss);
            }
            state.view = None;
            drop(state);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            waker.wake();
            return;
        }
        keep_above(&ctx, &mut state, &view);
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
        let height = window_height(&view, state.devices_open);
        if (height - state.height).abs() > 0.5 {
            state.height = height;
            let size = vec2(WINDOW_WIDTH, height);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        (view, state.devices_open, state.levels.clone())
    };
    let mut requests = Vec::new();
    window_body(ui, &view, &levels, &mut devices_open, &mut requests);
    {
        let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
        state.devices_open = devices_open;
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

/// Who, how far along, how loud the peer is, and the controls.
fn window_body(
    ui: &mut egui::Ui,
    view: &CallView,
    levels: &std::collections::VecDeque<f32>,
    devices_open: &mut bool,
    requests: &mut Vec<CallRequest>,
) {
    let CallView {
        call,
        peer,
        picture,
        palette,
        locale,
        on_top,
        ..
    } = view;
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, surface_color(palette));

    if crate::keep_above::method() != crate::keep_above::Method::Unsupported {
        let pin = Rect::from_min_size(
            pos2(rect.right() - 8.0 - PIN, rect.top() + 8.0),
            Vec2::splat(PIN),
        );
        let tip = if *on_top {
            gettext(
                *locale,
                "The call window stays above other windows. Click to stop.",
            )
        } else {
            gettext(*locale, "Keep the call window above other windows")
        }
        .into_owned();
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(pin), |ui| {
                control(
                    ui,
                    Icon::Pin,
                    PIN,
                    if *on_top {
                        palette.accent
                    } else {
                        palette.surface_active
                    },
                    if *on_top {
                        palette.on_accent
                    } else {
                        palette.secondary
                    },
                    &tip,
                    true,
                )
            })
            .inner;
        if response.clicked() {
            requests.push(CallRequest::OnTop(!*on_top));
        }
    }

    let inner = rect.shrink(MARGIN);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            widgets::avatar(ui, palette, peer, &call.chat, AVATAR, picture.as_deref());
            ui.add_space(8.0);
            theme::text(ui, peer, theme::bold(18.0), palette.text);
            ui.add_space(2.0);
            theme::text(
                ui,
                status(*locale, call),
                theme::medium(13.0),
                status_color(call, palette),
            );
            ui.add_space(8.0);
            level_bars(ui, levels, palette);
            under_status(ui, *locale, call, palette);
            ui.add_space(14.0);
            if *devices_open {
                devices(ui, view, requests);
                ui.add_space(14.0);
            }
            ui.horizontal(|ui| controls(ui, view, devices_open, requests));
        },
    );
}

/// The peer's loudness as bars that spread from the middle: the newest sample in the centre, older
/// ones outwards, so the row moves like an equalizer while the peer talks and lies flat while they
/// are quiet.
fn level_bars(ui: &mut egui::Ui, levels: &std::collections::VecDeque<f32>, palette: &Palette) {
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

/// The line under the peer's name, derived from the backend's phase and nothing else.
fn status(locale: Locale, call: &CallUpdate) -> String {
    match call.phase {
        CallPhase::Dialing => gettext(locale, "Calling…").into_owned(),
        // The peer's phone is ringing: its `<preaccept>` arrived and nobody has answered yet.
        CallPhase::Ringing => gettext(locale, "Ringing…").into_owned(),
        // The peer answered and the media is still coming up. The duration has not started, which
        // is the whole point: this is the line that used to stay on screen as "Ringing…".
        CallPhase::Connecting => gettext(locale, "Connected").into_owned(),
        // This side picked the phone up and the media is still being negotiated.
        CallPhase::Accepted => gettext(locale, "Connecting…").into_owned(),
        // A live call shows its length, which starts when the media plane came up rather than when
        // the button was pressed.
        CallPhase::Active => call
            .started
            .map(elapsed)
            .unwrap_or_else(|| gettext(locale, "Connected").into_owned()),
        // A finished call says what became of it, from the peer's own signaling and the media
        // plane rather than from the button that was pressed.
        CallPhase::Ended | CallPhase::Failed => call
            .outcome
            .map(|outcome| outcome_text(locale, outcome, call.phase))
            .unwrap_or_else(|| gettext(locale, "Call ended").into_owned()),
        CallPhase::Incoming => gettext(locale, "Incoming call").into_owned(),
    }
}

/// How a call that is over reads on screen.
///
/// A lost relay is told from a call that never came up by the phase, since the outcome is the same
/// cause in both cases.
fn outcome_text(locale: Locale, outcome: CallOutcome, phase: CallPhase) -> String {
    match outcome {
        CallOutcome::Answered => gettext(locale, "Call ended").into_owned(),
        CallOutcome::Missed => gettext(locale, "Missed call").into_owned(),
        CallOutcome::Declined => gettext(locale, "Declined").into_owned(),
        CallOutcome::Busy => gettext(locale, "Busy").into_owned(),
        CallOutcome::Failed => gettext(locale, "Could not connect").into_owned(),
        CallOutcome::NoAnswer => gettext(locale, "No answer").into_owned(),
        CallOutcome::ConnectionLost => {
            if phase == CallPhase::Ended {
                gettext(locale, "Connection lost").into_owned()
            } else {
                gettext(locale, "The call could not be established").into_owned()
            }
        }
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

/// The lines under the status: what moved beneath it.
///
/// A device that went away is shown rather than swallowed, so a headset switching off reads as a
/// reason instead of as a call that quietly started using another microphone.
fn under_status(ui: &mut egui::Ui, locale: Locale, call: &CallUpdate, palette: &Palette) {
    if !call.lost_devices.is_empty() {
        ui.add_space(4.0);
        for lost in &call.lost_devices {
            theme::text(
                ui,
                lost_device(locale, lost),
                theme::medium(12.5),
                palette.warning,
            );
        }
    }
    // The engine's own diagnosis of a call that carries no audio from the peer: the peer is better
    // told that the call is one-sided than left wondering whether their microphone is broken.
    if let Some(audio) = call.peer_audio {
        ui.add_space(4.0);
        theme::text(
            ui,
            peer_audio_text(locale, audio),
            theme::medium(12.5),
            palette.warning,
        );
    }
}

/// What the engine says about the peer's audio, in the reader's language.
fn peer_audio_text(locale: Locale, audio: PeerAudio) -> String {
    match audio {
        PeerAudio::Stalled => gettext(locale, "No audio is arriving from this call").into_owned(),
        PeerAudio::Silent(SilenceReason::NoDecoder) => {
            gettext(locale, "This call's audio cannot be decoded").into_owned()
        }
        PeerAudio::Silent(SilenceReason::AuthenticationFailing) => {
            gettext(locale, "This call's audio is not authenticating").into_owned()
        }
        PeerAudio::Silent(SilenceReason::UnexpectedPayloadType) => gettext(
            locale,
            "This call's audio is arriving in an unexpected format",
        )
        .into_owned(),
        PeerAudio::Silent(SilenceReason::CodecRejectingFrames) => {
            gettext(locale, "This call's audio is being rejected by the decoder").into_owned()
        }
        PeerAudio::Silent(SilenceReason::CodecFlapping) => {
            gettext(locale, "This call's audio format keeps changing").into_owned()
        }
        PeerAudio::Silent(SilenceReason::Unknown) => {
            gettext(locale, "The peer's audio stopped").into_owned()
        }
    }
}

/// Why a device the user picked is not the one in use, with its name filled in.
///
/// Translators: `{name}` is replaced by the device's own description, which may be long.
fn lost_device(locale: Locale, lost: &crate::calls::LostDevice) -> String {
    let template = match lost.kind {
        crate::calls::DeviceKind::Microphone => gettext(
            locale,
            "Microphone “{name}” is not available; using the system default",
        ),
        crate::calls::DeviceKind::Speaker => gettext(
            locale,
            "Speaker “{name}” is not available; using the system default",
        ),
    };
    template.replace("{name}", &lost.name)
}

/// `mm:ss`, or `h:mm:ss` past an hour.
fn elapsed(started: Instant) -> String {
    let seconds = started.elapsed().as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
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
            fill.gamma_multiply(1.15)
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

/// The call window's round buttons: mute, devices, and hang up.
fn controls(
    ui: &mut egui::Ui,
    view: &CallView,
    devices_open: &mut bool,
    requests: &mut Vec<CallRequest>,
) {
    let CallView {
        call,
        palette,
        locale,
        ..
    } = view;
    let locale = *locale;
    if call.phase == CallPhase::Incoming {
        ringing_controls(ui, palette, locale, requests);
        return;
    }
    let connected = call.phase.is_connected();
    let live = call.phase.is_live();
    let buttons = 3.0;
    let spacing = 22.0;
    let width = buttons * CONTROL + (buttons - 1.0) * spacing;
    ui.spacing_mut().item_spacing.x = spacing;
    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));

    // Microphone: the engine's own mute flag, which is what stops outgoing audio.
    let tip = if call.muted {
        gettext(locale, "Unmute").into_owned()
    } else {
        gettext(locale, "Mute").into_owned()
    };
    let response = control(
        ui,
        if call.muted { Icon::VolumeX } else { Icon::Mic },
        CONTROL,
        if call.muted {
            palette.danger
        } else {
            palette.surface_active
        },
        palette.text,
        &tip,
        connected,
    );
    if response.clicked() {
        requests.push(CallRequest::Mute(!call.muted));
    }

    // The speaker button shows and hides the device pickers above it.
    let tip = gettext(locale, "Speaker and devices").into_owned();
    let response = control(
        ui,
        Icon::Volume2,
        CONTROL,
        if *devices_open {
            palette.accent
        } else {
            palette.surface_active
        },
        if *devices_open {
            palette.on_accent
        } else {
            palette.text
        },
        &tip,
        live,
    );
    if response.clicked() {
        *devices_open = !*devices_open;
    }

    let tip = gettext(locale, "Hang up").into_owned();
    let response = control(
        ui,
        Icon::Phone,
        CONTROL,
        palette.danger,
        Color32::WHITE,
        &tip,
        live,
    );
    if response.clicked() {
        requests.push(CallRequest::Hangup);
    }
}

/// Decline and Accept, for a call that is still ringing, so the window can answer it without the
/// main window.
fn ringing_controls(
    ui: &mut egui::Ui,
    palette: &Palette,
    locale: Locale,
    requests: &mut Vec<CallRequest>,
) {
    let spacing = 44.0;
    let width = 2.0 * CONTROL + spacing;
    ui.spacing_mut().item_spacing.x = spacing;
    ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
    let tip = gettext(locale, "Decline").into_owned();
    let response = control(
        ui,
        Icon::Phone,
        CONTROL,
        palette.danger,
        Color32::WHITE,
        &tip,
        true,
    );
    if response.clicked() {
        requests.push(CallRequest::Decline);
    }
    let tip = gettext(locale, "Accept").into_owned();
    let response = control(
        ui,
        Icon::Phone,
        CONTROL,
        palette.accent,
        palette.on_accent,
        &tip,
        true,
    );
    if response.clicked() {
        requests.push(CallRequest::Answer);
    }
}

/// The device pickers, one above the other. Each one rebinds a live stream, so what it shows is
/// what is in use.
fn devices(ui: &mut egui::Ui, view: &CallView, requests: &mut Vec<CallRequest>) {
    let CallView {
        call,
        locale,
        devices,
        ..
    } = view;
    let locale = *locale;
    let default_label = gettext(locale, "Default device").into_owned();
    let list = |devices: &[crate::calls::AudioDevice]| -> Vec<(String, String)> {
        devices
            .iter()
            .map(|device| (device.id.clone(), device.label.clone()))
            .collect()
    };
    let microphones = list(&devices.microphones);
    let speakers = list(&devices.speakers);
    let connected = call.phase.is_connected();
    let size = vec2(PICKER_WIDTH, DEVICES_HEIGHT / 2.0 - 4.0);

    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
        Picker {
            salt: "call-microphone",
            icon: Icon::Mic,
            title: &gettext(locale, "Microphone"),
            default_label: &default_label,
            current: call.microphone.as_deref(),
            devices: &microphones,
            enabled: connected,
            width: PICKER_WIDTH,
        }
        .show(ui, requests, CallRequest::Microphone);
    });
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
        Picker {
            salt: "call-speaker",
            icon: Icon::Volume2,
            title: &gettext(locale, "Speaker"),
            default_label: &default_label,
            current: call.speaker.as_deref(),
            devices: &speakers,
            enabled: connected,
            width: PICKER_WIDTH,
        }
        .show(ui, requests, CallRequest::Speaker);
    });
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
        actions: &mut Vec<CallRequest>,
        action: impl Fn(Option<String>) -> CallRequest,
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
