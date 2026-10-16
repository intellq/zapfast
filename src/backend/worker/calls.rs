//! The worker's side of a 1:1 call: commands, signaling routing, and media events.
//!
//! One call at a time. [`Worker::call`] holds the only one, and every entry point here refuses to
//! start or accept a second while it exists — a double click, a second chat's call button, and a
//! second inbound offer all land on the same refusal.
//!
//! The state the UI renders is never computed here: it comes from the backend. `<accept>` moves an
//! outgoing call out of dialing, the media plane's `RelayAllocated` is what makes a call Active and
//! starts the timer, and a `<reject>`/`<terminate>`/relay loss ends it with the reason the peer
//! gave. Nothing is inferred from the button that was pressed.

use super::*;
use crate::calls::{self, Call, CallUpdate, VideoTick};
use whatsapp_rust::types::call::IncomingCall;
use whatsapp_rust::voip::{CallEvent, KeyframeUrgency};

/// The call this worker owns: its state, and the channels the tasks around it feed.
///
/// Dropped the moment the call reaches a terminal phase, which is what releases the media tasks,
/// the microphone and speaker streams and the camera thread.
pub(super) struct CallRuntime {
    pub(super) call: Call,
    /// Media-plane events, and the signal that the media task finished.
    pub(super) events: async_channel::Receiver<CallRuntimeEvent>,
    /// Video frames for the UI.
    pub(super) frames: Option<async_channel::Receiver<VideoTick>>,
    /// Whether the media-plane watcher was started for this call's handle.
    watching: bool,
    /// The snapshot the UI was last handed, so the periodic check publishes only real changes.
    last: CallUpdate,
}

/// What a call's background tasks report back to the worker loop.
pub(super) enum CallRuntimeEvent {
    /// One event off the call's media plane.
    Media(CallEvent),
    /// The media task is gone: relay disconnect, send failure, or hangup.
    Ended,
}

impl CallRuntime {
    /// A runtime whose media plane is not watched yet: a call that is still ringing has no handle
    /// to watch, and gets one the moment it is placed or answered.
    pub(super) fn new(call: Call, frames: Option<async_channel::Receiver<VideoTick>>) -> Self {
        let (_tx, events) = async_channel::bounded(64);
        let last = call.update();
        Self {
            call,
            events,
            frames,
            watching: false,
            last,
        }
    }

    /// Starts forwarding this call's media events and its media-finished signal, once.
    pub(super) fn watch(&mut self) {
        if self.watching {
            return;
        }
        let Some(handle) = self.call.handle() else {
            return;
        };
        self.watching = true;
        let (tx, events) = async_channel::bounded(64);
        self.events = events;
        // The handle's event queue has one reader, and this is it.
        let Some(mut media) = handle.take_events() else {
            log::warn!("[CALL] the call's events were already taken");
            return;
        };
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = media.recv() => match event {
                        Ok(event) => {
                            if tx.send(CallRuntimeEvent::Media(event)).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => {
                            let _ = tx.send(CallRuntimeEvent::Ended).await;
                            return;
                        }
                    },
                    _ = handle.wait_ended() => {
                        let _ = tx.send(CallRuntimeEvent::Ended).await;
                        return;
                    }
                }
            }
        });
    }
}

impl Worker {
    // -----------------------------------------------------------------------
    // Command entry points
    // -----------------------------------------------------------------------

    /// Hands the call screen the microphones, speakers and cameras it can offer, and keeps them:
    /// they are how a device that goes away is named by the description the user saw.
    pub(super) async fn emit_call_devices(&mut self) -> calls::DeviceList {
        let devices = discover_devices().await;
        self.publish_call_devices(devices.clone());
        devices
    }

    /// Stores and publishes a device list, so the pickers and the fallback naming share one
    /// snapshot of the machine.
    pub(super) fn publish_call_devices(&mut self, devices: calls::DeviceList) {
        self.call_devices = devices.clone();
        self.emit(Event::CallDevices(Box::new(devices)));
    }

