//! The window's side of a call: which account it is on, what the call card
//! and the call window show, the desktop notification, and the actions they
//! push, routed to the backend of the account the call belongs to.
//!
//! A call that rings is a card in the corner of the main window; once it is
//! answered or placed it moves to a small window of its own (a viewport,
//! `ui::call`), which the app hands a [`crate::ui::call::CallView`] every
//! frame and whose requests it reads back. A call that rings while the main
//! window is not in front rings in that window instead, as a popup in the
//! corner of the screen.
//!
//! The call's state comes from the worker that owns it (`Event::Call`); the
//! window never decides what a call is doing, with one exception: a call
//! whose button was just pressed is drawn as placing at once, before the
//! worker has answered, and the worker's updates take over from there. One
//! call at a time across every account, since there is one microphone: an
//! offer to another account while a call is up is declined there.
//!
//! The tones follow the phase: the ringback while the peer's phone rings (the
//! protocol's `<preaccept>`), the ringtone while a call to us waits, and a
//! chime when the call connects, ends, or finds the peer busy. The ringtone
//! follows the chat's quiet rules (muted, archived or locked chats do not
//! ring), not the message sound: a call rings even with message sounds off.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{App, CALL_FAREWELL};
use crate::backend::{Backend, Command};
use crate::call_tones::{Chime, Tone};
use crate::calls::{CallPhase, CallUpdate};
use crate::model::{AccountId, Action, CallDirection, Chat, ChatId};
use crate::settings::NotificationSound;

/// One person on a group call, as the call screen draws them.
#[derive(Clone, Debug, PartialEq)]
pub struct CallPerson {
    pub id: String,
    pub name: String,
    pub picture: Option<std::path::PathBuf>,
    /// Whether the call service counts them as in the call, not only asked.
    pub joined: bool,
    /// Whether their camera's picture is arriving.
    pub video: bool,
}

/// How long a call may stay placing with no word from its worker before the
/// screen gives up on it. Placing waits on device checks, the camera (whose
/// first use on macOS asks permission) and the peer's devices, so this is
/// generous; it is only there so a lost answer never leaves the screen stuck.
const PLACING_TIMEOUT: Duration = Duration::from_secs(60);

impl App {
    /// Whether a call is up or ringing, on any account.
    pub fn call_live(&self) -> bool {
        self.call.as_ref().is_some_and(|call| call.phase.is_live())
    }

    /// Whether the call is shown in its own window rather than on the
    /// ringing card: any call that is past ringing, and the farewell of one
    /// that got there.
    pub fn call_in_window(&self) -> bool {
        self.call.as_ref().is_some_and(|call| {
            self.call_windowed == Some(call.generation)
                || (call.phase != CallPhase::Incoming && call.phase.is_live())
        })
    }

    /// Whether the call window opens on its own, in place of the main
    /// window: a call shown in its window while the main window is in the
    /// tray and nobody asked for it.
    pub fn call_window_alone(&self) -> bool {
        self.window_hidden && !self.wants_show && !self.quit_requested && self.call_in_window()
    }

    /// Closes the standalone call window to open the main window in its place.
    pub fn reopen_window(&mut self) {
        self.reopen = true;
    }

    /// Whether a call needs the main window to stay when there is no tray to
    /// go to: hiding it then only minimizes it.
    pub(super) fn call_keeps_window(&self) -> bool {
        self.call_live()
    }

    /// Whether a ringing call offers Decline and Accept: not on an account in
    /// anonymous mode, which only logs it.
    pub fn call_answerable(&self) -> bool {
        !self.anonymous()
    }

    /// The handle that wakes this app from another thread or window.
    pub(crate) fn waker(&self) -> crate::backend::Waker {
        self.waker.clone()
    }

    /// Hands the call window what to show, and takes back what it asked for.
    ///
    /// Runs every frame the app runs, including those in which the main
    /// window only ticks its logic (minimized on Wayland), so the call window
    /// keeps up with the call either way.
    pub(super) fn sync_call_window(&mut self, ctx: &egui::Context) {
        use crate::ui::call::{CallRequest, CallView, Frames};
        let view = if self.call_in_window() {
            let locked = self.app_lock.is_locked();
            let unknown = crate::i18n::gettext(self.locale, "Unknown caller").into_owned();
            self.call.clone().map(|call| CallView {
                // Locked, the call window says nothing about who is on the line.
                peer: if locked { unknown } else { self.call_name() },
                picture: if locked { None } else { self.call_avatar() },
                caller: if locked {
                    None
                } else {
                    self.call_caller_name()
                },
                people: if locked {
                    Vec::new()
                } else {
                    self.call_people()
                },
                me: self.call_self(),
                palette: self.palette,
                locale: self.locale,
                devices: self.call_devices.clone(),
                on_top: self.settings.call_window_on_top,
                answerable: self.call_answerable(),
                call,
            })
        } else {
            None
        };
        // A locked chat's call shows no picture of anyone on it, as it names
        // nobody; nor does any call behind the app lock.
        let frames = if view.is_some() && !self.call_is_private() && !self.app_lock.is_locked() {
            Frames {
                remote: self.call_remote_frame.clone(),
                local: self.call_local_frame.clone(),
                members: self.call_member_frames.clone(),
            }
        } else {
            Frames::default()
        };
        let requests = {
            let mut window = self.call_window.lock().unwrap_or_else(|p| p.into_inner());
            let mut changed = !window.frames.same(&frames);
            window.frames = frames;
            if window.view != view {
                if view.is_none() {
                    window.reset();
                }
                window.view = view;
                changed = true;
            }
            if changed {
                ctx.request_repaint_of(crate::ui::call::window_id());
            }
            std::mem::take(&mut window.requests)
        };
        for request in requests {
            match request {
                CallRequest::Action(action) => self.actions.push(action),
                CallRequest::OnTop(on_top) => {
                    self.settings.call_window_on_top = on_top;
                    self.mark_settings_dirty();
                }
                CallRequest::Dismiss => {
                    self.call = None;
                    self.call_account = None;
                    self.call_surface_until = None;
                    self.drop_call_frames();
                }
                CallRequest::Hide => {
                    self.call_windowed = None;
                    self.call_alert = None;
                }
            }
        }
    }

    /// Whether this chat of the account on screen has the live call, which is
    /// what the chat header's call button reflects. A finished call keeps its
    /// snapshot for the farewell, but that is no call to hang up.
    pub fn call_live_here(&self, id: &str) -> bool {
        self.call_account.as_ref() == Some(&self.account().id)
            && self
                .call
                .as_ref()
                .is_some_and(|call| call.chat == id && call.phase.is_live())
    }

