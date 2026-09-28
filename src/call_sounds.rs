//! The tones a 1:1 call plays: the ringtone of an incoming call, the ringback
//! while ours rings, and a short sound when it connects, ends or finds the
//! peer busy.
//!
//! The sounds are Telegram Desktop's (GPL-3.0, see `assets/sounds/README.md`).
//! The ringtone plays through the default speaker, as a phone rings out loud;
//! the rest go to the call's own speaker, where the call is heard.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rodio::Source;

use crate::calls::{CallOutcome, CallPhase, CallUpdate};
use crate::model::CallDirection;

const INCOMING: &[u8] = include_bytes!("../assets/sounds/call_incoming.mp3");
const OUTGOING: &[u8] = include_bytes!("../assets/sounds/call_outgoing.mp3");
const CONNECT: &[u8] = include_bytes!("../assets/sounds/call_connect.mp3");
const END: &[u8] = include_bytes!("../assets/sounds/call_end.mp3");
const BUSY: &[u8] = include_bytes!("../assets/sounds/call_busy.mp3");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Someone is calling; repeats until the call is answered or gone.
    Incoming,
    /// Our call is going out or ringing; repeats until it is answered.
    Outgoing,
    /// The call connected.
    Connect,
    /// The call ended, whoever hung up.
    End,
    /// The peer was busy or declined.
    Busy,
}

impl Tone {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Incoming => INCOMING,
            Self::Outgoing => OUTGOING,
            Self::Connect => CONNECT,
            Self::End => END,
            Self::Busy => BUSY,
        }
    }

    fn repeats(self) -> bool {
        matches!(self, Self::Incoming | Self::Outgoing)
    }
}

/// What a call update does to the sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// Whatever plays goes on.
    Keep,
    /// Silence.
    Stop,
    /// This tone, in place of whatever plays.
    Play(Tone),
}

/// The cue for a call moving from `previous` (the same call's last phase, or
/// `None` for a call not seen before) to `update`.
pub fn cue(previous: Option<CallPhase>, update: &CallUpdate) -> Cue {
    let ringing_out = |phase: CallPhase| matches!(phase, CallPhase::Dialing | CallPhase::Ringing);
    match update.phase {
        CallPhase::Incoming if previous != Some(CallPhase::Incoming) => Cue::Play(Tone::Incoming),
        phase if ringing_out(phase) && !previous.is_some_and(ringing_out) => {
            Cue::Play(Tone::Outgoing)
        }
        CallPhase::Connecting | CallPhase::Accepted => Cue::Stop,
        CallPhase::Active if previous != Some(CallPhase::Active) => Cue::Play(Tone::Connect),
        CallPhase::Ended | CallPhase::Failed => {
            // A call first seen already over was never heard here.
            if !previous.is_some_and(CallPhase::is_live) {
                return Cue::Keep;
            }
            match update.outcome {
                Some(CallOutcome::Busy) => Cue::Play(Tone::Busy),
                Some(CallOutcome::Declined) if update.direction == CallDirection::Outgoing => {
                    Cue::Play(Tone::Busy)
                }
                // A call that rang here and was taken or left elsewhere just
                // stops ringing.
                Some(
                    CallOutcome::Missed
                    | CallOutcome::AnsweredElsewhere
                    | CallOutcome::DeclinedElsewhere,
                ) => Cue::Stop,
                _ => Cue::Play(Tone::End),
            }
        }
        _ => Cue::Keep,
    }
}

/// Plays one tone at a time, each on its own thread.
#[derive(Default)]
pub struct CallSounds {
    enabled: bool,
    playing: Option<(Tone, Arc<AtomicBool>)>,
}

impl CallSounds {
    /// Lets tones play; tests and demos leave them off.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Whether tones play.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Follows a call update. `ring` is false when the chat stays quiet (muted,
    /// archived or locked), which silences the ringtone only. `speaker` is the
    /// call's speaker, `None` for the default.
    pub fn follow(
        &mut self,
        previous: Option<CallPhase>,
        update: &CallUpdate,
        ring: bool,
        speaker: Option<&str>,
    ) {
        match cue(previous, update) {
            Cue::Keep => {}
            Cue::Stop => self.stop(),
            Cue::Play(Tone::Incoming) if !ring => self.stop(),
            Cue::Play(tone) => {
                let speaker = (tone != Tone::Incoming).then_some(speaker).flatten();
                self.play(tone, speaker);
            }
        }
    }