    /// Re-sends the current call after privacy recovery, since call updates are withheld while the
    /// lock state is unknown.
    ///
    /// Nothing is rediscovered: the last published list is what the picker already had, and the
    /// runtime's own snapshot is what the surface should draw. The chat can finally be judged
    /// private or not, so a call that arrived mid-recovery rings behind the right lock state rather
    /// than being dropped for the whole recovery.
    pub(super) fn replay_call(&mut self) {
        let Some(runtime) = self.call.as_ref() else {
            return;
        };
        let devices = self.call_devices.clone();
        let update = runtime.last.clone();
        self.publish_call_devices(devices);
        self.emit_call(update);
    }

    /// Publishes one call state, and lets the call go once it reaches a terminal phase.
    ///
    /// The UI keeps rendering the snapshot it was handed; what is released here is the media, so a
    /// finished call holds no child process and no task.
    pub(super) fn emit_call(&mut self, update: CallUpdate) {
        let finished = !update.phase.is_live();
        // Remembered so the periodic check can tell a real change from a snapshot it already sent.
        if let Some(runtime) = self.call.as_mut()
            && runtime.call.generation() == update.generation
        {
            runtime.last = update.clone();
        }
        self.emit(Event::Call(Box::new(update.clone())));
        if finished && let Some(runtime) = self.call.take() {
            self.file_finished_call(&runtime.call, &update);
        }
    }

    /// Whether a call is already up, which every entry point refuses to double.
    pub(super) fn call_busy(&mut self) -> bool {
        if self.call.is_some() {
            log::warn!("[CALL] refusing a new call: one is already up");
            return true;
        }
        false
    }

    pub(super) async fn start_call(&mut self, chat: ChatId, video: bool) {
        if self.call_busy() {
            return;
        }
        if !callable_chat(&chat) {
            log::warn!("[CALL] refusing a call to a chat that is not one to one");
            self.emit(Event::Error("Calls are one to one only".to_owned()));
            return;
        }
        if self.is_blocked(&chat) {
            log::info!("[CALL] refusing a call to a blocked contact");
            self.emit(Event::Error(
                crate::i18n::tr("Unblock this contact to call them").to_owned(),
            ));
            return;
        }
        let Some(client) = self.client.clone() else {
            log::warn!("[CALL] cannot start a call: WhatsApp is not connected");
            self.emit(Event::Error(
                "ZapFast is not connected to WhatsApp".to_owned(),
            ));
            return;
        };
        // A call opens on the devices last picked, or the system's defaults when they are gone;
        // the call screen's pickers move it to others while it runs.
        let devices = self.emit_call_devices().await;
        let resolved = calls::resolve_devices(
            &devices,
            self.call_defaults.microphone.clone(),
            self.call_defaults.speaker.clone(),
            None,
        );
        let camera = video
            .then(|| calls::default_camera(&devices, None))
            .flatten();
        match Call::place(
            &client,
            chat.to_string(),
            video,
            resolved.microphone,
            resolved.speaker,
            camera,
        )
        .await
        {
            Ok((mut call, frames)) => {
                call.set_lost_devices(resolved.lost_devices);
                let mut runtime = CallRuntime::new(call, frames);
                runtime.watch();
                let update = runtime.call.update();
                self.call = Some(runtime);
                self.emit_call(update);
            }
            Err(error) => {
                log::error!("[CALL] could not start the call: {error}");
                self.emit(Event::Error(format!(
                    "The call could not be started: {error}"
                )));
            }
        }
    }