    /// Runs `f` with the call's account as the active one, so names and
    /// pictures come from the account the call is on.
    fn with_call_account<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let index = self
            .call_account
            .as_ref()
            .and_then(|id| self.accounts.iter().position(|account| &account.id == id))
            .unwrap_or(self.active);
        let real = std::mem::replace(&mut self.active, index);
        let result = f(self);
        self.active = real;
        result
    }

    /// Whether the call belongs to a chat the lock is hiding right now: a
    /// locked chat says nothing until its folder is open, and a folder is only
    /// ever open for the account on screen.
    pub(crate) fn call_is_private(&self) -> bool {
        let (Some(call), Some(account)) = (&self.call, &self.call_account) else {
            return false;
        };
        let Some(owner) = self.accounts.iter().find(|owner| &owner.id == account) else {
            return false;
        };
        let locked = owner
            .chats
            .iter()
            .any(|chat| chat.id == call.chat && chat.locked);
        let on_screen = !self.events_hidden && owner.id == self.account().id;
        locked && !(on_screen && self.locked_folder_open())
    }

    /// The name the call shows for the other side: the chat's own title (a
    /// group's subject for a group call), or "Locked chat" while the lock
    /// hides it.
    pub fn call_name(&mut self) -> String {
        let Some(call) = self.call.as_ref() else {
            return String::new();
        };
        // A call joined through a link belongs to no chat.
        if call.group.as_ref().is_some_and(|group| group.link) {
            return crate::i18n::gettext(self.locale, "Call link").into_owned();
        }
        let chat = call.chat.clone();
        if self.call_is_private() {
            return crate::i18n::gettext(self.locale, "Locked chat").into_owned();
        }
        self.with_call_account(|app| app.chat_title_of(&chat))
    }

    /// Who rang a group call, by name: `None` for a one-to-one call, whose
    /// chat already says who, and while the lock hides the chat.
    pub fn call_caller_name(&mut self) -> Option<String> {
        let caller = self.call.as_ref()?.group.as_ref()?.caller.clone()?;
        if self.call_is_private() {
            return None;
        }
        Some(self.with_call_account(|app| app.display_name(&caller)))
    }

    /// Ourselves on a group call, as our own tile shows us: our photo under
    /// "You". `None` on a one-to-one call.
    pub fn call_self(&mut self) -> Option<CallPerson> {
        self.call.as_ref()?.group.as_ref()?;
        let you = crate::i18n::gettext(self.locale, "You").into_owned();
        self.with_call_account(|app| {
            let id = app.me.clone().unwrap_or_default();
            Some(CallPerson {
                picture: (!id.is_empty()).then(|| app.avatar(&id)).flatten(),
                name: you,
                id,
                joined: true,
                video: false,
            })
        })
    }

    /// The people on a group call, named and pictured by the call's own
    /// account. Nobody while the lock hides the chat: a member's name says as
    /// much about the chat as its own does.
    pub fn call_people(&mut self) -> Vec<CallPerson> {
        let Some(group) = self.call.as_ref().and_then(|call| call.group.clone()) else {
            return Vec::new();
        };
        if self.call_is_private() {
            return Vec::new();
        }
        self.with_call_account(|app| {
            group
                .participants
                .into_iter()
                .map(|person| CallPerson {
                    name: app.display_name(&person.id),
                    picture: app.avatar(&person.id),
                    joined: person.joined,
                    video: person.video,
                    id: person.id,
                })
                .collect()
        })
    }

    /// The picture the call shows for the other side: none while the chat is
    /// locked, because a photo is the same disclosure the name is.
    pub fn call_avatar(&mut self) -> Option<std::path::PathBuf> {
        let call = self.call.as_ref()?;
        if call.chat.is_empty() {
            return None;
        }
        let chat = call.chat.clone();
        if self.call_is_private() {
            return None;
        }
        self.with_call_account(|app| app.avatar(&chat))
    }

    /// The backend of the account the call is on.
    fn call_backend(&self) -> Option<&Backend> {
        let account = self.call_account.as_ref()?;
        self.accounts
            .iter()
            .find(|owner| &owner.id == account)
            .map(|owner| &owner.backend)
    }

    /// Sends a command to the call's own account, whichever is on screen.
    pub(super) fn send_call_command(&self, command: Command) {
        if let Some(backend) = self.call_backend() {
            backend.send(command);
        }
    }

    /// Forgets a video call's pictures, so nothing frozen is drawn as live.
    fn drop_call_frames(&mut self) {
        self.call_local_frame = None;
        self.call_remote_frame = None;
        self.call_member_frames.clear();
    }

    /// A video call's newest pictures, from the account the call is on.
    pub(super) fn handle_call_video(
        &mut self,
        local: Option<std::sync::Arc<egui::ColorImage>>,
        remote: Option<std::sync::Arc<egui::ColorImage>>,
        from: Option<String>,
    ) {
        if self.call_account.as_ref() != Some(&self.account().id) || !self.call_live() {
            return;
        }
        if let Some(image) = local {
            self.call_local_frame = Some(image);
        }
        match (remote, from) {
            (Some(image), Some(member)) => {
                self.call_member_frames.insert(member, image);
            }
            (Some(image), None) => self.call_remote_frame = Some(image),
            (None, _) => {}
        }
    }

    /// Applies one call state from the backend of the account being handled.
    ///
    /// A finished call keeps its surface for a moment so the outcome can be
    /// read, then goes; the backend has already released the media.
    pub(super) fn handle_call_update(&mut self, update: CallUpdate) {
        let account = self.account().id.clone();
        let other_account = self
            .call_account
            .as_ref()
            .is_some_and(|current| current != &account);
        if other_account && self.call_live() {
            // One call at a time: an offer to another account while a call is
            // up is declined there, and its later updates are not this call's.
            if update.phase == CallPhase::Incoming {
                self.backend.send(Command::DeclineCall);
            }
            return;
        }
        // The call the reader hung up or declined here already left the
        // screen; whatever the worker still says about it is not drawn.
        if self.call_dismissed == Some(update.generation) {
            return;
        }
        // An older call's update is stale, except that an incoming call takes
        // the place of one that is only being placed: the worker refuses the
        // placing one as busy.
        if !other_account
            && self.call.as_ref().is_some_and(|current| {
                current.generation > update.generation && current.phase != CallPhase::Placing
            })
        {
            return;
        }
        // The end of a call other than the one on screen, such as one placed
        // here and refused because a call to us came first, is not drawn over
        // the live one.
        if !other_account
            && !update.phase.is_live()
            && self.call.as_ref().is_some_and(|current| {
                current.generation != update.generation && current.phase.is_live()
            })
        {
            return;
        }
        if update.phase != CallPhase::Placing {
            self.call_placed = None;
        }
        // A call we announced that no longer rings was picked up or given up:
        // its notification goes, and only its own.
        if update.phase != CallPhase::Incoming && self.call_notified == Some(update.generation) {
            self.call_notified = None;
            self.notifications.clear_call(&account, &update.chat);
        }
        let ringing = update.phase == CallPhase::Incoming
            && self
                .call
                .as_ref()
                .is_none_or(|current| current.generation != update.generation);
        let finished = !update.phase.is_live();
        let incoming = update.direction == CallDirection::Incoming;
        let was_live = self.call_live() && !other_account;
        // The same call's last phase, which tells a change worth a chime from a repeat.
        let previous = self
            .call
            .as_ref()
            .filter(|current| !other_account && current.generation == update.generation)
            .map(|current| current.phase);
        let chime = chime(previous, &update);
        // A peer who stopped sending video leaves no picture: the last frame
        // is not a live one, so the surface goes back to their photo.
        if !update.remote_video || other_account {
            self.call_remote_frame = None;
        }
        if !update.camera_on || other_account {
            self.call_local_frame = None;
        }
        // Likewise a member whose video stopped, or who left: their tile goes back to their photo.
        let sending: Vec<&str> = update
            .group
            .iter()
            .flat_map(|group| &group.participants)
            .filter(|person| person.video && !other_account)
            .map(|person| person.id.as_str())
            .collect();
        self.call_member_frames
            .retain(|member, _| sending.contains(&member.as_str()));
        self.call = Some(update);
        self.call_account = Some(account);
        if finished {
            if !was_live && self.call_surface_until.is_none() {
                // Nothing was drawn for this call, so there is nothing to
                // take down or say farewell to.
                self.call = None;
                self.call_account = None;
                return;
            }
            self.call_surface_until = Some(Instant::now() + CALL_FAREWELL);
            self.drop_call_frames();
        } else {
            self.call_surface_until = None;
            // Answered here, answered on the other side, or placed from here:
            // the call moves from the card to its own window, where its
            // farewell is shown too.
            if let Some(call) = self
                .call
                .as_ref()
                .filter(|call| call.phase != CallPhase::Incoming)
            {
                self.call_windowed = Some(call.generation);
            }
        }
        if finished || incoming {
            self.call_cancel = None;
        }
        self.request_call_repaint();
        if ringing {
            if self.call_should_ring() {
                self.call_rings = self.call.as_ref().map(|call| call.generation);
            }
            // While the main window is not in front, the call rings in the
            // call window in the corner of the screen.
            self.call_alert = self.call.as_ref().map(|call| call.generation);
            self.notify_incoming_call();
        }
        if let Some(chime) = chime {
            let speaker = self.call.as_ref().and_then(|call| call.speaker.clone());
            self.call_tones.chime(chime, speaker.as_deref());
        }
        self.sync_call_tone();
    }

    /// Whether the call ringing now plays the ringtone: always, unless the
    /// account is silenced, anonymous (the call cannot be answered here), or
    /// the chat stays quiet (muted, archived or locked). The message sound
    /// plays no part: a call rings even when messages make no sound. A
    /// caller without a chat row is a stranger, and rings.
    fn call_should_ring(&self) -> bool {
        let Some(call) = self.call.as_ref() else {
            return false;
        };
        if self.account().settings.silenced || self.anonymous() {
            return false;
        }
        self.chat(&call.chat)
            .is_none_or(|chat| call_notification_eligible(chat, crate::util::now()))
    }

    /// Places a call from the chat header or the Calls page: the screen shows
    /// it as placing in this same frame, with the peer's name and picture,
    /// and the worker's updates move it on once its offer is out.
    fn place_call(&mut self, chat: ChatId, video: bool) {
        let generation = crate::calls::next_generation();
        let cancel = Arc::new(AtomicBool::new(false));
        self.backend.send(Command::StartCall {
            chat: chat.clone(),
            video,
            generation,
            cancel: Arc::clone(&cancel),
        });
        self.handle_call_update(CallUpdate::placing(generation, chat.to_string(), video));
        self.call_cancel = Some(cancel);
        self.call_placed = Some(Instant::now());
    }

    /// Joins a call link from a message: drawn as connecting in this same
    /// frame, as a placed call is, while the worker asks to be let in.
    pub(super) fn join_call_link(&mut self, token: String, video: bool) {
        if self.call_live() {
            self.toast_error(crate::i18n::gettext(
                self.locale,
                "Hang up the current call first",
            ));
            return;
        }
        let generation = crate::calls::next_generation();
        let cancel = Arc::new(AtomicBool::new(false));
        self.backend.send(Command::JoinCallLink {
            token,
            video,
            generation,
            cancel: Arc::clone(&cancel),
        });
        let mut placing = CallUpdate::placing(generation, String::new(), video);
        placing.group = Some(crate::calls::GroupCall {
            link: true,
            ..crate::calls::GroupCall::default()
        });
        self.handle_call_update(placing);
        self.call_cancel = Some(cancel);
        self.call_placed = Some(Instant::now());
    }

    /// Takes down the call the reader hung up or declined here at once: they
    /// know how it ended, so there is no farewell to wait through. Its
    /// notification goes with it, and the worker's last updates about it are
    /// not drawn again.
    fn dismiss_call(&mut self) {
        if let Some(cancel) = self.call_cancel.take() {
            // A call still being placed is not offered at all.
            cancel.store(true, Ordering::Relaxed);
        }
        let Some(call) = self.call.take() else {
            return;
        };
        // Our own hang-up or refusal ends the call as the peer's would, with
        // its chime; a call still being placed was never heard.
        if call.phase.is_live() && call.phase != CallPhase::Placing {
            self.call_tones.chime(Chime::End, call.speaker.as_deref());
        }
        self.call_dismissed = Some(call.generation);
        if self.call_notified == Some(call.generation) {
            self.call_notified = None;
            if let Some(account) = self.call_account.clone() {
                self.notifications.clear_call(&account, &call.chat);
            }
        }
        self.call_account = None;
        self.call_surface_until = None;
        self.call_placed = None;
        self.call_rings = None;
        self.drop_call_frames();
        self.sync_call_tone();
        self.request_call_repaint();
    }

    /// Plays the tone the call's phase calls for, and stops the one it no
    /// longer does: the ringback only while the peer's phone rings, the
    /// ringtone only while a call to us waits and its notification may sound.
    fn sync_call_tone(&mut self) {
        let tone = match &self.call {
            Some(call)
                if call.phase == CallPhase::Ringing
                    && call.direction == CallDirection::Outgoing =>
            {
                Some(Tone::Ringback)
            }
            Some(call)
                if call.phase == CallPhase::Incoming
                    && self.call_rings == Some(call.generation) =>
            {
                Some(Tone::Ringtone)
            }
            _ => None,
        };
        self.call_tones.set(tone);
    }

    /// The tone playing for the call now, if any.
    pub fn call_tone(&self) -> Option<Tone> {
        self.call_tones.current()
    }

    /// The last chime a call played, if any.
    pub fn call_chime(&self) -> Option<Chime> {
        self.call_tones.last_chime()
    }

    fn request_call_repaint(&self) {
        self.waker.wake();
    }

    /// Tells the desktop a call is waiting: the window may be in the tray or
    /// behind another program. The click brings the window up, where the
    /// call waits on its card, or in its own window, for Accept or Decline.
    fn notify_incoming_call(&mut self) {
        let Some(call) = self.call.clone() else {
            return;
        };
        if !self.account().settings.notifications {
            return;
        }
        let now = crate::util::now();
        // The chat's own rules: a call in a chat that is muted, archived or
        // locked stays as quiet as a message in it. A caller without a chat
        // row is a stranger, and rings with the ordinary sound.
        if self
            .chat(&call.chat)
            .is_some_and(|chat| !call_notification_eligible(chat, now))
        {
            return;
        }
        let (title, body, picture) = if self.app_lock.is_locked() {
            // Behind the app lock the desktop learns that a call came, and
            // nothing about who is calling.
            (
                "ZapFast".to_owned(),
                crate::i18n::gettext(self.locale, "Incoming call").into_owned(),
                None,
            )
        } else {
            // A group call names its group, and in the body who is calling.
            let caller = self.call_caller_name();
            (
                self.call_name(),
                crate::ui::call::incoming_text(self.locale, &call, caller.as_deref()),
                self.call_avatar(),
            )
        };
        // The ringtone is the call's sound (see `call_should_ring`), so the
        // notification itself stays quiet and the two never play over each
        // other.
        let waker = self.waker.clone();
        self.call_notified = Some(call.generation);
        self.wants_attention = true;
        self.notifications.show(
            title,
            body,
            picture,
            NotificationSound::None,
            crate::notify::NotificationTarget {
                account: self.account().id.clone(),
                chat: call.chat.clone(),
                message: None,
            },
            std::sync::Arc::clone(&self.notification_opens),
            move || waker.wake(),
            // A call is never held back.
            false,
        );
    }

    /// Per-frame upkeep: the farewell's end, a placing call that never heard
    /// back, a ringing call moving to its window when the main window is not
    /// in front, the running timer, and the call's tone.
    pub(super) fn tick_call(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.call_surface_until.is_some_and(|until| now >= until) {
            self.call_surface_until = None;
            self.call = None;
            self.call_account = None;
            self.drop_call_frames();
        }
        if let Some(placed) = self.call_placed
            && now.duration_since(placed) >= PLACING_TIMEOUT
            && let Some(call) = self
                .call
                .clone()
                .filter(|call| call.phase == CallPhase::Placing)
        {
            log::warn!("[CALL] the call was not placed in time");
            if let Some(cancel) = self.call_cancel.take() {
                cancel.store(true, Ordering::Relaxed);
            }
            self.handle_call_update(call.not_placed());
        }
        // A call that rings while the main window is in the tray, minimized,
        // or behind other windows rings in the call window in the corner of
        // the screen, with Decline and Accept, and leaves the main window
        // where it is. In front, the card in its corner is enough; once the
        // main window leaves the front, the card makes way for the call
        // window for good, so the call is never offered in two places.
        if (self.window_hidden || !self.window_focused)
            && let Some(call) = self.call.as_ref().filter(|call| {
                call.phase == CallPhase::Incoming && self.call_alert == Some(call.generation)
            })
        {
            self.call_windowed = Some(call.generation);
        }
        self.sync_call_tone();
        if let Some(until) = self.call_surface_until {
            // The farewell goes on time even with nothing else to draw.
            ctx.request_repaint_after(until.saturating_duration_since(now));
        }
        if self.call.as_ref().is_some_and(|call| call.phase.is_live()) {
            // The duration changes every second.
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Applies the actions the call card, the call window and the chat
    /// header push.
    /// Returns the action back when it is not a call action.
    pub(super) fn apply_call_action(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::StartCall(chat) | Action::StartVideoCall(chat) if self.call_live() => {
                let _ = chat;
                self.toast_error(crate::i18n::gettext(
                    self.locale,
                    "Hang up the current call first",
                ));
            }
            Action::StartCall(chat) => self.place_call(chat, false),
            Action::StartVideoCall(chat) => self.place_call(chat, true),
            Action::SetCallCamera(on) => self.send_call_command(Command::SetCallCamera(on)),
            Action::SetCallCameraDevice(device) => {
                self.send_call_command(Command::SetCallCameraDevice(device));
            }
            Action::AnswerCall => self.send_call_command(Command::AnswerCall),
            // The worker answers before it turns the camera off, so the peer
            // sees a voice answer to their video call.
            Action::AnswerCallWithoutVideo => {
                self.send_call_command(Command::AnswerCall);
                self.send_call_command(Command::SetCallCamera(false));
            }
            Action::DeclineCall => {
                self.send_call_command(Command::DeclineCall);
                self.dismiss_call();
            }
            Action::HangupCall => {
                self.send_call_command(Command::HangupCall);
                self.dismiss_call();
            }
            Action::SetCallMuted(muted) => self.send_call_command(Command::SetCallMuted(muted)),
            Action::SetCallMicrophone(device) => {
                // Picked here means preferred from now on, so it is written to the settings the
                // same moment the live call is rebound.
                self.settings.call_microphone = device.clone();
                self.mark_settings_dirty();
                self.send_call_command(Command::SetCallMicrophone(device));
            }
            Action::SetCallSpeaker(device) => {
                self.settings.call_speaker = device.clone();
                self.mark_settings_dirty();
                self.send_call_command(Command::SetCallSpeaker(device));
            }
            other => return Some(other),
        }
        None
    }

    /// Whether the account being removed or unlinked has the call: its
    /// backend ends it, and the surface goes with it.
    pub(super) fn forget_call_of(&mut self, account: &AccountId) {
        if self.call_account.as_ref() == Some(account) {
            self.call = None;
            self.call_account = None;
            self.call_surface_until = None;
            self.call_rings = None;
            self.call_cancel = None;
            self.call_placed = None;
            self.drop_call_frames();
            self.sync_call_tone();
        }
    }
}