    /// Stops the tone that plays, if any.
    pub fn stop(&mut self) {
        if let Some((_, stop)) = self.playing.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    fn play(&mut self, tone: Tone, speaker: Option<&str>) {
        self.stop();
        if !self.enabled {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let speaker = speaker.map(str::to_owned);
        let spawned = std::thread::Builder::new()
            .name("call-sound".to_owned())
            .spawn(move || {
                if let Err(error) = sound(tone, speaker.as_deref(), &flag) {
                    log::debug!("call sound {tone:?} not played: {error}");
                }
            });
        match spawned {
            Ok(_) => self.playing = Some((tone, stop)),
            Err(error) => log::debug!("no thread for a call sound: {error}"),
        }
    }
}

impl Drop for CallSounds {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Plays `tone` until it ends, or until `stop` for one that repeats.
fn sound(tone: Tone, speaker: Option<&str>, stop: &AtomicBool) -> Result<(), String> {
    // A speaker that went away leaves the tone on the default one.
    let device = crate::call_audio::open_output(speaker).or_else(|error| {
        if speaker.is_some() {
            log::debug!("call sound on the default speaker: {error}");
            crate::call_audio::open_output(None)
        } else {
            Err(error)
        }
    })?;
    let player = rodio::Player::connect_new(device.mixer());
    let decoder = rodio::Decoder::new(std::io::Cursor::new(tone.bytes()))
        .map_err(|error| error.to_string())?;
    if tone.repeats() {
        player.append(decoder.buffered().repeat_infinite());
    } else {
        player.append(decoder);
    }
    while !stop.load(Ordering::Relaxed) && !player.empty() {
        std::thread::sleep(Duration::from_millis(30));
    }
    player.stop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(
        direction: CallDirection,
        phase: CallPhase,
        outcome: Option<CallOutcome>,
    ) -> CallUpdate {
        CallUpdate {
            generation: 1,
            chat: "1@s.whatsapp.net".to_owned(),
            direction,
            phase,
            started: None,
            muted: false,
            outcome,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
        }
    }

    #[test]
    fn an_incoming_call_rings_until_it_connects_and_ends_with_a_sound() {
        use CallPhase::*;
        let incoming = CallDirection::Incoming;
        assert_eq!(
            cue(None, &update(incoming, Incoming, None)),
            Cue::Play(Tone::Incoming)
        );
        assert_eq!(
            cue(Some(Incoming), &update(incoming, Incoming, None)),
            Cue::Keep
        );
        assert_eq!(
            cue(Some(Incoming), &update(incoming, Accepted, None)),
            Cue::Stop
        );
        assert_eq!(
            cue(Some(Accepted), &update(incoming, Active, None)),
            Cue::Play(Tone::Connect)
        );
        assert_eq!(
            cue(Some(Active), &update(incoming, Active, None)),
            Cue::Keep
        );
        assert_eq!(
            cue(
                Some(Active),
                &update(incoming, Ended, Some(CallOutcome::Answered))
            ),
            Cue::Play(Tone::End)
        );
    }

    #[test]
    fn an_outgoing_call_rings_back_and_a_busy_peer_sounds_busy() {
        use CallPhase::*;
        let outgoing = CallDirection::Outgoing;
        assert_eq!(
            cue(None, &update(outgoing, Dialing, None)),
            Cue::Play(Tone::Outgoing)
        );
        assert_eq!(
            cue(Some(Dialing), &update(outgoing, Ringing, None)),
            Cue::Keep
        );
        assert_eq!(
            cue(Some(Ringing), &update(outgoing, Connecting, None)),
            Cue::Stop
        );
        for outcome in [CallOutcome::Busy, CallOutcome::Declined] {
            assert_eq!(
                cue(Some(Ringing), &update(outgoing, Ended, Some(outcome))),
                Cue::Play(Tone::Busy)
            );
        }
        assert_eq!(
            cue(
                Some(Ringing),
                &update(outgoing, Ended, Some(CallOutcome::NoAnswer))
            ),
            Cue::Play(Tone::End)
        );
    }

    #[test]
    fn a_call_taken_elsewhere_just_stops_ringing() {
        use CallPhase::*;
        let incoming = CallDirection::Incoming;
        for outcome in [
            CallOutcome::Missed,
            CallOutcome::AnsweredElsewhere,
            CallOutcome::DeclinedElsewhere,
        ] {
            assert_eq!(
                cue(Some(Incoming), &update(incoming, Ended, Some(outcome))),
                Cue::Stop
            );
        }
        // Declining it here ends it with the ending sound.
        assert_eq!(
            cue(
                Some(Incoming),
                &update(incoming, Ended, Some(CallOutcome::Declined))
            ),
            Cue::Play(Tone::End)
        );
        // A call first seen over was never heard.
        assert_eq!(
            cue(None, &update(incoming, Ended, Some(CallOutcome::Missed))),
            Cue::Keep
        );
    }

    #[test]
    fn every_tone_decodes() {
        for tone in [
            Tone::Incoming,
            Tone::Outgoing,
            Tone::Connect,
            Tone::End,
            Tone::Busy,
        ] {
            let decoder = rodio::Decoder::new(std::io::Cursor::new(tone.bytes())).unwrap();
            assert!(decoder.count() > 1000, "{tone:?}");
        }
    }
}