    pub(super) async fn answer_call(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let update = match self.call.as_mut() {
            Some(runtime) => {
                // Whatever the user picked while the phone was ringing is what the media binds to,
                // checked against the machine so one that vanished falls back rather than opening a
                // stream that can never deliver.
                let (microphone, speaker, camera) = runtime.call.selections();
                let resolved =
                    calls::resolve_devices(&discover_devices().await, microphone, speaker, camera);
                let answered = runtime
                    .call
                    .answer(
                        &client,
                        resolved.microphone,
                        resolved.speaker,
                        resolved.camera,
                    )
                    .await;
                match answered {
                    Ok(frames) => {
                        if frames.is_some() {
                            runtime.frames = frames;
                        }
                        runtime.call.set_lost_devices(resolved.lost_devices);
                        runtime.watch();
                        Some(runtime.call.update())
                    }
                    Err(error) => {
                        log::error!("[CALL] could not answer the call: {error}");
                        let update = runtime.call.abandon(&client).await;
                        self.emit(Event::Error(format!(
                            "The call could not be answered: {error}"
                        )));
                        Some(update)
                    }
                }
            }
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn decline_call(&mut self) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let update = match self.call.as_mut() {
            Some(runtime) => match runtime.call.decline(&client).await {
                Ok(()) => Some(runtime.call.update()),
                Err(error) => {
                    log::error!("[CALL] could not decline the call: {error}");
                    return;
                }
            },
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn hangup_call(&mut self) {
        let client = self.client.clone();
        let update = match self.call.as_mut() {
            Some(runtime) => Some(runtime.call.hangup(client.as_ref()).await),
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn set_call_muted(&mut self, muted: bool) {
        let update = match self.call.as_mut() {
            Some(runtime) => runtime.call.set_muted(muted).await,
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn set_call_camera(&mut self, on: bool) {
        let outcome = match self.call.as_mut() {
            Some(runtime) => {
                if on && !runtime.call.is_video() {
                    // A voice call: ask the peer for video, or answer the peer who asked, and start
                    // draining the frames it brings, from the first camera the machine reports.
                    let camera = calls::default_camera(&self.call_devices, None);
                    match runtime.call.upgrade_to_video(camera).await {
                        Ok(frames) => {
                            if frames.is_some() {
                                runtime.frames = frames;
                            }
                            Ok(runtime.call.update())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    runtime.call.set_camera(on).await
                }
            }
            None => return,
        };
        match outcome {
            Ok(update) => self.emit_call(update),
            Err(error) => {
                log::error!("[CALL] the camera could not be toggled: {error}");
                self.emit(Event::Error(error.to_string()));
            }
        }
    }

    pub(super) fn set_call_microphone(&mut self, device: Option<String>) {
        let update = self
            .call
            .as_mut()
            .map(|runtime| runtime.call.set_microphone(device));
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) fn set_call_speaker(&mut self, device: Option<String>) {
        let update = self
            .call
            .as_mut()
            .map(|runtime| runtime.call.set_speaker(device));
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    pub(super) async fn set_call_camera_device(&mut self, device: Option<String>) {
        // `None` is the picker's "First camera": a camera has no system default, so it is read as
        // the first camera the machine reports, the same reading a call's startup applies.
        let device = calls::default_camera(&self.call_devices, device);
        let outcome = match self.call.as_mut() {
            Some(runtime) => Some(runtime.call.set_camera_device(device).await),
            None => None,
        };
        match outcome {
            Some(Ok(update)) => self.emit_call(update),
            Some(Err(error)) => {
                log::error!("[CALL] the camera could not be switched: {error}");
                self.emit(Event::Error(error.to_string()));
            }
            None => {}
        }
    }

    // -----------------------------------------------------------------------
    // Backend events
    // -----------------------------------------------------------------------

    /// One event from the current call's media plane.
    pub(super) fn call_runtime(&mut self, event: CallRuntimeEvent) {
        let update = match self.call.as_mut() {
            Some(runtime) => match event {
                CallRuntimeEvent::Ended => runtime.call.media_ended(),
                CallRuntimeEvent::Media(media) => {
                    if matches!(media, CallEvent::RelayAllocated)
                        && runtime.call.is_video()
                        && let Some(handle) = runtime.call.handle()
                    {
                        // A decoder cannot start on a delta frame, so ask for an IDR at once.
                        handle.request_peer_keyframe(KeyframeUrgency::Immediate);
                    }
                    runtime.call.media(&media)
                }
            },
            None => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    /// One video frame from the current call, straight to the UI.
    pub(super) fn call_frame(&mut self, tick: VideoTick) {
        match tick {
            VideoTick::Lost => {
                if let Some(handle) = self.call.as_ref().and_then(|runtime| runtime.call.handle()) {
                    handle.request_peer_keyframe(KeyframeUrgency::Coalesced);
                }
            }
            VideoTick::Local(image) => {
                self.emit(Event::CallVideo {
                    local: Some(image),
                    remote: None,
                });
            }
            VideoTick::Remote(image) => {
                let first = self
                    .call
                    .as_mut()
                    .is_some_and(|runtime| runtime.call.saw_remote_video());
                // The flag is what the UI gates the picture on, so a frame that arrives after the
                // peer said their video stopped is not drawn: it is a picture of a stream that has
                // ended, and showing it would contradict the state the peer just sent.
                let still_sending = self
                    .call
                    .as_ref()
                    .is_none_or(|runtime| runtime.call.peer_video_visible());
                if still_sending {
                    self.emit(Event::CallVideo {
                        local: None,
                        remote: Some(image),
                    });
                }
                if first && let Some(runtime) = self.call.as_ref() {
                    let update = runtime.call.update();
                    self.emit(Event::Call(Box::new(update)));
                }
            }
        }
    }

    /// Routes one inbound `<call>` stanza: either it belongs to the call we own, or it is a new
    /// offer that should ring.
    ///
    /// This is where an outgoing call learns it was answered, rejected, or ended, so the phase the
    /// UI shows is the peer's, not the dialer's.
    pub(super) async fn call_signaling(&mut self, incoming: &IncomingCall) {
        let action = &incoming.action;
        let mine = self
            .call
            .as_ref()
            .is_some_and(|runtime| runtime.call.call_id() == action.call_id());
        if mine {
            let update = self
                .call
                .as_mut()
                .and_then(|runtime| runtime.call.signaling(action));
            if let Some(update) = update {
                self.emit_call(update);
            }
            return;
        }
        if !calls::is_offer(action) {
            return;
        }
        // The offer carries the caller's number beside their privacy id, which the library has
        // learned by now, so a stranger's call lands in the chat named by that number.
        let chat = self.canonical_sync_chat(&incoming.from).await;
        if self.call_busy() {
            // Refuse it to the caller rather than dropping it. An offer that is only dropped leaves
            // the other side ringing until its own protocol timeout, believing this device might
            // still answer; a real `<reject>` ends it there and then. The call that is already up
            // is untouched: this is the second offer's own call id being refused, not ours.
            match self.client.as_ref() {
                Some(client) => match client.voip().reject(incoming).await {
                    Ok(()) => log::info!(
                        "[CALL] refused a second offer while one is up call_id={}",
                        action.call_id()
                    ),
                    Err(error) => {
                        log::warn!("[CALL] a second offer could not be refused: {error}")
                    }
                },
                None => log::warn!("[CALL] no client to refuse a second offer with"),
            }
            return;
        }
        // A group, broadcast or newsletter offer never reaches the 1:1 call surface. The interface
        // hides the buttons for those chats, but the worker is the boundary: a non-direct offer is
        // refused here rather than rung, so no call is created and no history entry is written.
        //
        // Refuse it to the caller instead of only dropping it: a dropped offer leaves the other
        // side ringing until its own protocol timeout, believing this device might still answer,
        // while a real `<reject>` ends it there and then. Only a 1:1 offer rings, so an offer that
        // reaches here is one the caller is told we will not take.
        if !callable_chat(&chat) {
            match self.client.as_ref() {
                Some(client) => match client.voip().reject(incoming).await {
                    Ok(()) => log::warn!(
                        "[CALL] refused an incoming offer from a chat that is not one to one"
                    ),
                    Err(error) => {
                        log::warn!("[CALL] a non-direct offer could not be refused: {error}")
                    }
                },
                None => log::warn!("[CALL] no client to refuse a non-direct offer with"),
            }
            return;
        }
        let video = calls::offer_is_video(action);
        let call = Call::ringing(chat, Box::new(incoming.clone()), video)
            .with_devices(self.call_defaults.clone());
        let update = call.update();
        self.call = Some(CallRuntime::new(call, None));
        self.emit_call_devices().await;
        self.emit_call(update);
    }

    /// A call we were ringing for was resolved on another of the account's devices (`elsewhere`
    /// says how), or the caller gave up before anyone answered (`None`).
    pub(super) fn call_resolved(&mut self, call_id: &str, elsewhere: Option<calls::CallOutcome>) {
        let update = match self.call.as_mut() {
            Some(runtime) if runtime.call.call_id() == call_id => {
                runtime.call.resolved_elsewhere(elsewhere)
            }
            _ => None,
        };
        if let Some(update) = update {
            self.emit_call(update);
        }
    }

    /// Releases the call on the way out, so quitting leaves no child process or task behind.
    pub(super) async fn shutdown_call(&mut self) {
        if let Some(mut runtime) = self.call.take() {
            let update = runtime.call.hangup(self.client.as_ref()).await;
            // The window lets the call go as it lets any other finished one.
            self.emit(Event::Call(Box::new(update.clone())));
            self.file_finished_call(&runtime.call, &update);
        }
    }

    /// Keeps a live call's picture of the machine honest.
    ///
    /// A device the user picked can disappear mid-call — a Bluetooth headset switching off, a camera
    /// unplugged, a sound card taken over by another app — and nothing here ends the call over it.
    /// The stream behind it is rebound to the system default, a camera that stopped is stopped for
    /// the peer as well, and the snapshot carries the reason, so the picker stops claiming a device
    /// that is not there.
    ///
    /// The fresh list is also published when it changed, so a device that came or went shows up in
    /// the pickers without anyone pressing anything.
    pub(super) async fn reconcile_call(&mut self) {
        if self
            .call
            .as_ref()
            .is_none_or(|runtime| !runtime.call.phase().is_live())
        {
            return;
        }
        // Once per heartbeat, the engine's own counters for this call. A call that is up but carrying
        // no audio is otherwise indistinguishable in a bug report from one where nobody spoke, and
        // these numbers are what tell the two apart.
        if let Some(runtime) = self.call.as_ref() {
            runtime.call.log_media_stats();
        }
        // Discovery opens every audio device and asks every camera to name itself, so it runs on a
        // blocking thread rather than on the worker's async loop: a slow device must not hold up
        // message handling or the call's own events.
        let devices = discover_devices().await;
        // Read before the fresh list replaces it: a device that just went away is still named in
        // here by the description the user saw.
        let known = self.call_devices.clone();
        if devices != known {
            self.call_devices = devices.clone();
            self.emit(Event::CallDevices(Box::new(devices.clone())));
        }
        let mut lost = Vec::new();
        if let Some(runtime) = self.call.as_mut() {
            lost.extend(runtime.call.take_stream_fallbacks());
            lost.extend(runtime.call.verify_devices(&devices).await);
            if runtime.call.camera_stalled()
                && let Err(error) = runtime.call.set_camera(false).await
            {
                log::warn!("[CALL] the peer was not told the camera stopped: {error}");
            }
        }
        if !lost.is_empty() {
            // Worded by the call screen, which knows the reader's language; the log says it plainly.
            let lost: Vec<calls::LostDevice> = lost
                .into_iter()
                .map(|(kind, id)| {
                    let name = calls::device_name(&known, kind, &id);
                    log::warn!("[CALL] {kind:?} \"{name}\" is not available any more");
                    calls::LostDevice { kind, name }
                })
                .collect();
            if let Some(runtime) = self.call.as_mut() {
                runtime.call.set_lost_devices(lost);
            }
        }
        // Compared against the snapshot the UI was handed, so a camera that stopped on its own or a
        // mute another device set reaches the screen even though no user pressed anything.
        let changed = self.call.as_mut().and_then(|runtime| {
            let current = runtime.call.update();
            let changed = current != runtime.last;
            runtime.last = current.clone();
            changed.then_some(current)
        });
        if let Some(update) = changed {
            self.emit_call(update);
        }
    }
}

/// A call to file among a chat's messages.
pub(super) struct CallEntry {
    pub chat: ChatId,
    pub call_id: String,
    pub from_me: bool,
    pub timestamp: i64,
    pub video: bool,
    pub status: calls::CallOutcome,
    pub duration: u64,
    pub group: bool,
}

impl Worker {
    /// Files a call among its chat's messages, as WhatsApp shows calls, unless
    /// the chat already has an entry for it (the phone's own call-log
    /// message). The entry is local: an id of ZapFast's own, never sent, never
    /// unread.
    pub(super) fn file_call_entry(&mut self, entry: CallEntry) {
        if entry.chat.is_empty() || entry.call_id.is_empty() || entry.timestamp <= 0 {
            return;
        }
        let id = format!("{}{}", crate::model::CALL_ENTRY_PREFIX, entry.call_id);
        match self
            .archive
            .call_entry_near(&entry.chat, entry.timestamp, entry.from_me, &id)
        {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                log::warn!("[CALL] the chat's call entries could not be read: {error}");
                return;
            }
        }
        self.ensure_chat(&entry.chat, None);
        let sender = if entry.from_me {
            self.me()
        } else {
            entry.chat.clone()
        };
        let message = Message {
            id,
            chat: entry.chat.clone(),
            sender,
            sender_name: None,
            from_me: entry.from_me,
            timestamp: entry.timestamp,
            history_order: None,
            content: Content::Call {
                video: entry.video,
                status: entry.status,
                duration: entry.duration,
                group: entry.group,
            },
            status: Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        };
        if let Err(error) = self.archive.insert_message(&message, None) {
            log::warn!("[CALL] a call entry could not be filed: {error}");
            return;
        }
        self.emit(Event::Messages {
            chat: entry.chat.clone(),
            messages: vec![message],
            older: false,
            complete: false,
        });
        self.emit_chat(&entry.chat);
    }

    /// Removes ZapFast's own entries for a call once the phone's own message
    /// for it is filed, so the chat shows the call once.
    pub(super) fn retire_local_call_twins(&mut self, message: &Message) {
        if !matches!(message.content, Content::Call { .. })
            || message.id.starts_with(crate::model::CALL_ENTRY_PREFIX)
        {
            return;
        }
        let twins =
            match self
                .archive
                .local_call_twins(&message.chat, message.timestamp, message.from_me)
            {
                Ok(twins) => twins,
                Err(error) => {
                    log::warn!("[CALL] the chat's call entries could not be read: {error}");
                    return;
                }
            };
        for id in twins {
            match self.archive.delete_message(&message.chat, &id) {
                Ok(_) => self.emit(Event::MessageDeleted {
                    chat: message.chat.clone(),
                    id,
                }),
                Err(error) => log::warn!("[CALL] a duplicate call entry stays: {error}"),
            }
        }
    }

    /// Files a call this worker took part in once it is over.
    fn file_finished_call(&mut self, call: &Call, update: &CallUpdate) {
        let Some(status) = update.outcome else {
            return;
        };
        self.file_call_entry(CallEntry {
            chat: update.chat.clone(),
            call_id: call.call_id().to_owned(),
            from_me: update.direction == crate::model::CallDirection::Outgoing,
            timestamp: call.began(),
            video: call.carried_video(),
            status,
            duration: update
                .started
                .map_or(0, |started| started.elapsed().as_secs()),
            group: false,
        });
    }

    /// Files a call the phone reports as missed while this device was away.
    pub(super) fn missed_call_entry(&mut self, call: &whatsapp_rust::types::call::MissedCall) {
        let chat = self.canonical(&call.from);
        self.file_call_entry(CallEntry {
            chat,
            call_id: call.call_id.clone(),
            from_me: false,
            timestamp: call.timestamp.timestamp(),
            video: false,
            status: calls::CallOutcome::Missed,
            duration: 0,
            group: false,
        });
    }

    /// Files a call from the phone's call log: synced as it happens, or sent
    /// with history when this device was linked.
    pub(super) fn call_log_entry(
        &mut self,
        call_id: &str,
        creator: Option<&Jid>,
        from_me: Option<bool>,
        record: &wa::CallLogRecord,
    ) {
        let Some(from_me) = from_me else {
            return;
        };
        let Some(status) = record_status(record, from_me) else {
            return;
        };
        let group = record
            .group_jid
            .as_deref()
            .and_then(|jid| jid.parse::<Jid>().ok());
        let chat = match (&group, from_me) {
            (Some(group), _) => self.canonical(group),
            (None, false) => match creator {
                Some(creator) => self.canonical(&creator.to_non_ad()),
                None => return,
            },
            // Our own call: the other side is its participant.
            (None, true) => match record
                .participants
                .iter()
                .filter_map(|participant| participant.user_jid.as_deref())
                .filter_map(|jid| jid.parse::<Jid>().ok())
                .next()
            {
                Some(peer) => self.canonical(&peer.to_non_ad()),
                None => return,
            },
        };
        self.file_call_entry(CallEntry {
            chat,
            call_id: call_id.to_owned(),
            from_me,
            timestamp: record.start_time.unwrap_or(0),
            video: record.is_video.unwrap_or(false),
            status,
            duration: record.duration.unwrap_or(0).max(0) as u64,
            group: group.is_some(),
        });
    }

    /// Hands the Calls page this account's calls.
    pub(super) fn emit_call_log(&mut self) {
        match self.archive.call_log(crate::archive::CALL_LOG_LIMIT) {
            Ok(entries) => self.emit(Event::CallLog(entries)),
            Err(error) => log::warn!("[CALL] the call log could not be read: {error}"),
        }
    }
}

/// What a call-log record says became of a call; `None` for a call that has
/// not happened yet or is still going on.
fn record_status(record: &wa::CallLogRecord, from_me: bool) -> Option<calls::CallOutcome> {
    use calls::CallOutcome as Status;
    use wa::call_log_record::CallResult;
    Some(match record.call_result? {
        CallResult::CONNECTED => Status::Answered,
        CallResult::REJECTED => Status::Declined,
        CallResult::ACCEPTEDELSEWHERE => Status::AnsweredElsewhere,
        CallResult::MISSED => Status::Missed,
        CallResult::CANCELLED | CallResult::ABANDONED | CallResult::UNAVAILABLE => {
            if from_me {
                Status::NoAnswer
            } else {
                Status::Missed
            }
        }
        CallResult::INVALID | CallResult::FAILED => Status::Failed,
        CallResult::UPCOMING | CallResult::ONGOING => return None,
        _ => return None,
    })
}

/// A call-log message in a chat, as WhatsApp files a call there.
pub(super) fn call_log_content(log: &wa::message::CallLogMessage) -> Content {
    use calls::CallOutcome as Status;
    use wa::message::call_log_message::CallOutcome;
    let status = match log.call_outcome {
        Some(CallOutcome::CONNECTED) | None => Status::Answered,
        Some(CallOutcome::REJECTED) => Status::Declined,
        Some(CallOutcome::ACCEPTED_ELSEWHERE) => Status::AnsweredElsewhere,
        Some(CallOutcome::FAILED) => Status::Failed,
        Some(CallOutcome::ONGOING) => Status::Answered,
        // Missed, silenced while Do Not Disturb was on or for an unknown
        // caller: either way nobody answered.
        _ => Status::Missed,
    };
    Content::Call {
        video: log.is_video.unwrap_or(false),
        status,
        duration: log.duration_secs.unwrap_or(0).max(0) as u64,
        group: log.participants.len() > 1,
    }
}

/// A missed call the phone filed as a bare notice in a chat's history.
pub(super) fn missed_call_stub(stub: wa::web_message_info::StubType) -> Option<Content> {
    use wa::web_message_info::StubType;
    let (video, group) = match stub {
        StubType::CALL_MISSED_VOICE => (false, false),
        StubType::CALL_MISSED_VIDEO => (true, false),
        StubType::CALL_MISSED_GROUP_VOICE => (false, true),
        StubType::CALL_MISSED_GROUP_VIDEO => (true, true),
        _ => return None,
    };
    Some(Content::Call {
        video,
        status: calls::CallOutcome::Missed,
        duration: 0,
        group,
    })
}

/// Device discovery off the worker thread: enumerating opens every audio device to name it, so it
/// runs on a blocking thread rather than on the async loop, where a slow device would hold up
/// message handling and the call's own events.
async fn discover_devices() -> calls::DeviceList {
    tokio::task::spawn_blocking(calls::devices)
        .await
        .unwrap_or_default()
}

/// Whether a chat can be called at all.
///
/// The interface only draws the phone and camera buttons on a one-to-one chat, but the worker is the
/// boundary rather than the interface: a group, a channel or a broadcast list reaching the 1:1
/// builder would be refused by the protocol at best and misbehave at worst, so the JID is checked
/// here as well.
fn callable_chat(chat: &str) -> bool {
    matches!(
        crate::model::ChatKind::from_id(chat),
        crate::model::ChatKind::Direct
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_one_to_one_chat_can_be_called() {
        assert!(callable_chat("15551234567@s.whatsapp.net"));
        assert!(callable_chat("123456789012345@lid"));
        assert!(!callable_chat("12345-67890@g.us"));
        assert!(!callable_chat("1234567890@broadcast"));
        assert!(!callable_chat("1234567890@newsletter"));
    }

    #[test]
    fn the_phones_call_log_says_how_each_call_ended() {
        use calls::CallOutcome as Status;
        use wa::call_log_record::CallResult;
        let record = |result| {
            let mut record = wa::CallLogRecord::default();
            record.call_result = result;
            record
        };
        assert_eq!(
            record_status(&record(Some(CallResult::CONNECTED)), true),
            Some(Status::Answered)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::REJECTED)), false),
            Some(Status::Declined)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::ACCEPTEDELSEWHERE)), false),
            Some(Status::AnsweredElsewhere)
        );
        // A call nobody picked up: unanswered when we placed it, missed when
        // it rang here.
        assert_eq!(
            record_status(&record(Some(CallResult::CANCELLED)), true),
            Some(Status::NoAnswer)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::CANCELLED)), false),
            Some(Status::Missed)
        );
        assert_eq!(
            record_status(&record(Some(CallResult::FAILED)), true),
            Some(Status::Failed)
        );
        // Calls still to come or still going on are not filed yet.
        assert_eq!(
            record_status(&record(Some(CallResult::ONGOING)), true),
            None
        );
        assert_eq!(record_status(&record(None), true), None);
    }

    #[test]
    fn a_call_log_message_reads_as_a_call() {
        use wa::message::call_log_message::CallOutcome;
        let mut log = wa::message::CallLogMessage::default();
        log.is_video = Some(true);
        log.call_outcome = Some(CallOutcome::CONNECTED);
        log.duration_secs = Some(754);
        assert!(matches!(
            call_log_content(&log),
            Content::Call {
                video: true,
                status: calls::CallOutcome::Answered,
                duration: 754,
                group: false,
            }
        ));
        let mut missed = wa::message::CallLogMessage::default();
        missed.call_outcome = Some(CallOutcome::MISSED);
        assert!(matches!(
            call_log_content(&missed),
            Content::Call {
                video: false,
                status: calls::CallOutcome::Missed,
                duration: 0,
                ..
            }
        ));
    }

    #[test]
    fn only_missed_call_notices_become_calls() {
        use wa::web_message_info::StubType;
        assert!(matches!(
            missed_call_stub(StubType::CALL_MISSED_GROUP_VIDEO),
            Some(Content::Call {
                video: true,
                group: true,
                status: calls::CallOutcome::Missed,
                ..
            })
        ));
        assert!(matches!(
            missed_call_stub(StubType::CALL_MISSED_VOICE),
            Some(Content::Call {
                video: false,
                group: false,
                ..
            })
        ));
        assert!(missed_call_stub(StubType::GROUP_CREATE).is_none());
    }
}