/// The chime a call moving from `previous` (the same call's last phase, or
/// `None` for a call not seen before) to `update` plays: connecting, the end
/// of a call that was heard here, and the peer busy or declining ours. A call
/// that rang here and was missed, or taken or turned down elsewhere, only
/// stops ringing.
fn chime(previous: Option<CallPhase>, update: &CallUpdate) -> Option<Chime> {
    use crate::calls::CallOutcome;
    let previous = previous?;
    match update.phase {
        CallPhase::Active if previous != CallPhase::Active => Some(Chime::Connect),
        CallPhase::Ended | CallPhase::Failed if previous.is_live() => match update.outcome {
            Some(CallOutcome::Busy) => Some(Chime::Busy),
            Some(CallOutcome::Declined) if update.direction == CallDirection::Outgoing => {
                Some(Chime::Busy)
            }
            Some(
                CallOutcome::Missed
                | CallOutcome::AnsweredElsewhere
                | CallOutcome::DeclinedElsewhere,
            ) => None,
            // A call that was never offered says why in words, not with a chime.
            _ if previous == CallPhase::Placing => None,
            _ => Some(Chime::End),
        },
        _ => None,
    }
}

/// Whether a call in this chat may raise a notification: the quiet rules a
/// message follows, without the unread one, since a call often comes to a
/// chat with nothing unread.
pub(super) fn call_notification_eligible(chat: &Chat, now: i64) -> bool {
    !chat.archived && !chat.muted(now) && !chat.locked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Account;
    use crate::backend::LinkStatus;
    use crate::calls::CallOutcome;
    use crate::model::CallDirection;
    use crate::paths::AppDirs;
    use crate::settings::Settings;

    const ADA: &str = "15550001111@s.whatsapp.net";

    /// A window with one account whose commands a test can read.
    fn app(directory: &std::path::Path) -> (App, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let mut app = App::headless(AppDirs::under(directory), Settings::default()).0;
        let (backend, commands) = Backend::recording();
        app.backend = backend;
        app.chats.push(Chat::new(ADA.into(), "Ada".into()));
        (app, commands)
    }

    /// Adds a second account, not on screen, with its own command queue.
    fn second_account(app: &mut App) -> tokio::sync::mpsc::UnboundedReceiver<Command> {
        let mut second = Account::detached(
            &app.dirs,
            AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        let (backend, commands) = Backend::recording();
        second.backend = backend;
        second.link = LinkStatus::Connected;
        second
            .chats
            .push(Chat::new(ADA.into(), "Ada at work".into()));
        app.accounts.push(second);
        commands
    }

    fn sent(commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>) -> Vec<Command> {
        std::iter::from_fn(|| commands.try_recv().ok()).collect()
    }

    fn update(generation: u64, phase: CallPhase) -> CallUpdate {
        CallUpdate {
            generation,
            chat: ADA.to_owned(),
            direction: CallDirection::Incoming,
            video: false,
            phase,
            started: (phase == CallPhase::Active).then(Instant::now),
            muted: false,
            camera_on: false,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
            group: None,
        }
    }

    fn ended(generation: u64, outcome: CallOutcome) -> CallUpdate {
        let mut update = update(generation, CallPhase::Ended);
        update.outcome = Some(outcome);
        update
    }

    #[test]
    fn a_ringing_call_is_a_card_and_is_announced_once() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let ctx = egui::Context::default();
        app.window_focused = true;
        app.handle_call_update(update(1, CallPhase::Incoming));
        app.tick_call(&ctx);
        assert!(app.call_live());
        assert!(
            !app.call_in_window(),
            "a call that rings in front is the card in the main window"
        );
        assert_eq!(app.notifications.shown.len(), 1, "the desktop is told");
        assert_eq!(app.notifications.shown[0].title, "Ada");
        // The same call again is not a second ring.
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.notifications.shown.len(), 1);
        // Answered, its notification is taken back and the call gets its window.
        app.handle_call_update(update(1, CallPhase::Active));
        assert_eq!(app.call_notified, None);
        assert!(app.call_in_window(), "the running call is in its window");
    }

    #[test]
    fn a_call_that_rings_behind_other_windows_rings_in_the_corner() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        app.window_focused = false;
        app.handle_call_update(update(1, CallPhase::Incoming));
        app.tick_call(&ctx);
        assert!(
            app.call_in_window(),
            "the call window pops up with the answers"
        );
        app.sync_call_window(&ctx);
        let view = app.call_window.lock().unwrap().view.clone();
        assert!(view.is_some_and(|view| view.answerable));
        // Closing it only stops showing it there: the call keeps ringing, on the card.
        crate::ui::call::close_window(&mut app.call_window.lock().unwrap());
        app.sync_call_window(&ctx);
        app.tick_call(&ctx);
        assert!(app.call_live(), "the call still rings");
        assert!(!app.call_in_window(), "back on the card, for good");
        assert!(app.actions.is_empty(), "nothing was declined");
        assert!(
            !sent(&mut commands)
                .iter()
                .any(|command| matches!(command, Command::DeclineCall | Command::HangupCall))
        );
    }

    #[test]
    fn a_call_that_ends_says_how_then_goes() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Active));
        app.handle_call_update(ended(1, CallOutcome::Answered));
        assert!(app.call.is_some(), "the farewell is on screen");
        assert!(app.call_in_window(), "in the window the call had");
        assert!(!app.call_live_here(ADA), "but the header offers a new call");
        app.call_surface_until = Some(Instant::now() - Duration::from_secs(1));
        app.tick_call(&ctx);
        assert!(app.call.is_none(), "and then it goes");
        assert!(app.call_account.is_none());
        app.sync_call_window(&ctx);
        assert!(
            app.call_window.lock().unwrap().view.is_none(),
            "and its window with it"
        );
    }

    #[test]
    fn a_finished_call_nobody_saw_is_not_drawn() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        app.handle_call_update(ended(3, CallOutcome::Failed));
        assert!(app.call.is_none());
    }

    #[test]
    fn closing_the_call_window_hangs_up() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Active));
        app.apply(Action::OpenChat(ADA.into()), &ctx);
        assert!(
            !sent(&mut commands)
                .iter()
                .any(|command| matches!(command, Command::HangupCall)),
            "going about the chats leaves the call alone"
        );
        app.sync_call_window(&ctx);
        assert!(app.call_window.lock().unwrap().view.is_some());
        crate::ui::call::close_window(&mut app.call_window.lock().unwrap());
        app.sync_call_window(&ctx);
        let actions = std::mem::take(&mut app.actions);
        assert!(
            matches!(actions.as_slice(), [Action::HangupCall]),
            "{actions:?}"
        );
        for action in actions {
            app.apply(action, &ctx);
        }
        assert!(
            sent(&mut commands)
                .iter()
                .any(|command| matches!(command, Command::HangupCall)),
            "and closing it hangs up"
        );
        assert!(
            app.call.is_none(),
            "the window is the call: it goes with it"
        );
    }

    #[test]
    fn hiding_the_window_keeps_the_call() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        app.handle_call_update(update(1, CallPhase::Active));
        app.window_gone();
        assert!(app.call_live(), "a hidden window is still a call");
        assert!(app.call_in_window());
        assert!(sent(&mut commands).is_empty());
    }

    #[test]
    fn a_video_call_can_be_answered_with_voice_only() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        let mut ringing = update(1, CallPhase::Incoming);
        ringing.video = true;
        app.handle_call_update(ringing);
        app.apply(Action::AnswerCallWithoutVideo, &ctx);
        let sent: Vec<Command> = sent(&mut commands)
            .into_iter()
            .filter(|command| !matches!(command, Command::FetchAvatar { .. }))
            .collect();
        assert!(
            matches!(
                sent.as_slice(),
                [Command::AnswerCall, Command::SetCallCamera(false)]
            ),
            "{sent:?}"
        );
    }

    #[test]
    fn a_locked_chat_says_nothing_on_a_call_until_its_folder_opens() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let ctx = egui::Context::default();
        app.chats[0].locked = true;
        app.settings.set_chat_lock_code(Some("test-code"));
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.call_name(), "Locked chat");
        assert!(app.call_avatar().is_none(), "no photo either");
        assert!(
            app.notifications.shown.is_empty(),
            "a locked chat's call is as quiet as its messages"
        );
        app.apply(Action::UnlockLockedFolder("test-code".into()), &ctx);
        assert_eq!(app.call_name(), "Ada");
        app.close_locked_folder();
        assert_eq!(app.call_name(), "Locked chat");
    }

    /// A group member's picture stays on their tile while their video runs, and goes when it
    /// stops; it never stands in for a one-to-one peer's.
    #[test]
    fn a_members_picture_lasts_as_long_as_their_video() {
        use crate::calls::{GroupCall, Participant};
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let with = |video: bool| {
            let mut update = update(1, CallPhase::Active);
            update.video = true;
            update.group = Some(GroupCall {
                link: false,
                caller: None,
                participants: vec![Participant {
                    id: ADA.to_owned(),
                    joined: true,
                    video,
                }],
            });
            update
        };
        app.handle_call_update(with(false));
        let image = Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::BLACK));
        app.handle_call_video(None, Some(image), Some(ADA.to_owned()));
        app.handle_call_update(with(true));
        assert!(app.call_member_frames.contains_key(ADA));
        assert!(app.call_remote_frame.is_none(), "not a one-to-one picture");
        assert!(app.call_people()[0].video);
        app.handle_call_update(with(false));
        assert!(
            app.call_member_frames.is_empty(),
            "a stopped camera leaves no frozen picture"
        );
    }

    /// A call link from a message is joined here rather than opened in the browser, and is
    /// drawn at once under the link's name, with no chat behind it.
    #[test]
    fn a_call_link_is_joined_here() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        app.apply(
            Action::OpenUrl("https://call.whatsapp.com/video/AbCdEf1234567890".to_owned()),
            &ctx,
        );
        let sent: Vec<Command> = sent(&mut commands)
            .into_iter()
            .filter(|command| !matches!(command, Command::FetchAvatar { .. }))
            .collect();
        assert!(
            matches!(
                sent.as_slice(),
                [Command::JoinCallLink { token, video: true, .. }] if token == "AbCdEf1234567890"
            ),
            "{sent:?}"
        );
        let call = app.call.clone().expect("on screen at once");
        assert_eq!(call.phase, CallPhase::Placing);
        assert_eq!(app.call_name(), "Call link");
        assert!(app.call_avatar().is_none());
        // A second link while that call is up is not joined.
        app.apply(
            Action::OpenUrl("https://call.whatsapp.com/voice/AbCdEf1234567890".to_owned()),
            &ctx,
        );
        assert!(self::sent(&mut commands).is_empty());
    }

    #[test]
    fn a_call_chimes_as_it_connects_and_ends() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        let (generation, _) = press_call(&mut app, &mut commands, false);
        app.handle_call_update(outgoing(generation, CallPhase::Ringing));
        assert_eq!(app.call_chime(), None);
        app.handle_call_update(outgoing(generation, CallPhase::Active));
        assert_eq!(app.call_chime(), Some(Chime::Connect));
        assert_eq!(app.call_tone(), None);
        let mut over = outgoing(generation, CallPhase::Ended);
        over.outcome = Some(CallOutcome::Answered);
        app.handle_call_update(over);
        assert_eq!(app.call_chime(), Some(Chime::End));

        // The peer busy, on a fresh window.
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = self::app(directory.path());
        let (generation, _) = press_call(&mut app, &mut commands, false);
        app.handle_call_update(outgoing(generation, CallPhase::Ringing));
        let mut busy = outgoing(generation, CallPhase::Failed);
        busy.outcome = Some(CallOutcome::Busy);
        app.handle_call_update(busy);
        assert_eq!(app.call_chime(), Some(Chime::Busy));

        // Our own hang-up ends with the same chime as the peer's.
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = self::app(directory.path());
        let (generation, _) = press_call(&mut app, &mut commands, false);
        app.handle_call_update(outgoing(generation, CallPhase::Active));
        app.apply(Action::HangupCall, &ctx);
        assert_eq!(app.call_chime(), Some(Chime::End));

        // A call that rang here and was missed only stops ringing.
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = self::app(directory.path());
        app.handle_call_update(update(1, CallPhase::Incoming));
        let mut missed = update(1, CallPhase::Failed);
        missed.outcome = Some(CallOutcome::Missed);
        app.handle_call_update(missed);
        assert_eq!(app.call_chime(), None);
    }

    #[test]
    fn a_call_in_a_muted_chat_is_not_announced() {
        let now = crate::util::now();
        let mut chat = Chat::new(ADA.into(), "Ada".into());
        assert!(call_notification_eligible(&chat, now));
        chat.muted_until = Some(now + 3600);
        assert!(!call_notification_eligible(&chat, now));
        chat.muted_until = None;
        chat.archived = true;
        assert!(!call_notification_eligible(&chat, now));
        chat.archived = false;
        chat.locked = true;
        assert!(!call_notification_eligible(&chat, now));
    }

    #[test]
    fn the_calls_actions_reach_the_account_it_is_on() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut first) = app(directory.path());
        let mut second = second_account(&mut app);
        let ctx = egui::Context::default();
        // The second account rings while the first is on screen.
        app.active = 1;
        app.handle_call_update(update(1, CallPhase::Incoming));
        app.active = 0;
        assert_eq!(app.call_account, AccountId::parse("2"));
        assert_eq!(app.call_name(), "Ada at work", "named by its own account");
        assert!(
            !app.call_live_here(ADA),
            "the account on screen has no call"
        );
        app.apply(Action::AnswerCall, &ctx);
        app.apply(Action::SetCallMuted(true), &ctx);
        let calls = |commands: Vec<Command>| -> Vec<Command> {
            commands
                .into_iter()
                .filter(|command| matches!(command, Command::AnswerCall | Command::SetCallMuted(_)))
                .collect()
        };
        assert!(
            calls(sent(&mut first)).is_empty(),
            "not the account on screen"
        );
        assert!(matches!(
            calls(sent(&mut second)).as_slice(),
            [Command::AnswerCall, Command::SetCallMuted(true)]
        ));
    }

    #[test]
    fn a_second_accounts_offer_is_declined_while_a_call_is_up() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut first) = app(directory.path());
        let mut second = second_account(&mut app);
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Active));
        app.active = 1;
        app.handle_call_update(update(2, CallPhase::Incoming));
        app.active = 0;
        assert!(matches!(
            sent(&mut second).as_slice(),
            [Command::DeclineCall]
        ));
        assert_eq!(app.call.as_ref().map(|call| call.generation), Some(1));
        assert_eq!(app.call_account, Some(AccountId::first()));
        // And no second call is started from the window either.
        app.apply(Action::StartCall(ADA.into()), &ctx);
        assert!(sent(&mut first).is_empty());
    }

    #[test]
    fn removing_the_calls_account_takes_its_surface_away() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        let _second = second_account(&mut app);
        app.active = 1;
        app.handle_call_update(update(1, CallPhase::Active));
        app.active = 0;
        app.forget_call_of(&AccountId::parse("2").unwrap());
        assert!(app.call.is_none());
    }

    /// An update from the worker about a call placed here.
    fn outgoing(generation: u64, phase: CallPhase) -> CallUpdate {
        let mut update = update(generation, phase);
        update.direction = CallDirection::Outgoing;
        update
    }

    /// Presses the call button, and returns the generation and the cancel
    /// flag the worker was handed.
    fn press_call(
        app: &mut App,
        commands: &mut tokio::sync::mpsc::UnboundedReceiver<Command>,
        video: bool,
    ) -> (u64, Arc<AtomicBool>) {
        let ctx = egui::Context::default();
        let action = if video {
            Action::StartVideoCall(ADA.into())
        } else {
            Action::StartCall(ADA.into())
        };
        app.apply(action, &ctx);
        let sent = sent(commands);
        let [
            Command::StartCall {
                chat,
                video: offered,
                generation,
                cancel,
            },
        ] = sent.as_slice()
        else {
            panic!("one StartCall, got {sent:?}");
        };
        assert_eq!(chat.as_str(), ADA);
        assert_eq!(*offered, video);
        (*generation, Arc::clone(cancel))
    }

    #[test]
    fn a_call_shows_connecting_the_moment_its_button_is_pressed() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        let (generation, _) = press_call(&mut app, &mut commands, true);
        // In the same apply, before the worker has said anything.
        let call = app.call.clone().expect("the call is on screen at once");
        assert_eq!(call.phase, CallPhase::Placing);
        assert_eq!(call.generation, generation);
        assert!(call.video);
        assert!(app.call_in_window(), "the call window opens at once");
        assert!(app.call_live_here(ADA), "the header offers to hang up");
        assert_eq!(app.call_name(), "Ada");
        assert_eq!(
            crate::ui::call::status(app.locale, &call),
            "Connecting…".to_owned()
        );
        assert_eq!(app.call_tone(), None, "nothing rings yet");
        // A second press while it is placing does not place a second call.
        app.apply(Action::StartCall(ADA.into()), &ctx);
        assert!(sent(&mut commands).is_empty());

        // The protocol moves it on: offer out, the peer's phone rings, answered.
        app.handle_call_update(outgoing(generation, CallPhase::Dialing));
        assert_eq!(app.call.as_ref().unwrap().phase, CallPhase::Dialing);
        assert_eq!(app.call_tone(), None, "no device has said it rings");
        app.handle_call_update(outgoing(generation, CallPhase::Ringing));
        assert_eq!(app.call_tone(), Some(Tone::Ringback));
        app.tick_call(&ctx);
        assert_eq!(app.call_tone(), Some(Tone::Ringback), "it keeps ringing");
        app.handle_call_update(outgoing(generation, CallPhase::Connecting));
        assert_eq!(app.call_tone(), None, "the answer stops the ringback");
        app.handle_call_update(outgoing(generation, CallPhase::Active));
        assert!(app.call_in_window());

        // The peer hangs up: "Call ended" for about a second and a half.
        let mut over = outgoing(generation, CallPhase::Ended);
        over.outcome = Some(CallOutcome::Answered);
        let before = Instant::now();
        app.handle_call_update(over);
        let until = app.call_surface_until.expect("a farewell");
        let lingers = until.duration_since(before);
        assert!(
            lingers >= CALL_FAREWELL && lingers < Duration::from_millis(1600),
            "{lingers:?}"
        );
        app.tick_call(&ctx);
        assert!(app.call.is_some(), "still saying how it ended");
        app.call_surface_until = Some(Instant::now() - Duration::from_millis(1));
        app.tick_call(&ctx);
        assert!(app.call.is_none(), "and then it goes");
    }

    #[test]
    fn the_peer_declining_stops_the_ringback_and_says_so_briefly() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let (generation, _) = press_call(&mut app, &mut commands, false);
        app.handle_call_update(outgoing(generation, CallPhase::Ringing));
        assert_eq!(app.call_tone(), Some(Tone::Ringback));
        let mut declined = outgoing(generation, CallPhase::Failed);
        declined.outcome = Some(CallOutcome::Declined);
        app.handle_call_update(declined);
        assert_eq!(app.call_tone(), None);
        assert_eq!(
            crate::ui::call::status(app.locale, app.call.as_ref().unwrap()),
            "Declined".to_owned()
        );
        assert!(app.call_surface_until.is_some(), "shown briefly, then gone");
    }

    #[test]
    fn hanging_up_while_connecting_never_rings_the_peer() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        let (generation, cancel) = press_call(&mut app, &mut commands, false);
        app.apply(Action::HangupCall, &ctx);
        assert!(
            cancel.load(Ordering::Relaxed),
            "the worker is told not to offer it"
        );
        assert!(
            sent(&mut commands)
                .iter()
                .any(|command| matches!(command, Command::HangupCall)),
            "and to end it if the offer already went"
        );
        assert!(app.call.is_none(), "the screen goes at once");
        assert!(!app.call_live());
        // Whatever the worker still reports about it is not drawn again.
        app.handle_call_update(outgoing(generation, CallPhase::Dialing));
        assert!(app.call.is_none());
        app.handle_call_update(outgoing(generation, CallPhase::Placing).not_placed());
        assert!(app.call.is_none());
        assert_eq!(app.call_tone(), None);
        // And a new call can be placed straight away.
        let (next, _) = press_call(&mut app, &mut commands, false);
        assert_ne!(next, generation);
        assert_eq!(app.call.as_ref().unwrap().phase, CallPhase::Placing);
    }

    #[test]
    fn hanging_up_a_ringing_call_ends_it_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        let (generation, _) = press_call(&mut app, &mut commands, false);
        app.handle_call_update(outgoing(generation, CallPhase::Ringing));
        app.apply(Action::HangupCall, &ctx);
        assert!(app.call.is_none(), "no farewell for our own hang-up");
        assert_eq!(app.call_tone(), None, "the ringback stops");
        let mut ended = outgoing(generation, CallPhase::Ended);
        ended.outcome = Some(CallOutcome::NoAnswer);
        app.handle_call_update(ended);
        assert!(app.call.is_none());
    }

    #[test]
    fn a_call_that_cannot_be_placed_says_so_and_goes() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        let (generation, _) = press_call(&mut app, &mut commands, false);
        app.handle_call_update(outgoing(generation, CallPhase::Placing).not_placed());
        let call = app.call.clone().unwrap();
        assert_eq!(call.phase, CallPhase::Failed);
        assert_eq!(
            crate::ui::call::status(app.locale, &call),
            "Could not connect".to_owned()
        );
        assert!(!app.call_live(), "nothing is left to hang up");
        assert!(app.call_surface_until.is_some());
        app.call_surface_until = Some(Instant::now() - Duration::from_millis(1));
        app.tick_call(&ctx);
        assert!(app.call.is_none());
    }

    #[test]
    fn a_call_the_worker_never_answers_for_does_not_stay_connecting() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        let (_, cancel) = press_call(&mut app, &mut commands, false);
        app.call_placed = Some(Instant::now() - PLACING_TIMEOUT);
        app.tick_call(&ctx);
        assert_eq!(app.call.as_ref().unwrap().phase, CallPhase::Failed);
        assert!(cancel.load(Ordering::Relaxed), "and it is not placed late");
    }

    #[test]
    fn a_call_to_us_takes_the_place_of_one_still_being_placed() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let (generation, _) = press_call(&mut app, &mut commands, false);
        // The offer reached the worker first, under an older generation (no
        // call is ever given generation 0).
        app.handle_call_update(update(0, CallPhase::Incoming));
        assert_eq!(app.call.as_ref().unwrap().phase, CallPhase::Incoming);
        // The worker refuses the placing call as busy; that is not drawn.
        app.handle_call_update(outgoing(generation, CallPhase::Placing).not_placed());
        assert_eq!(app.call.as_ref().unwrap().phase, CallPhase::Incoming);
    }

    #[test]
    fn a_call_to_us_rings_until_it_is_answered() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.call_tone(), Some(Tone::Ringtone));
        assert_eq!(
            app.notifications.shown[0].sound,
            NotificationSound::None,
            "the ringtone is the sound, not the notification"
        );
        app.handle_call_update(update(1, CallPhase::Accepted));
        assert_eq!(app.call_tone(), None);
    }

    #[test]
    fn declining_a_call_silences_it_and_takes_it_down_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, mut commands) = app(directory.path());
        let ctx = egui::Context::default();
        app.handle_call_update(update(1, CallPhase::Incoming));
        app.apply(Action::DeclineCall, &ctx);
        assert!(
            sent(&mut commands)
                .iter()
                .any(|command| matches!(command, Command::DeclineCall))
        );
        assert!(app.call.is_none());
        assert_eq!(app.call_tone(), None);
        assert_eq!(app.call_notified, None, "its notification goes too");
        app.handle_call_update(ended(1, CallOutcome::Declined));
        assert!(app.call.is_none(), "the worker's own end is not drawn");
    }

    #[test]
    fn a_call_the_caller_gives_up_on_stops_ringing_and_says_missed() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        app.handle_call_update(update(1, CallPhase::Incoming));
        let mut missed = update(1, CallPhase::Failed);
        missed.outcome = Some(CallOutcome::Missed);
        app.handle_call_update(missed);
        assert_eq!(app.call_tone(), None);
        assert!(app.call_surface_until.is_some(), "missed, briefly");
    }

    #[test]
    fn a_call_rings_with_message_sounds_off_but_not_in_a_quiet_chat() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = app(directory.path());
        // The fork's default: messages make no sound. A call still rings.
        app.settings.message_sound = NotificationSound::None;
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.notifications.shown.len(), 1, "announced");
        assert_eq!(app.call_tone(), Some(Tone::Ringtone));

        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = self::app(directory.path());
        app.chats[0].archived = true;
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.call_tone(), None, "an archived chat does not ring");

        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = self::app(directory.path());
        app.account_mut().settings.silenced = true;
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.call_tone(), None, "a silenced account does not ring");

        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = self::app(directory.path());
        app.chats[0].muted_until = Some(crate::util::now() + 3600);
        app.handle_call_update(update(1, CallPhase::Incoming));
        assert_eq!(app.call_tone(), None, "a muted chat does not ring");
    }

    /// The main window in the tray, and a call that rings: the headless loop is woken by the
    /// call's event and asks for the call window alone, which rings with Decline and Accept.
    fn ringing_in_the_tray(
        directory: &std::path::Path,
    ) -> (App, std::sync::mpsc::Sender<crate::backend::Event>) {
        use fastframe_shell::{Headless, Resident};
        let (mut app, _) = app(directory);
        let (backend, _commands, events) = Backend::recording_with_events();
        app.backend = backend;
        let headless = egui::Context::default();
        Resident::window_gone(&mut app);
        assert_eq!(app.headless_frame(&headless), Headless::Wait);
        events
            .send(crate::backend::Event::Call(Box::new(update(
                1,
                CallPhase::Incoming,
            ))))
            .unwrap();
        assert_eq!(app.headless_frame(&headless), Headless::Show);
        assert!(
            app.call_window_alone(),
            "the shell opens the call window, not the main one"
        );
        assert_eq!(app.call_tone(), Some(Tone::Ringtone));
        let view = app.call_window.lock().unwrap().view.clone();
        assert!(view.is_some_and(|view| view.answerable));
        (app, events)
    }

    /// One frame of the standalone call window, drawn or (as a compositor that gives it no
    /// frames leaves it) only its logic: what it decided, whether it asked to close, and when it
    /// asked for its next frame.
    fn standalone_frame(
        window: &mut crate::ui::call::StandaloneWindow,
        app: &mut App,
        ctx: &egui::Context,
        drawn: bool,
    ) -> (crate::ui::call::Standalone, bool, Duration) {
        crate::theme::install(ctx);
        let mut next = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            next = Some(window.logic(app, ui.ctx()));
            if drawn {
                window.drawn();
                crate::ui::call::standalone(ui, app);
            }
        });
        output.textures_delta.clear();
        let root = &output.viewport_output[&egui::ViewportId::ROOT];
        let closes = root.commands.contains(&egui::ViewportCommand::Close);
        (next.unwrap(), closes, root.repaint_delay)
    }

    /// The tray's Show while the call rings in its window alone closes that window for the main
    /// one, drawn or not, and asks for the frame that closes it at once.
    #[test]
    fn the_tray_shows_the_main_window_while_a_call_rings_alone() {
        use crate::ui::call::{Standalone, StandaloneWindow};
        use fastframe_shell::{Closed, Resident};
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) = ringing_in_the_tray(directory.path());
        let ctx = egui::Context::default();
        let mut window = StandaloneWindow::default();
        for drawn in [true, false] {
            let (next, closes, _) = standalone_frame(&mut window, &mut app, &ctx, drawn);
            assert_eq!(next, Standalone::Stay);
            assert!(!closes);
        }
        app.actions.extend(crate::app::tray_action(
            fastframe_tray::Event::Toggle,
            app.window_hidden,
            false,
        ));
        let (next, closes, repaint) = standalone_frame(&mut window, &mut app, &ctx, false);
        assert_eq!(next, Standalone::Main);
        assert!(closes);
        assert_eq!(repaint, Duration::ZERO, "the close goes through at once");
        assert_eq!(app.closed(), Closed::Reopen);
        assert!(
            !app.call_window_alone(),
            "the shell opens the main window next"
        );
        assert!(
            app.call_live(),
            "the call keeps ringing for the main window"
        );
        // The shell closes the call window and opens the main window: the card rings.
        // As `attach` does for the new main window.
        app.window_hidden = false;
        app.reopen = false;
        app.window_focused = true;
        app.sync_call_window(&ctx);
        crate::ui::call::close_window(&mut app.call_window.lock().unwrap());
        app.sync_call_window(&ctx);
        app.tick_call(&ctx);
        assert!(!app.call_in_window(), "on the card");
        // Back to the tray: the call window comes back to answer it.
        app.hide_intent = true;
        assert_eq!(app.closed(), Closed::Hide);
        Resident::window_gone(&mut app);
        app.tick_call(&ctx);
        assert!(
            app.call_window_alone(),
            "a second trip to the tray brings the call window back"
        );
    }

    /// A call window the compositor never draws gives way to the main window, so the call is
    /// never left ringing with nothing on screen to answer it.
    #[test]
    fn a_call_window_never_drawn_gives_way_to_the_main_window() {
        use crate::ui::call::{Standalone, StandaloneWindow};
        use fastframe_shell::{Closed, Resident};
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) = ringing_in_the_tray(directory.path());
        let ctx = egui::Context::default();
        let mut fresh = StandaloneWindow::default();
        let (next, _, _) = standalone_frame(&mut fresh, &mut app, &ctx, false);
        assert_eq!(next, Standalone::Stay, "a new window has time to be drawn");
        let mut drawn = StandaloneWindow::opened_at(Instant::now() - Duration::from_secs(10));
        drawn.drawn();
        let (next, _, _) = standalone_frame(&mut drawn, &mut app, &ctx, false);
        assert_eq!(next, Standalone::Stay, "one that was drawn stays");
        let mut undrawn = StandaloneWindow::opened_at(Instant::now() - Duration::from_secs(10));
        let (next, closes, repaint) = standalone_frame(&mut undrawn, &mut app, &ctx, false);
        assert_eq!(next, Standalone::Main);
        assert!(closes);
        assert_eq!(repaint, Duration::ZERO);
        assert_eq!(app.closed(), Closed::Reopen);
        assert!(!app.call_window_alone());
    }

    /// Answered in the call window alone, the call stays there; once it ends there, the window
    /// closes at once and ZapFast goes back to the tray.
    #[test]
    fn a_call_answered_alone_stays_in_its_window_and_ends_in_the_tray() {
        use crate::ui::call::{Standalone, StandaloneWindow};
        use fastframe_shell::{Closed, Headless, Resident};
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) = ringing_in_the_tray(directory.path());
        let ctx = egui::Context::default();
        let mut window = StandaloneWindow::default();
        events
            .send(crate::backend::Event::Call(Box::new(update(
                1,
                CallPhase::Active,
            ))))
            .unwrap();
        let (next, _, _) = standalone_frame(&mut window, &mut app, &ctx, true);
        assert_eq!(
            next,
            Standalone::Stay,
            "the answered call stays in its window"
        );
        assert!(app.call_in_window());
        events
            .send(crate::backend::Event::Call(Box::new(ended(
                1,
                CallOutcome::Answered,
            ))))
            .unwrap();
        let (next, _, _) = standalone_frame(&mut window, &mut app, &ctx, true);
        assert_eq!(next, Standalone::Stay, "the farewell is shown there");
        app.call_surface_until = Some(Instant::now() - Duration::from_secs(1));
        let (next, closes, repaint) = standalone_frame(&mut window, &mut app, &ctx, false);
        assert_eq!(next, Standalone::Hide);
        assert!(closes);
        assert_eq!(repaint, Duration::ZERO);
        assert_eq!(app.closed(), Closed::Hide);
        Resident::window_gone(&mut app);
        assert_eq!(
            app.headless_frame(&egui::Context::default()),
            Headless::Wait
        );
    }

    /// A call that stops ringing, answered on the phone or given up, takes its window with it.
    #[test]
    fn a_call_resolved_elsewhere_closes_its_window_alone() {
        use crate::ui::call::{Standalone, StandaloneWindow};
        use fastframe_shell::{Closed, Resident};
        let directory = tempfile::tempdir().unwrap();
        let (mut app, events) = ringing_in_the_tray(directory.path());
        let ctx = egui::Context::default();
        let mut window = StandaloneWindow::default();
        events
            .send(crate::backend::Event::Call(Box::new(ended(
                1,
                CallOutcome::AnsweredElsewhere,
            ))))
            .unwrap();
        standalone_frame(&mut window, &mut app, &ctx, true);
        assert_eq!(app.call_tone(), None, "the ringing stops");
        // Once its farewell is over.
        app.call_surface_until = Some(Instant::now() - Duration::from_secs(1));
        let (next, closes, repaint) = standalone_frame(&mut window, &mut app, &ctx, false);
        assert!(closes);
        assert_eq!(repaint, Duration::ZERO);
        assert_eq!(next, Standalone::Hide);
        assert_eq!(app.closed(), Closed::Hide);
    }

    /// The main window open, with a tray, and a call ringing on its card.
    fn ringing_in_front(directory: &std::path::Path) -> App {
        let (mut app, _) = app(directory);
        app.test_tray_shown = Some(true);
        app.window_hidden = false;
        app.window_focused = true;
        app.handle_call_update(update(1, CallPhase::Incoming));
        app.tick_call(&egui::Context::default());
        assert!(app.call_live() && !app.call_in_window(), "on the card");
        app
    }

    /// From the tray to the call window alone, as the shell does once the main window is gone.
    fn hidden_to_the_call_window(app: &mut App) {
        use fastframe_shell::{Closed, Headless, Resident};
        assert_eq!(app.closed(), Closed::Hide);
        Resident::window_gone(app);
        assert_eq!(
            app.headless_frame(&egui::Context::default()),
            Headless::Show
        );
        assert!(app.call_window_alone());
    }

    /// The tray's Toggle while a call rings on the main window used to only minimize it, where
    /// eframe draws nothing: no call window, and every later Toggle minimized it again until the
    /// call stopped ringing. It now goes to the tray, and the call to its window alone.
    #[test]
    fn the_tray_hides_the_main_window_during_a_call_and_the_call_rings_alone() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = ringing_in_front(directory.path());
        let ctx = egui::Context::default();
        let action =
            crate::app::tray_action(fastframe_tray::Event::Toggle, app.window_hidden, true);
        assert!(matches!(action, Some(Action::HideWindow)));
        app.apply(action.unwrap(), &ctx);
        assert!(app.hide_intent, "to the tray, not minimized");
        hidden_to_the_call_window(&mut app);
    }

    /// Closing the main window while a call rings sends it to the tray too.
    #[test]
    fn closing_the_main_window_during_a_call_hides_it_to_the_tray() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = ringing_in_front(directory.path());
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        let mut output = ctx.run_ui(input, |ui| app.window_logic(ui.ctx()));
        output.textures_delta.clear();
        let root = &output.viewport_output[&egui::ViewportId::ROOT];
        assert!(!root.commands.contains(&egui::ViewportCommand::CancelClose));
        hidden_to_the_call_window(&mut app);
    }

    /// A main window that stops being drawn (minimized) while the call is in its window hands
    /// the call to the call window alone; without a call, or without a tray, it stays.
    #[test]
    fn a_main_window_no_longer_drawn_hands_its_call_to_the_tray() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let (mut app, _) = app(directory.path());
        app.test_tray_shown = Some(true);
        app.window_hidden = false;
        app.main_window_undrawn(&ctx);
        assert!(!app.hide_intent, "no call, nothing to hand over");

        let mut app = ringing_in_front(directory.path());
        app.window_focused = false;
        app.tick_call(&ctx);
        assert!(app.call_in_window());
        app.test_tray_shown = Some(false);
        app.main_window_undrawn(&ctx);
        assert!(
            !app.hide_intent,
            "without a tray nothing would bring it back"
        );
        app.test_tray_shown = Some(true);
        // A call window that was never drawn gave the call back: the main
        // window keeps it rather than bouncing it between the two.
        app.call_window_gave_way = true;
        app.main_window_undrawn(&ctx);
        assert!(!app.hide_intent);
        app.call_window_gave_way = false;
        app.main_window_undrawn(&ctx);
        assert!(app.hide_intent);
        hidden_to_the_call_window(&mut app);
    }
}
