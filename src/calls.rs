//! 1:1 WhatsApp calling: signaling, media, audio devices, camera, and state.
//!
//! The protocol, the encryption and the media engine are whatsapp-rust's (`Client::voip()`); this
//! module keeps the state the interface draws. Audio goes through rodio, the layer the rest of the
//! app plays and records with, so a call opens the platform's own audio API (PipeWire or ALSA,
//! CoreAudio, WASAPI). The engine's 16 kHz mono frames meet whatever rate and channel count a device
//! wants inside [`crate::call_audio`], and each direction sits behind a *stable* channel, so
//! changing the input or output device replaces only the stream behind it without the engine ever
//! seeing a port close.
//!
//! Video is H.264 Annex-B both ways, because that is what the library transports: it never touches
//! pixels. Capture encodes with the `openh264` encoder the app already links, and the peer's access
//! units decode back to `egui` images with the `openh264` decoder `crate::video` already uses.
//! Frames cross to the UI through [`VideoTick`]; no decode work happens on the UI thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use egui::ColorImage;

use crate::model::CallDirection;
use whatsapp_rust::prelude::{Client, Jid};
use whatsapp_rust::types::call::{CallAction, IncomingCall};
use whatsapp_rust::voip::{
    CallEvent, CallHandle, TimedVideoFrame, VideoFrame, VideoSink, VideoSource,
};

/// The pixel budget one camera session encodes within: a landscape frame at most 1280 by 720, a
/// portrait one at most 720 by 1280.
///
/// Following the camera's own aspect is what keeps a portrait camera portrait on the peer's screen
/// and in the corner preview. Stretching it into one fixed landscape frame would either pad it into
/// a letterbox or squash it, and neither is what the camera saw. The budget is 720p because that is
/// what the peer's screen is: a smaller frame is scaled up twice (once by this side's bitrate and
/// again by the phone) and reads as the soft, blocky picture a 640 by 360 call is.
const CAPTURE_LONG_SIDE: usize = 1280;
const CAPTURE_SHORT_SIDE: usize = 720;
/// The largest the corner preview may be, in the same orientation as the capture. The preview is a
/// thumbnail rather than the call, so it stays the size of one and costs nothing to draw.
const PREVIEW_LONG_SIDE: usize = 320;
const PREVIEW_SHORT_SIDE: usize = 180;
/// The cadence the camera is asked for and the encoder is built for: what WhatsApp's own clients
/// send at most. A camera that grants fewer frames is used at what it gives, and the frames carry
/// their own capture times, so nothing plays back at the wrong speed.
const VIDEO_FPS: u32 = 20;
/// RTP video clock (90 kHz) divided by the capture cadence.
const VIDEO_TS_STRIDE: u32 = 90_000 / VIDEO_FPS;
/// The encoder's bitrate for the budget above, about what WhatsApp's own clients use for a 720p
/// call, and low enough to fit the uplink a call shares with its own audio.
const VIDEO_BITRATE: u32 = 1_500_000;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Where a 1:1 call is in its life.
///
/// Every value except the terminal ones comes from the backend: the offer that was sent or
/// received, the peer's `<accept>`, and the media plane reporting itself live. Nothing is inferred
/// from the button that started the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallPhase {
    /// Our offer went out and nobody has answered.
    Dialing,
    /// The peer's device is ringing: its `<preaccept>` arrived. Nobody has answered yet.
    Ringing,
    /// Someone else's offer is ringing and waits for the user.
    Incoming,
    /// The peer answered and the media plane is coming up.
    Connecting,
    /// We answered a ringing call and the media plane is coming up.
    ///
    /// Kept apart from [`CallPhase::Connecting`] because the two are not the same claim: there the
    /// peer's `<accept>` arrived, here the user picked up and the media is still being negotiated.
    Accepted,
    /// Media is up. The duration runs from this point, not from the button press.
    Active,
    /// The call is over. [`CallUpdate::outcome`] says how.
    Ended,
    /// The call never came up. [`CallUpdate::outcome`] says why.
    Failed,
}

impl CallPhase {
    /// Whether the call is still doing something; the UI keeps its surface for these.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Dialing
                | Self::Ringing
                | Self::Incoming
                | Self::Connecting
                | Self::Accepted
                | Self::Active
        )
    }

    /// Whether the microphone, camera and device pickers apply.
    pub fn is_connected(self) -> bool {
        matches!(self, Self::Connecting | Self::Accepted | Self::Active)
    }

    /// Whether the two sides are talking, or are about to: the phases the peer is on the line for.
    pub fn is_answered(self) -> bool {
        matches!(self, Self::Connecting | Self::Accepted | Self::Active)
    }
}

/// How a call turned out: the status a call entry keeps, as data rather than as a finished
/// sentence. The words belong to the call screen and the history, which know the reader's
/// language; each value comes from the peer's signaling or from the media plane, never from the
/// button that was pressed.
pub use crate::model::CallStatus as CallOutcome;

/// Why the peer's audio is not becoming sound, as the engine names the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SilenceReason {
    /// This build has no decoder for the codec the call settled on.
    NoDecoder,
    /// The peer's media is arriving but does not authenticate.
    AuthenticationFailing,
    /// Packets arrive that are not the payload type the call negotiated.
    UnexpectedPayloadType,
    /// The decoder keeps refusing frames.
    CodecRejectingFrames,
    /// The decode path keeps changing its mind about the payload grammar.
    CodecFlapping,
    /// Something else; the counters are the only detail there is.
    Unknown,
}

/// What the engine reports about the peer's audio when it is not arriving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerAudio {
    /// Audio is arriving and none of it is becoming sound.
    Silent(SilenceReason),
    /// No audio is arriving at all, which is a transport problem rather than a codec one.
    Stalled,
}

/// One snapshot of the live call, or of the call that just ended.
#[derive(Clone, Debug, PartialEq)]
pub struct CallUpdate {
    /// Changes with every call, so the UI can tell a stale update from the current one.
    pub generation: u64,
    /// The chat the call belongs to, as an archive id.
    pub chat: String,
    /// Which side placed the call.
    pub direction: CallDirection,
    /// Whether this call carries video.
    pub video: bool,
    pub phase: CallPhase,
    /// Set when the call became active; the timer counts from here.
    pub started: Option<Instant>,
    pub muted: bool,
    pub camera_on: bool,
    /// Whether the peer's picture has started arriving.
    pub remote_video: bool,
    /// Whether the peer asked to turn a voice call into a video call and waits for an answer.
    pub video_requested: bool,
    /// How the call ended, once it has. Worded by the call screen, stored by the call history.
    pub outcome: Option<CallOutcome>,
    /// Set when the engine says the peer's audio is missing, so a silent call says why instead of
    /// just being silent.
    pub peer_audio: Option<PeerAudio>,
    /// Devices the user picked that the machine no longer has, so the call screen can name them
    /// instead of quietly using another one.
    pub lost_devices: Vec<LostDevice>,
    /// The selected input, output and camera nodes; `None` means the system default.
    pub microphone: Option<String>,
    pub speaker: Option<String>,
    pub camera: Option<String>,
}

/// A microphone or speaker the platform reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    /// The name the platform reports and accepts back when a stream is opened.
    pub id: String,
    /// What the picker shows.
    pub label: String,
}

/// The devices a call opens with: the microphone and speaker last picked, which the settings keep
/// from one call to the next. `None` is the system default.
#[derive(Clone, Debug, Default)]
pub struct CallDevices {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
}

/// A camera the platform reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraDevice {
    /// What [`crate::camera::Source::open`] takes back: an index, or the platform's own id.
    pub id: String,
    /// The name the platform reports.
    pub label: String,
}

/// The devices the call screen offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceList {
    pub microphones: Vec<AudioDevice>,
    pub speakers: Vec<AudioDevice>,
    pub cameras: Vec<CameraDevice>,
}

/// Which of the three kinds of call device a selection belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Microphone,
    Speaker,
    Camera,
}

/// What a set of selections resolved to against the machine's own list.
#[derive(Clone, Debug, Default)]
pub struct ResolvedDevices {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
    pub camera: Option<String>,
    /// Whatever had to be given up, so the call screen can say so in the reader's language.
    pub lost_devices: Vec<LostDevice>,
}

/// A device the call had to give up because the machine no longer has it.
///
/// Kept as data rather than as a finished sentence: the words belong to the call screen, which is
/// the only place that knows the reader's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostDevice {
    pub kind: DeviceKind,
    /// The friendliest name at hand: what the picker showed, or the node itself.
    pub name: String,
}

/// The friendliest name for `id` in `list`: the description the picker showed, or the node itself.
pub fn device_name(list: &DeviceList, kind: DeviceKind, id: &str) -> String {
    let label = match kind {
        DeviceKind::Microphone => list
            .microphones
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.label.clone()),
        DeviceKind::Speaker => list
            .speakers
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.label.clone()),
        DeviceKind::Camera => list
            .cameras
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.label.clone()),
    };
    label.unwrap_or_else(|| id.to_owned())
}

/// Checks selections against what the machine has, falling back to the system default.
///
/// A selection is kept when its list is empty: that means device discovery could not run, not that
/// nothing is plugged in, and dropping the choice over a broken tool would be a lie. The returned
/// note names what moved, so a call that fell back says so instead of quietly using another device.
pub fn resolve_devices(
    list: &DeviceList,
    microphone: Option<String>,
    speaker: Option<String>,
    camera: Option<String>,
) -> ResolvedDevices {
    let microphones: Vec<&str> = list.microphones.iter().map(|d| d.id.as_str()).collect();
    let speakers: Vec<&str> = list.speakers.iter().map(|d| d.id.as_str()).collect();
    let cameras: Vec<&str> = list.cameras.iter().map(|d| d.id.as_str()).collect();
    let mut lost: Vec<LostDevice> = Vec::new();

    let checked =
        |kind: DeviceKind, wanted: Option<String>, known: &[&str], lost: &mut Vec<LostDevice>| {
            match wanted {
                // Nothing is known about a list that came back empty, so the choice stands.
                Some(id) if known.is_empty() => Some(id),
                Some(id) if known.contains(&id.as_str()) => Some(id),
                Some(id) => {
                    lost.push(LostDevice {
                        kind,
                        name: device_name(list, kind, &id),
                    });
                    None
                }
                None => None,
            }
        };
    let microphone = checked(DeviceKind::Microphone, microphone, &microphones, &mut lost);
    let speaker = checked(DeviceKind::Speaker, speaker, &speakers, &mut lost);
    // A camera has no system default the way an audio device does, so the picker's "First camera"
    // has to mean the first camera the machine reports. Without this a fresh install — whose stored
    // camera is `None` — could not start or answer a video call at all, and could not even reach the
    // in-call picker to choose a node. A named camera the machine no longer has falls back the same
    // way, and the fallback is reported, so the screen can say the old one is gone.
    let camera = checked(DeviceKind::Camera, camera, &cameras, &mut lost)
        .or_else(|| default_camera(list, None));

    ResolvedDevices {
        microphone,
        speaker,
        camera,
        lost_devices: lost,
    }
}

/// Reads the picker's "First camera" as a camera that exists.
///
/// A camera has no system default the way a sound card does, so `None` means the first camera the
/// machine reports — the same reading [`resolve_devices`] applies when a call starts. A camera the
/// machine no longer has falls back the same way. `None` comes back only when nothing is named and
/// nothing is listed, and then the caller's own failure path says so rather than this inventing one.
///
/// Split out of [`resolve_devices`] so the two paths that change a camera mid-call — upgrading a
/// voice call to video, and picking a device while one is running — read the default exactly as a
/// call's startup does, and so that reading is testable without a machine.
pub fn default_camera(list: &DeviceList, camera: Option<String>) -> Option<String> {
    let named = camera.filter(|id| !id.is_empty());
    match named {
        // A camera the machine still reports is kept, even when the list is empty: an empty list
        // means discovery could not run, not that nothing is plugged in.
        Some(id) if list.cameras.is_empty() || list.cameras.iter().any(|c| c.id == id) => Some(id),
        // A camera the machine no longer has, or no choice at all: the first one it does have.
        _ => list.cameras.first().map(|camera| camera.id.clone()),
    }
}

/// The corner preview's size for a capture of `capture`, in the capture's own orientation.
fn preview_size(capture: (usize, usize)) -> (usize, usize) {
    let bounds = if capture.1 > capture.0 {
        (PREVIEW_SHORT_SIDE, PREVIEW_LONG_SIDE)
    } else {
        (PREVIEW_LONG_SIDE, PREVIEW_SHORT_SIDE)
    };
    crate::camera::fit_even(capture, bounds)
}

/// Whether an offer announces video.
pub fn offer_is_video(action: &CallAction) -> bool {
    match action {
        CallAction::Offer { is_video, .. } => *is_video,
        _ => false,
    }
}

/// Whether a stanza is an offer, which is the only one that may start ringing.
pub fn is_offer(action: &CallAction) -> bool {
    matches!(action, CallAction::Offer { .. })
}

// ---------------------------------------------------------------------------
// Device discovery
// ---------------------------------------------------------------------------

/// Lists microphones, speakers and cameras for the call screen.
///
/// They come from rodio, the same layer a call opens them through, so a picker offers what a call
/// can actually take, on every platform: PipeWire or ALSA, CoreAudio, WASAPI.
pub fn devices() -> DeviceList {
    DeviceList {
        microphones: audio_devices(crate::call_audio::microphones()),
        speakers: audio_devices(crate::call_audio::speakers()),
        cameras: cameras(),
    }
}

/// Wraps the names rodio reports as the call screen's device entries.
fn audio_devices(found: Vec<(String, String)>) -> Vec<AudioDevice> {
    found
        .into_iter()
        .map(|(id, label)| AudioDevice { id, label })
        .collect()
}

/// Lists the cameras the platform reports, through the camera module.
fn cameras() -> Vec<CameraDevice> {
    crate::camera::cameras()
        .into_iter()
        .map(|(id, label)| CameraDevice { id, label })
        .collect()
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

/// The engine's audio clock, and the microphone and the speaker the engine talks to.
///
/// Both live in [`crate::call_audio`], which opens them through rodio the way the rest of the app
/// already plays and records. A device change replaces only the reader or the writer behind the
/// engine's channel, so nothing about the call's codec or stream state is rebuilt for it.
pub use crate::call_audio::{AudioInput, AudioOutput, RATE};

/// What a call still cannot open on this machine, if anything.
///
/// Read against the devices this call would really open rather than the system defaults alone: a
/// machine whose default microphone is busy still takes a call aimed at one the user picked, so
/// the check follows the same selections the streams will.
fn missing_audio_device(microphone: Option<&str>, speaker: Option<&str>) -> Option<&'static str> {
    crate::call_audio::unavailable(microphone, speaker)
}

/// The same check, run where it cannot stall the caller.
///
/// `missing_audio_device` opens every device to name it, which is a blocking enumeration. Both
/// `Call::place` and `Call::answer` are awaited on the worker's own async loop, and the worker
/// already moves its other device discovery to `spawn_blocking` for exactly this reason: a slow or
/// wedged audio stack would otherwise hold up message handling and signaling while a call starts.
/// The microphone and speaker names are cloned in, so nothing borrowed crosses the thread.
async fn missing_audio_device_off_thread(
    microphone: Option<String>,
    speaker: Option<String>,
) -> Option<&'static str> {
    tokio::task::spawn_blocking(move || {
        missing_audio_device(microphone.as_deref(), speaker.as_deref())
    })
    .await
    // A panicked check is not a machine without devices: refusing the call over it would be a lie,
    // so it is treated as "nothing missing" and the ordinary open path reports what it finds.
    .unwrap_or(None)
}

/// Whether a requested video path really came up.
///
/// The user pressed the video button, or the peer offered video; connecting with a media type
/// nobody chose and not saying so is worse than refusing. The check runs before any signaling, so a
/// camera or encoder that will not start fails the call instead of quietly downgrading it.
fn require_video(requested: bool, pipeline: bool) -> Result<()> {
    if requested && !pipeline {
        return Err(anyhow!(
            "video could not be started; the camera or its encoder is not available"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------

/// One frame on its way to the UI.
#[derive(Clone, Debug)]
pub enum VideoTick {
    /// Our own camera, for the corner preview.
    Local(Arc<ColorImage>),
    /// The peer's picture.
    Remote(Arc<ColorImage>),
    /// The peer's picture could not be decoded: the peer is asked for a keyframe.
    Lost,
}

/// The camera side of the call: the camera module delivers YUV 4:2:0 at a fixed size, so the
/// encoder and the preview both have a known layout, and the frames the engine wants are complete
/// H.264 Annex-B access units led by an access-unit delimiter.
///
/// A device change starts a fresh thread rather than telling this one to switch: a running encoder
/// carries state the new device's stream cannot inherit.
struct CameraCapture {
    timed: async_channel::Receiver<TimedVideoFrame>,
    /// Cleared when the camera is no longer wanted, and read by the capture loop between frames.
    running: Arc<AtomicBool>,
    /// Set when the engine needs a keyframe; the next frame is encoded as one.
    keyframe: Arc<AtomicBool>,
    /// Set by the capture thread as it leaves, so `stop` can wait for the device to be released
    /// before another camera opens it, without waiting forever on a driver that hangs.
    finished: std::sync::mpsc::Receiver<()>,
}

/// How long a camera is given to open and build its encoder before the video direction gives up.
///
/// The wait is what keeps a busy camera or an encoder that will not start from signaling a video
/// call with no outgoing picture. It covers macOS's permission prompt the first time a camera is
/// used, which the capture thread waits on.
const CAMERA_STARTUP: std::time::Duration = std::time::Duration::from_secs(20);
/// How long a stopping camera is waited for before its thread is left to finish on its own.
const CAMERA_RELEASE: std::time::Duration = std::time::Duration::from_secs(2);

impl CameraCapture {
    /// Opens `device` (the first camera when `None`) on a thread of its own and waits until it
    /// delivers, so a call never signals video it cannot send. Blocking: call it off the async loop.
    fn start(device: Option<String>, ticks: async_channel::Sender<VideoTick>) -> Result<Self> {
        let (frames, timed) = async_channel::bounded::<TimedVideoFrame>(4);
        let running = Arc::new(AtomicBool::new(true));
        let keyframe = Arc::new(AtomicBool::new(false));
        let (ready, came_up) = std::sync::mpsc::channel::<Result<(), String>>();
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let alive = CaptureAlive(Arc::clone(&running));
        let stopping = Arc::clone(&running);
        let forced = Arc::clone(&keyframe);
        std::thread::Builder::new()
            .name("zapfast-camera".to_owned())
            .spawn(move || {
                let _alive = alive;
                capture(device, frames, ticks, &stopping, &forced, ready);
                let _ = done.send(());
            })
            .context("camera thread could not be started")?;
        let capture = Self {
            timed,
            running,
            keyframe,
            finished,
        };
        match came_up.recv_timeout(CAMERA_STARTUP) {
            Ok(Ok(())) => Ok(capture),
            Ok(Err(error)) => Err(anyhow!(error)),
            Err(_) => Err(anyhow!("the camera did not start in time")),
        }
    }

    /// Ends the camera: the loop sees the cleared flag after the frame it is waiting for, and the
    /// device is released before this returns, unless the driver hangs past [`CAMERA_RELEASE`].
    fn stop(&mut self) {
        if self.running.swap(false, Ordering::Relaxed) {
            let _ = self.finished.recv_timeout(CAMERA_RELEASE);
        }
    }

    fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Clears a capture's running flag on the way out, including when the thread panics.
struct CaptureAlive(Arc<AtomicBool>);

impl Drop for CaptureAlive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// The `VideoSource` the engine pulls access units from.
struct LocalVideoSource {
    timed: async_channel::Receiver<TimedVideoFrame>,
    /// Never fed: the engine uses the timestamped channel above.
    empty: async_channel::Receiver<Vec<u8>>,
}

impl VideoSource for LocalVideoSource {
    fn frames(&self) -> async_channel::Receiver<Vec<u8>> {
        self.empty.clone()
    }

    fn timed_frames(&self) -> Option<async_channel::Receiver<TimedVideoFrame>> {
        Some(self.timed.clone())
    }

    fn rtp_timestamp_stride(&self) -> u32 {
        VIDEO_TS_STRIDE
    }
}

/// The `VideoSink` the engine hands the peer's access units to.
struct RemoteVideoSink {
    tx: async_channel::Sender<VideoFrame>,
}

impl VideoSink for RemoteVideoSink {
    fn playout(&self) -> async_channel::Sender<VideoFrame> {
        self.tx.clone()
    }
}

/// Everything the video direction owns, plus what a device change or camera toggle needs.
pub struct VideoPipeline {
    ticks: async_channel::Sender<VideoTick>,
    remote_tx: async_channel::Sender<VideoFrame>,
    camera: Option<CameraCapture>,
    device: Option<String>,
}

impl VideoPipeline {
    /// Starts the camera and the peer's decoder. The returned receiver carries both directions'
    /// frames to the UI. Waits for the camera, so it runs on a blocking thread.
    pub async fn start(
        device: Option<String>,
    ) -> Result<(Self, async_channel::Receiver<VideoTick>)> {
        tokio::task::spawn_blocking(move || Self::start_blocking(device))
            .await
            .map_err(|error| anyhow!("the camera thread failed: {error}"))?
    }

    fn start_blocking(
        device: Option<String>,
    ) -> Result<(Self, async_channel::Receiver<VideoTick>)> {
        let (ticks, tick_rx) = async_channel::bounded::<VideoTick>(2);
        let (remote_tx, remote_rx) = async_channel::bounded::<VideoFrame>(8);
        decode_remote(remote_rx, ticks.clone());
        // A machine without a camera, or one whose camera is busy, still takes part in a video
        // call: it sees the other side, its own camera reads as off, and the camera button says
        // why when pressed.
        let camera = match CameraCapture::start(device.clone(), ticks.clone()) {
            Ok(camera) => Some(camera),
            Err(error) => {
                log::warn!("[CALL] the video call goes on without our camera: {error}");
                None
            }
        };
        Ok((
            Self {
                ticks,
                remote_tx,
                camera,
                device,
            },
            tick_rx,
        ))
    }

    fn source(&self) -> LocalVideoSource {
        let (empty_tx, empty) = async_channel::bounded::<Vec<u8>>(1);
        drop(empty_tx);
        let timed = match &self.camera {
            Some(camera) => camera.timed.clone(),
            // No camera running: a closed timestamped channel, so the feed simply sees no frames.
            None => {
                let (unused, closed) = async_channel::bounded::<TimedVideoFrame>(1);
                drop(unused);
                closed
            }
        };
        LocalVideoSource { timed, empty }
    }

    fn sink(&self) -> RemoteVideoSink {
        RemoteVideoSink {
            tx: self.remote_tx.clone(),
        }
    }

    /// Releases the camera. A resume builds a new source, so nothing here is kept.
    pub fn pause_camera(&mut self) {
        if let Some(mut camera) = self.camera.take() {
            camera.stop();
        }
    }

    /// Starts capturing from `device` again, reusing the frame channel the UI already drains.
    /// Waits for the camera, so it runs on a blocking thread.
    pub async fn resume_camera(&mut self, device: Option<String>) -> Result<()> {
        let previous = self.camera.take();
        self.device = device.clone();
        let ticks = self.ticks.clone();
        // The old camera is released first, on the same blocking thread, so the new one never
        // races it for the device.
        let camera = tokio::task::spawn_blocking(move || {
            drop(previous);
            CameraCapture::start(device, ticks)
        })
        .await
        .map_err(|error| anyhow!("the camera thread failed: {error}"))??;
        self.camera = Some(camera);
        Ok(())
    }

    /// Whether the camera is capturing right now. A capture thread that ended (an unplugged
    /// camera, one another program took) reads as off, so the frame channel and the controls agree.
    pub fn camera_running(&self) -> bool {
        self.camera.as_ref().is_some_and(CameraCapture::running)
    }

    /// Asks the encoder for a keyframe, which the engine needs whenever the peer's decoder has
    /// lost track (a lost packet, a resumed camera).
    pub fn force_keyframe(&self) {
        if let Some(camera) = &self.camera {
            camera.keyframe.store(true, Ordering::Relaxed);
        }
    }

    /// Ends the video direction: the capture thread stops and the device is released.
    pub fn shutdown(&mut self) {
        if let Some(mut camera) = self.camera.take() {
            camera.stop();
        }
    }
}

/// Captures from the camera, encodes each frame to an Annex-B access unit, and feeds the preview.
///
/// Runs on its own thread: the platforms' capture objects are not all `Send`, the camera read is
/// blocking, and the `openh264` encoder is not shared. Everything it hands out goes through a
/// bounded channel with `try_send`, so a stalled consumer drops a frame instead of stalling the
/// camera.
fn capture(
    device: Option<String>,
    timed: async_channel::Sender<TimedVideoFrame>,
    ticks: async_channel::Sender<VideoTick>,
    stopping: &AtomicBool,
    keyframe: &AtomicBool,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    // The camera's own shape decides the frame the encoder runs at, within the budget either way
    // round, so a portrait camera is encoded portrait.
    let budget = (CAPTURE_LONG_SIDE, CAPTURE_SHORT_SIDE);
    let mut source = match crate::camera::Source::open(device.as_deref(), budget, VIDEO_FPS) {
        Ok(source) => source,
        Err(error) => {
            log::error!("[CALL] camera capture could not start: {error}");
            let _ = ready.send(Err(error));
            return;
        }
    };
    let size = source.size();
    let mut encoder = match video_encoder() {
        Ok(encoder) => encoder,
        Err(error) => {
            log::error!("[CALL] camera encoder could not start: {error}");
            source.stop();
            ticks.close();
            let _ = ready.send(Err(format!("the camera encoder could not start: {error}")));
            return;
        }
    };
    // The camera and the encoder are both up: only now is a video call worth signaling.
    let _ = ready.send(Ok(()));
    capture_frames(
        &mut source,
        size,
        stopping,
        keyframe,
        &timed,
        &ticks,
        &mut encoder,
        Instant::now(),
    );
    source.stop();
}

/// The H.264 encoder a call's camera feeds: Constrained Baseline, which is what WhatsApp's clients
/// decode, with a keyframe every two seconds so a peer that lost one recovers on its own.
fn video_encoder() -> std::result::Result<openh264::encoder::Encoder, openh264::Error> {
    use openh264::encoder::{
        BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile,
    };
    let config = EncoderConfig::new()
        .profile(Profile::Baseline)
        .bitrate(BitRate::from_bps(VIDEO_BITRATE))
        .max_frame_rate(FrameRate::from_hz(VIDEO_FPS as f32))
        .intra_frame_period(IntraFramePeriod::from_num_frames(VIDEO_FPS * 2))
        .skip_frames(false);
    Encoder::with_api_config(openh264::OpenH264API::from_source(), config)
}

/// Reads frames from one camera until the call stops it, encoding and previewing each one.
#[allow(clippy::too_many_arguments)]
fn capture_frames(
    source: &mut crate::camera::Source,
    size: (usize, usize),
    stopping: &AtomicBool,
    keyframe: &AtomicBool,
    timed: &async_channel::Sender<TimedVideoFrame>,
    ticks: &async_channel::Sender<VideoTick>,
    encoder: &mut openh264::encoder::Encoder,
    started: Instant,
) {
    let (width, height) = size;
    let (luma_len, chroma_len) = (width * height, width * height / 4);
    let mut bytes = vec![0u8; luma_len + chroma_len * 2];
    let frame_gap = std::time::Duration::from_secs(1) / VIDEO_FPS;
    let mut last_sent: Option<Instant> = None;
    log::info!("[CALL] camera enabled size={width}x{height}");
    loop {
        if !stopping.load(Ordering::Relaxed) {
            log::info!("[CALL] camera disabled");
            return;
        }
        match source.read(&mut bytes) {
            crate::camera::Read::Frame => {}
            crate::camera::Read::Idle => continue,
            crate::camera::Read::Ended => {
                log::warn!("[CALL] the camera stopped delivering frames");
                return;
            }
        }
        // A camera faster than the call's cadence is thinned to it, so the encoder's rate control
        // and the timestamps agree.
        if last_sent.is_some_and(|last| last.elapsed() < frame_gap.mul_f32(0.8)) {
            continue;
        }
        last_sent = Some(Instant::now());

        let luma = &bytes[..luma_len];
        let chroma_u = &bytes[luma_len..luma_len + chroma_len];
        let chroma_v = &bytes[luma_len + chroma_len..];
        let planes = Yuv420 {
            y: luma,
            u: chroma_u,
            v: chroma_v,
            size,
        };
        if keyframe.swap(false, Ordering::Relaxed) {
            encoder.force_intra_frame();
        }
        match encoder.encode(&planes) {
            Ok(stream) => {
                // One access unit per encoded frame, led by an access-unit delimiter so the
                // peer's depacketizer sees the boundaries an encoder would have produced.
                let mut unit = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0xF0];
                stream.write_vec(&mut unit);
                let elapsed = started.elapsed().as_micros() as u64;
                let timestamp = (elapsed * 90 / 1000) as u32;
                // A full queue means the wire is behind: dropping is the documented answer for
                // video, and the next frame carries the picture forward anyway.
                let _ = timed.try_send(
                    TimedVideoFrame::builder()
                        .data(unit)
                        .timestamp(timestamp)
                        .build(),
                );
            }
            Err(error) => log::warn!("[CALL] camera frame could not be encoded: {error}"),
        }

        let image = crate::video::rgb_image(
            luma,
            chroma_u,
            chroma_v,
            (width, width / 2, width / 2),
            (width, height),
            preview_size(size),
            // Our own frames arrive upright from the camera: there is no rotation to undo.
            0,
        );
        let _ = ticks.try_send(VideoTick::Local(Arc::new(image)));
    }
}

/// A camera frame as `openh264` wants it: three contiguous planes with no row padding.
struct Yuv420<'a> {
    y: &'a [u8],
    u: &'a [u8],
    v: &'a [u8],
    size: (usize, usize),
}

impl openh264::formats::YUVSource for Yuv420<'_> {
    fn dimensions(&self) -> (usize, usize) {
        self.size
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.size.0, self.size.0 / 2, self.size.0 / 2)
    }

    fn y(&self) -> &[u8] {
        self.y
    }

    fn u(&self) -> &[u8] {
        self.u
    }

    fn v(&self) -> &[u8] {
        self.v
    }
}

/// Decodes the peer's access units into pictures for the UI.
///
/// A decoder cannot start on a delta frame, so frames are skipped until the sink marks one as a
/// keyframe. Runs on its own thread for the same reason the encoder does.
fn decode_remote(
    frames: async_channel::Receiver<VideoFrame>,
    ticks: async_channel::Sender<VideoTick>,
) {
    let spawned = std::thread::Builder::new()
        .name("zapfast-remote-video".to_owned())
        .spawn(move || {
            use openh264::formats::YUVSource as _;

            let mut decoder = openh264::decoder::Decoder::new().ok();
            let mut started = false;
            let mut last_orientation = 0u8;
            while let Ok(frame) = frames.recv_blocking() {
                if !started {
                    if !frame.keyframe {
                        continue;
                    }
                    started = true;
                }
                let Some(decoder) = decoder.as_mut() else {
                    return;
                };
                // The peer's camera rotation, which the frames carry so a phone held upright arrives
                // upright. It rides the same bits on a keyframe and on a delta frame, and a peer
                // that turns its phone changes it mid-call, so it is read per frame rather than once.
                let orientation = frame.orientation & 0x03;
                if orientation != last_orientation {
                    last_orientation = orientation;
                    log::info!(
                        "[CALL] remote camera orientation is {} degrees",
                        orientation as u32 * 90
                    );
                }
                let turns = crate::video::orientation_turns(orientation);
                match decoder.decode(&frame.data) {
                    Ok(Some(decoded)) => {
                        let (width, height) = decoded.dimensions();
                        if width == 0 || height == 0 {
                            continue;
                        }
                        let image = crate::video::rgb_image(
                            decoded.y(),
                            decoded.u(),
                            decoded.v(),
                            decoded.strides(),
                            (width, height),
                            (width, height),
                            turns,
                        );
                        let _ = ticks.try_send(VideoTick::Remote(Arc::new(image)));
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::debug!("[CALL] remote video frame was not decodable: {error}");
                        let _ = ticks.try_send(VideoTick::Lost);
                    }
                }
            }
        });
    if let Err(error) = spawned {
        log::error!("[CALL] remote video decoder could not start: {error}");
    }
}

// ---------------------------------------------------------------------------
// The call
// ---------------------------------------------------------------------------

/// Counter behind [`CallUpdate::generation`].
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed) + 1
}

/// A live 1:1 call: its signaling identity, its media, and the state the UI renders.
///
/// One per worker. The worker refuses to start or accept a second call while this exists, which is
/// what keeps a double click from opening two.
pub struct Call {
    generation: u64,
    call_id: String,
    chat: String,
    direction: CallDirection,
    video: bool,
    phase: CallPhase,
    started: Option<Instant>,
    outcome: Option<CallOutcome>,
    peer_audio: Option<PeerAudio>,
    /// The offer of an incoming call that has not been answered. Held because `accept` borrows it.
    incoming: Option<Box<IncomingCall>>,
    handle: Option<Arc<CallHandle>>,
    /// Whether the media plane has reported itself live. Kept apart from the phase because the
    /// relay is allocated as soon as our offer is acked, long before anyone answers: a call is
    /// Active only once the peer has answered *and* there is a media path to talk over.
    media_ready: bool,
    mic: Option<AudioInput>,
    speaker: Option<AudioOutput>,
    video_pipe: Option<VideoPipeline>,
    remote_video: bool,
    microphone: Option<String>,
    speaker_device: Option<String>,
    camera: Option<String>,
    /// The devices the user picked that the machine no longer has, newest check last.
    lost_devices: Vec<LostDevice>,
    /// Whether the call asked the camera to capture. Kept apart from whether frames are really
    /// arriving, so a camera that stopped on its own is told from one the user switched off.
    camera_wanted: bool,
    /// The peer's request to turn this voice call into a video call, which turning the camera on
    /// answers.
    peer_video_request: Option<whatsapp_rust::voip::VideoUpgradeToken>,
    /// When the call was placed or began to ring, Unix seconds: the time its
    /// entry in the chat carries.
    began: i64,
}

impl Call {
    /// Opens a ringing call on the devices last picked, so answering it starts there.
    pub fn with_devices(mut self, devices: CallDevices) -> Self {
        self.microphone = devices.microphone;
        self.speaker_device = devices.speaker;
        self
    }

    /// Places an outgoing 1:1 call. `video` offers video from the first frame, which is what makes
    /// the peer's phone ring as a video call.
    pub async fn place(
        client: &Arc<Client>,
        chat: String,
        video: bool,
        microphone: Option<String>,
        speaker: Option<String>,
        camera: Option<String>,
    ) -> Result<(Self, Option<async_channel::Receiver<VideoTick>>)> {
        let peer: Jid = chat
            .parse()
            .map_err(|error| anyhow!("not a WhatsApp JID: {error}"))?;
        if let Some(missing) =
            missing_audio_device_off_thread(microphone.clone(), speaker.clone()).await
        {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        // Video first: a camera or encoder that will not start refuses the call before any stream
        // exists and before the peer is rung, because a video call must not silently become voice.
        let mut pipe = None;
        if video {
            match VideoPipeline::start(camera.clone()).await {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        require_video(video, pipe.is_some())?;
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone());
        let (output, output_tx) = AudioOutput::spawn(speaker.clone());

        let voip = client.voip();
        let builder = voip.call(&peer).audio(mic_rx, output_tx);
        let placed = match &pipe {
            Some((pipeline, _)) => {
                builder
                    .video(pipeline.source(), pipeline.sink())
                    .start()
                    .await
            }
            None => builder.start().await,
        };
        let handle = match placed {
            Ok(handle) => Arc::new(handle),
            Err(error) => return Err(anyhow!("WhatsApp refused the call: {error}")),
        };
        // The destination is deliberately not logged: a call's chat id is the peer's phone
        // number, and this log ships.
        log::info!(
            "[CALL] outgoing created call_id={} video={}",
            handle.call_id(),
            pipe.is_some()
        );

        let (video_pipe, frames) = match pipe {
            Some((pipeline, frames)) => (Some(pipeline), Some(frames)),
            None => (None, None),
        };
        Ok((
            Self {
                generation: next_generation(),
                call_id: handle.call_id().to_owned(),
                chat,
                direction: CallDirection::Outgoing,
                video: video_pipe.is_some(),
                phase: CallPhase::Dialing,
                started: None,
                outcome: None,
                peer_audio: None,
                incoming: None,
                handle: Some(handle),
                media_ready: false,
                mic: Some(mic),
                speaker: Some(output),
                camera_wanted: video_pipe
                    .as_ref()
                    .is_some_and(VideoPipeline::camera_running),
                peer_video_request: None,
                began: crate::util::now(),
                video_pipe,
                remote_video: false,
                microphone,
                speaker_device: speaker,
                camera,
                lost_devices: Vec::new(),
            },
            frames,
        ))
    }

    /// Records an incoming offer so the UI can ask the user. No media exists until they answer,
    /// and answering opens the system's default devices unless others are picked meanwhile.
    pub fn ringing(chat: String, incoming: Box<IncomingCall>, video: bool) -> Self {
        let call_id = incoming.action.call_id().to_owned();
        log::info!("[CALL] incoming offer call_id={call_id} video={video}");
        Self {
            generation: next_generation(),
            call_id,
            chat,
            direction: CallDirection::Incoming,
            video,
            phase: CallPhase::Incoming,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: Some(incoming),
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            video_pipe: None,
            remote_video: false,
            microphone: None,
            speaker_device: None,
            camera: None,
            lost_devices: Vec::new(),
            camera_wanted: false,
            peer_video_request: None,
            began: crate::util::now(),
        }
    }

    /// A call for a test: a snapshot with no signaling and no engine behind it, so a test can put
    /// one on the worker and check what privacy recovery does with its update.
    #[cfg(test)]
    pub(crate) fn test_snapshot(chat: &str) -> Self {
        Self {
            generation: next_generation(),
            call_id: "test-call".to_owned(),
            chat: chat.to_owned(),
            direction: CallDirection::Incoming,
            video: false,
            phase: CallPhase::Incoming,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: None,
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            video_pipe: None,
            remote_video: false,
            microphone: None,
            speaker_device: None,
            camera: None,
            lost_devices: Vec::new(),
            camera_wanted: false,
            peer_video_request: None,
            began: crate::util::now(),
        }
    }

    /// The WhatsApp call id, which is how signaling for this call is told from any other's.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// The chat this call belongs to.
    pub fn chat(&self) -> &str {
        &self.chat
    }

    pub fn phase(&self) -> CallPhase {
        self.phase
    }

    pub fn is_video(&self) -> bool {
        self.video
    }

    /// The live handle, once the call has one.
    pub fn handle(&self) -> Option<Arc<CallHandle>> {
        self.handle.clone()
    }

    /// The snapshot the UI renders.
    pub fn update(&self) -> CallUpdate {
        CallUpdate {
            generation: self.generation,
            chat: self.chat.clone(),
            direction: self.direction,
            video: self.video,
            phase: self.phase,
            started: self.started,
            muted: self.handle.as_ref().is_some_and(|handle| handle.is_muted()),
            camera_on: self
                .video_pipe
                .as_ref()
                .is_some_and(VideoPipeline::camera_running),
            remote_video: self.remote_video,
            video_requested: self.peer_video_request.is_some(),
            outcome: self.outcome,
            peer_audio: self.peer_audio,
            lost_devices: self.lost_devices.clone(),
            microphone: self.microphone.clone(),
            speaker: self.speaker_device.clone(),
            camera: self.camera.clone(),
        }
    }

    /// When the call was placed or began to ring, Unix seconds.
    pub fn began(&self) -> i64 {
        self.began
    }

    /// Whether the call carried video at any point.
    pub fn carried_video(&self) -> bool {
        self.video
    }

    /// The id that tells one call from the next, for a caller that keeps the last snapshot.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Says which picked devices are not the ones in use, for the call screen to word.
    pub fn set_lost_devices(&mut self, lost: Vec<LostDevice>) {
        self.lost_devices = lost;
    }

    /// Picks up streams that had to reopen on the system default, and forgets the selections they
    /// were bound to. A Bluetooth headset that switches off mid-call can reach here before device
    /// discovery notices it is gone; either way the call says what moved.
    pub fn take_stream_fallbacks(&mut self) -> Vec<(DeviceKind, String)> {
        let mut lost = Vec::new();
        if self
            .mic
            .as_ref()
            .is_some_and(|mic| mic.fell_back.try_recv().is_ok())
            && let Some(device) = self.microphone.take()
        {
            lost.push((DeviceKind::Microphone, device));
        }
        if self
            .speaker
            .as_ref()
            .is_some_and(|speaker| speaker.fell_back.try_recv().is_ok())
            && let Some(device) = self.speaker_device.take()
        {
            lost.push((DeviceKind::Speaker, device));
        }
        lost
    }

    /// Rebinds anything the machine no longer has: a headset switched off, a camera unplugged.
    ///
    /// Returns what had to change, so the caller can say it. Nothing is reported against an empty
    /// list, which is a discovery tool that could not run rather than an empty machine. The call
    /// itself is never ended here: a lost camera leaves the rest of the call alone, and a lost
    /// microphone or speaker moves to the system default.
    pub async fn verify_devices(&mut self, list: &DeviceList) -> Vec<(DeviceKind, String)> {
        let mut lost = Vec::new();
        if !self.phase.is_live() {
            return lost;
        }
        if let Some(device) = self.microphone.clone()
            && !list.microphones.is_empty()
            && !list.microphones.iter().any(|known| known.id == device)
        {
            self.set_microphone(None);
            lost.push((DeviceKind::Microphone, device));
        }
        if let Some(device) = self.speaker_device.clone()
            && !list.speakers.is_empty()
            && !list.speakers.iter().any(|known| known.id == device)
        {
            self.set_speaker(None);
            lost.push((DeviceKind::Speaker, device));
        }
        if let Some(device) = self.camera.clone()
            && !list.cameras.is_empty()
            && !list.cameras.iter().any(|known| known.id == device)
        {
            self.camera = None;
            lost.push((DeviceKind::Camera, device));
            if self
                .video_pipe
                .as_ref()
                .is_some_and(VideoPipeline::camera_running)
            {
                // The peer is being sent a stream with no source behind it: stop it properly
                // rather than leaving the last frame frozen on their screen.
                if let Err(error) = self.set_camera(false).await {
                    log::warn!("[CALL] the peer was not told the camera is gone: {error}");
                }
            } else {
                self.camera_wanted = false;
            }
        }
        lost
    }

    /// Whether the camera stopped on its own: the call still wants it, and no frames are coming.
    ///
    /// The peer has a video stream open at this point, so a stalled camera is stopped the same way
    /// a switched-off one is instead of being left frozen.
    pub fn camera_stalled(&self) -> bool {
        self.camera_wanted
            && self.handle.is_some()
            && self
                .video_pipe
                .as_ref()
                .is_some_and(|pipe| !pipe.camera_running())
    }

    /// The devices picked so far: what a call that has not started media yet should bind to, so a
    /// microphone chosen while the phone was ringing is the one the call actually uses.
    pub fn selections(&self) -> (Option<String>, Option<String>, Option<String>) {
        (
            self.microphone.clone(),
            self.speaker_device.clone(),
            self.camera.clone(),
        )
    }

    /// Answers a ringing call: sends the real `<accept>`, brings up media, and returns the frame
    /// channel for the UI.
    pub async fn answer(
        &mut self,
        client: &Arc<Client>,
        microphone: Option<String>,
        speaker: Option<String>,
        camera: Option<String>,
    ) -> Result<Option<async_channel::Receiver<VideoTick>>> {
        let Some(incoming) = self.incoming.clone() else {
            return Err(anyhow!("nothing is ringing"));
        };
        // Answering opens the same streams a call places, so the same checks apply: an answered call
        // without a microphone would be worse than a refused one.
        if let Some(missing) =
            missing_audio_device_off_thread(microphone.clone(), speaker.clone()).await
        {
            return Err(anyhow!("no {missing} is available; a call needs one"));
        }
        // The same rule as placing a call: a video offer that cannot start video is not accepted as
        // something else behind the user's back. Nothing has been sent yet, so the call stays
        // ringing and the user can still decline it.
        let mut pipe = None;
        if self.video {
            match VideoPipeline::start(camera.clone()).await {
                Ok((pipeline, frames)) => pipe = Some((pipeline, frames)),
                Err(error) => log::warn!("[CALL] video pipeline could not start: {error}"),
            }
        }
        require_video(self.video, pipe.is_some())?;
        let (mic, mic_rx) = AudioInput::spawn(microphone.clone());
        let (output, output_tx) = AudioOutput::spawn(speaker.clone());

        let voip = client.voip();
        // From here the library owns the answer: if it fails, its own guard ends the call with the
        // peer, so there is no offer left for [`Call::abandon`] to refuse.
        self.incoming = None;
        let builder = voip.accept(&incoming).audio(mic_rx, output_tx);
        let accepted = match &pipe {
            Some((pipeline, _)) => {
                builder
                    .video(pipeline.source(), pipeline.sink())
                    .start()
                    .await
            }
            None => builder.start().await,
        };
        let handle = match accepted {
            Ok(handle) => Arc::new(handle),
            Err(error) => return Err(anyhow!("WhatsApp refused the answer: {error}")),
        };
        log::info!("[CALL] accepted call_id={}", handle.call_id());
        self.call_id = handle.call_id().to_owned();
        self.handle = Some(handle);
        self.mic = Some(mic);
        self.speaker = Some(output);
        self.microphone = microphone;
        self.speaker_device = speaker;
        self.camera = camera;
        self.video &= pipe.is_some();
        self.camera_wanted = pipe
            .as_ref()
            .is_some_and(|(pipeline, _)| pipeline.camera_running());
        // The user picked the phone up. The media plane is still coming up, so this is `Accepted`
        // rather than `Connecting`, which is what the peer's own `<accept>` means on an outgoing
        // call: neither phase claims the call is live yet.
        self.phase = CallPhase::Accepted;
        log::info!("[CALL] accepted locally call_id={}", self.call_id);
        self.became_active();
        let frames = match pipe {
            Some((pipeline, frames)) => {
                self.video_pipe = Some(pipeline);
                Some(frames)
            }
            None => None,
        };
        Ok(frames)
    }

    /// Ends a call this side could not answer: the offer is refused when it is still ours to
    /// refuse, so the caller is not left ringing, and the call reads as one that never came up.
    pub async fn abandon(&mut self, client: &Arc<Client>) -> CallUpdate {
        if let Some(incoming) = self.incoming.take()
            && let Err(error) = client.voip().reject(&incoming).await
        {
            log::warn!("[CALL] the offer could not be refused: {error}");
        }
        self.cleanup();
        self.phase = CallPhase::Failed;
        self.outcome = Some(CallOutcome::Failed);
        self.update()
    }

    /// Declines a ringing call with the real `<reject>`.
    pub async fn decline(&mut self, client: &Arc<Client>) -> Result<()> {
        let Some(incoming) = self.incoming.clone() else {
            return Err(anyhow!("nothing is ringing"));
        };
        log::info!("[CALL] rejecting call_id={}", self.call_id);
        client
            .voip()
            .reject(&incoming)
            .await
            .map_err(|error| anyhow!("WhatsApp refused the rejection: {error}"))?;
        self.incoming = None;
        self.phase = CallPhase::Ended;
        self.outcome = Some(CallOutcome::Declined);
        Ok(())
    }

    /// Ends the call: the real `<terminate>` (or `<reject>` while ringing), then every local
    /// resource released. Safe to call once the call is already over.
    pub async fn hangup(&mut self, client: Option<&Arc<Client>>) -> CallUpdate {
        if let Some(handle) = self.handle.clone() {
            log::info!("[CALL] ending call_id={}", handle.call_id());
            let outcome = handle.terminate().await;
            log::info!(
                "[CALL] terminate outcome {outcome:?} peer_notified={}",
                outcome.peer_notified()
            );
        } else if let Some((client, incoming)) = client.zip(self.incoming.clone()) {
            log::info!("[CALL] declining ringing call_id={}", self.call_id);
            let _ = client.voip().reject(&incoming).await;
        }
        // Only a call that was really up counts as answered. `is_connected` also covers the window
        // between the peer's answer and the media plane coming up, and a hangup in that window would
        // be recorded as an answered call of zero seconds even though the length only starts once
        // the call is Active.
        let connected = self.started.is_some();
        let was_live = self.phase.is_live();
        self.cleanup();
        if was_live {
            self.phase = CallPhase::Ended;
            // We ended it, so this is not the peer's answer to record. What it was before the
            // hangup is: a call that was up was answered, one still ringing never was.
            self.outcome = Some(if connected {
                CallOutcome::Answered
            } else if self.direction == CallDirection::Incoming {
                CallOutcome::Missed
            } else {
                CallOutcome::NoAnswer
            });
        }
        self.update()
    }

    /// Releases every local resource. Safe to call twice.
    pub fn cleanup(&mut self) {
        if let Some(mut pipe) = self.video_pipe.take() {
            pipe.shutdown();
        }
        if let Some(mut mic) = self.mic.take() {
            mic.stop();
        }
        if let Some(mut speaker) = self.speaker.take() {
            speaker.stop();
        }
        self.handle = None;
        self.incoming = None;
        log::info!("[CALL] cleanup complete call_id={}", self.call_id);
    }

    /// Applies one signaling stanza, when it belongs to this call.
    pub fn signaling(&mut self, action: &CallAction) -> Option<CallUpdate> {
        if action.call_id() != self.call_id {
            return None;
        }
        // A call that is already over keeps the answer it recorded. A peer that rejects and then
        // sends the terminate that follows it would otherwise have the rejection read as a call
        // nobody answered, and the log would name the wrong reason for a call that really did end.
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            log::debug!(
                "[CALL] a stanza arrived for a call that is already over call_id={} phase={:?}",
                self.call_id,
                self.phase
            );
            return None;
        }
        match action {
            CallAction::PreAccept { .. } => {
                if self.phase == CallPhase::Dialing {
                    log::info!("[CALL] remote ringing call_id={}", self.call_id);
                    self.phase = CallPhase::Ringing;
                    return Some(self.update());
                }
                None
            }
            CallAction::Accept { .. } => {
                if matches!(self.phase, CallPhase::Dialing | CallPhase::Ringing) {
                    log::info!("[CALL] remote accepted call_id={}", self.call_id);
                    self.phase = CallPhase::Connecting;
                    // A relay that was allocated while the peer's phone was ringing is a live media
                    // path waiting for them: the answer is what makes the call active, not the
                    // allocate, so both halves are checked here and in `media`.
                    return self.became_active().or_else(|| Some(self.update()));
                }
                None
            }
            CallAction::Reject { reason, .. } => {
                log::info!(
                    "[CALL] remote rejected call_id={} reason={reason:?}",
                    self.call_id
                );
                self.phase = CallPhase::Failed;
                self.outcome = Some(rejection_outcome(reason.as_deref()));
                self.cleanup();
                Some(self.update())
            }
            CallAction::Terminate { reason, .. } => {
                log::info!(
                    "[CALL] peer terminated call_id={} reason={reason:?}",
                    self.call_id
                );
                // Only a call that really reached Active was answered. `is_connected` also covers
                // the window between the peer's answer and the media plane coming up, and a
                // terminate there would be recorded as an answered call of zero seconds.
                let connected = self.started.is_some();
                self.phase = if connected {
                    CallPhase::Ended
                } else {
                    CallPhase::Failed
                };
                self.outcome = Some(termination_outcome(reason.as_deref(), connected));
                self.cleanup();
                Some(self.update())
            }
            _ => None,
        }
    }

    /// Applies one media-plane event.
    pub fn media(&mut self, event: &CallEvent) -> Option<CallUpdate> {
        match event {
            CallEvent::RelayAllocated => {
                self.media_ready = true;
                self.became_active()
            }
            CallEvent::RelayAllocateFailed(code) => {
                self.fail(format!("The relay refused the call ({code})"))
            }
            CallEvent::RelayAllocateTimedOut => self.fail("The relay did not answer".to_owned()),
            CallEvent::RelayReconnectTimedOut => {
                self.fail("The relay connection dropped".to_owned())
            }
            CallEvent::MediaSetupFailed(reason) => self.fail(reason.clone()),
            CallEvent::Closed(reason) => {
                log::info!(
                    "[CALL] media closed call_id={} reason={reason:?}",
                    self.call_id
                );
                self.ended_by(CallOutcome::ConnectionLost)
            }
            // The audio-health alarms, which are the only place a call that is connected but
            // carrying no audio says so. Each fires on its own schedule and repeats while the
            // condition holds, so only a change is published: the counters in the log are the
            // diagnosis, the screen only has room for the reason.
            CallEvent::AudioSilent {
                dominant_reason,
                rtp_received,
                frames_produced,
                silent_for_ms,
            } => {
                log::warn!(
                    "[CALL] the peer's audio is not becoming sound: reason={dominant_reason:?} silent_ms={silent_for_ms} rtp_received={rtp_received} frames_produced={frames_produced}"
                );
                let reason = silence_reason(*dominant_reason);
                if self.peer_audio == Some(PeerAudio::Silent(reason)) {
                    return None;
                }
                self.peer_audio = Some(PeerAudio::Silent(reason));
                Some(self.update())
            }
            CallEvent::AudioReceptionStalled { silent_for_ms } => {
                log::warn!("[CALL] no audio is arriving from the peer: silent_ms={silent_for_ms}");
                if self.peer_audio == Some(PeerAudio::Stalled) {
                    return None;
                }
                self.peer_audio = Some(PeerAudio::Stalled);
                Some(self.update())
            }
            CallEvent::AudioCodecSwitched {
                from,
                to,
                source,
                packets_observed,
            } => {
                log::info!(
                    "[CALL] audio codec switched {from:?} -> {to:?} source={source:?} packets={packets_observed}"
                );
                // The decode path moved, so whatever silence was reported is over.
                if self.peer_audio.take().is_some() {
                    return Some(self.update());
                }
                None
            }
            CallEvent::AudioFormatMismatch {
                expected_rate,
                received_rates,
            } => {
                log::warn!(
                    "[CALL] the peer offered audio rates {received_rates:?} against ours of {expected_rate}"
                );
                None
            }
            CallEvent::AudioCodecSourceIsFixed {
                sending,
                peer_expects,
                source,
            } => {
                log::warn!(
                    "[CALL] this call sends {sending:?} while the peer expects {peer_expects:?} source={source:?}"
                );
                None
            }
            // The peer's own video state, which is the only thing that says their picture stopped.
            // Without this the flag only ever went true, so a peer who turned their camera off was
            // still reported as sending video and the decoder's last frame stayed on screen as if
            // it were live.
            // Outbound video cannot go on the wire until a keyframe does.
            CallEvent::VideoKeyframeNeeded => {
                if let Some(pipe) = &self.video_pipe {
                    pipe.force_keyframe();
                }
                None
            }
            CallEvent::PeerVideoStateChanged {
                state,
                upgrade_token,
                ..
            } => {
                if state.is_upgrade_request() && !self.video {
                    // The peer turned their camera on in a voice call: the screen asks, and the
                    // camera button answers. A request resolved by signaling has no token.
                    let Some(token) = upgrade_token else {
                        return None;
                    };
                    log::info!("[CALL] the peer asked for video call_id={}", self.call_id);
                    self.peer_video_request = Some(*token);
                    return Some(self.update());
                }
                if state.is_inactive_for_call_mode() {
                    let withdrawn = self.peer_video_request.take().is_some();
                    if self.remote_video_stopped() || withdrawn {
                        return Some(self.update());
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Reads the engine's media counters into the log, so a call that is up but not carrying audio
    /// can be diagnosed from a bug report rather than guessed at. Called from the call heartbeat.
    pub fn log_media_stats(&self) {
        self.log_media_stats_at("heartbeat");
    }

    /// The same counters, tagged with what the call was doing when they were read.
    ///
    /// The tag is the whole point of the extra call sites: a heartbeat lands at some arbitrary
    /// point in a call, while these are read around the transitions that are suspected of changing
    /// it, so two lines a second apart are a before and an after rather than two samples of a
    /// steady state.
    pub fn log_media_stats_at(&self, marker: &str) {
        let Some(handle) = self.handle.as_ref() else {
            self.log_audio_path(marker);
            return;
        };
        // Every counter the engine keeps, by its own name. A list written out here missed one
        // (forwarding_envelope_rejected) and would miss each counter a newer engine adds, such as
        // its counts of the peer's video packets; the whole snapshot carries them all, and it
        // holds counts and nothing else.
        let stats = handle.media_stats();
        log::info!(
            "[CALL] media stats call_id={} at={} {stats:?}",
            self.call_id,
            marker,
        );
        self.log_audio_path(marker);
    }

    /// The call's own audio path, counted where the engine cannot see it.
    ///
    /// The engine's counters above say what it decoded and handed over; these say what this side
    /// did with it. Together they are what tells a call that is carrying nothing (nothing received,
    /// nothing decoded) from one whose audio dies after the engine handed it over, and from one
    /// whose sink was reopened: a reader or writer open that keeps climbing during a call is a
    /// stream being restarted rather than rebound.
    pub fn log_audio_path(&self, marker: &str) {
        log::info!(
            "[CALL] audio path call_id={} at={} mic_opens={} speaker_opens={} speaker_stalls={} speaker_restarts={} live={}",
            self.call_id,
            marker,
            self.mic
                .as_ref()
                .map_or(0, |mic| mic.opens.load(Ordering::Relaxed)),
            self.speaker
                .as_ref()
                .map_or(0, |speaker| speaker.opens.load(Ordering::Relaxed)),
            self.speaker
                .as_ref()
                .map_or(0, |speaker| speaker.stalls.load(Ordering::Relaxed)),
            self.speaker
                .as_ref()
                .map_or(0, |speaker| speaker.restarts.load(Ordering::Relaxed)),
            self.phase.is_live(),
        );
    }

    /// The media task is gone; whatever the phase was, the call is over.
    pub fn media_ended(&mut self) -> Option<CallUpdate> {
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            return None;
        }
        log::info!("[CALL] ended call_id={}", self.call_id);
        // A call is answered once it was Active and only then; the media going away while the call
        // was still connecting is a call that never came up.
        let outcome = if self.started.is_some() {
            CallOutcome::Answered
        } else {
            CallOutcome::NoAnswer
        };
        self.ended_by(outcome)
    }

    /// The call was resolved on another of the account's devices (`elsewhere` says how), or the
    /// caller gave up (`None`).
    ///
    /// A call we were ringing for that was answered or declined elsewhere is not the same thing as
    /// an unanswered one, so the cases get their own words while all release the media.
    pub fn resolved_elsewhere(&mut self, elsewhere: Option<CallOutcome>) -> Option<CallUpdate> {
        if !self.phase.is_live() {
            return None;
        }
        let ringing = self.phase == CallPhase::Incoming;
        // Only a call that reached Active really connected here. A resolve that arrives during
        // media negotiation is not an answered-elsewhere call with a zero duration: this device
        // never had the call up.
        let live = self.started.is_some();
        log::info!(
            "[CALL] resolved elsewhere call_id={} phase={:?}",
            self.call_id,
            self.phase
        );
        self.phase = if live {
            CallPhase::Ended
        } else {
            CallPhase::Failed
        };
        self.outcome = Some(if live {
            CallOutcome::AnsweredElsewhere
        } else if let Some(outcome) = elsewhere {
            outcome
        } else if ringing {
            CallOutcome::Missed
        } else {
            CallOutcome::NoAnswer
        });
        self.cleanup();
        Some(self.update())
    }

    /// Moves to Active once both halves are in place: the peer answered and the media plane is up.
    ///
    /// Either half alone is not a call. The relay is allocated as soon as our offer is acked, so
    /// `RelayAllocated` on its own would show a connected call while the other phone is still
    /// ringing — which is exactly the state this phase machine exists to avoid.
    fn became_active(&mut self) -> Option<CallUpdate> {
        if !matches!(self.phase, CallPhase::Connecting | CallPhase::Accepted) || !self.media_ready {
            return None;
        }
        self.phase = CallPhase::Active;
        // The timer starts here, when there is really a call, never at the button press.
        self.started = Some(Instant::now());
        log::info!(
            "[CALL] active call_id={} video={}",
            self.call_id,
            self.video
        );
        Some(self.update())
    }

    /// Ends the call because its media went away, telling a call that was up from one that never
    /// came up: the phase says which, and the outcome names the cause either way.
    fn ended_by(&mut self, outcome: CallOutcome) -> Option<CallUpdate> {
        // A media event that arrives after the call is already over must not reopen it or rewrite
        // the outcome it ended with. The relay's `Closed` can land after a hang-up has ended the
        // call, and recording that as `ConnectionLost` would replace the real reason with a false
        // one and publish a second end for a call the screen has already dropped.
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            return None;
        }
        self.phase = if self.started.is_some() {
            CallPhase::Ended
        } else {
            CallPhase::Failed
        };
        self.outcome = Some(outcome);
        self.cleanup();
        Some(self.update())
    }

    fn fail(&mut self, reason: String) -> Option<CallUpdate> {
        if matches!(self.phase, CallPhase::Ended | CallPhase::Failed) {
            return None;
        }
        log::warn!("[CALL] failed call_id={} reason={reason}", self.call_id);
        self.phase = CallPhase::Failed;
        self.outcome = Some(CallOutcome::Failed);
        self.cleanup();
        Some(self.update())
    }

    /// Mutes or unmutes through the engine, which is what stops outgoing audio.
    ///
    /// The engine zeroes the frames it already has while this is set, so the microphone stream
    /// stays fed and the peer never re-negotiates the transport.
    pub async fn set_muted(&mut self, muted: bool) -> Option<CallUpdate> {
        let handle = self.handle.clone()?;
        match handle.set_muted(muted).await {
            Ok(()) => {}
            Err(error) if muted => log::warn!(
                "[CALL] the peer was not told about the mute; the microphone is muted locally: {error}"
            ),
            Err(error) => log::warn!("[CALL] unmute failed, the microphone stays muted: {error}"),
        }
        let now = handle.is_muted();
        if now != muted {
            log::warn!("[CALL] mute state is {now} after asking for {muted}");
        }
        if now {
            log::info!("[CALL] microphone muted");
        } else {
            log::info!("[CALL] microphone unmuted");
        }
        Some(self.update())
    }

    /// Says a media stream the call owns has just been opened or closed.
    ///
    /// A camera is a second stream on this machine's audio graph, and the graph relinks every
    /// stream already on it while the new one settles. The sink is told, so a pause that belongs to
    /// the graph is not read as a device that died and is not recovered from by throwing the peer's
    /// audio away.
    fn media_stream_changed(&self) {
        if let Some(speaker) = &self.speaker {
            speaker.relink();
        }
    }

    /// The engine's counters and this side's audio path, for one transition of a call.
    ///
    /// Called at every point a media stream comes or goes, so a report has the numbers from before
    /// and after the change rather than one heartbeat somewhere in the middle. A call that goes
    /// quiet when the camera comes on is diagnosed from these lines: `rtp_received` and
    /// `audio_frames_decoded` climbing while `audio_sink_dropped` climbs says the peer is still
    /// being heard and this side is the one dropping it, and a flat `rtp_received` says the
    /// opposite.
    pub fn note_transition(&self, marker: &str) {
        self.log_media_stats_at(marker);
    }

    /// Turns the camera on or off through the engine's video direction.
    pub async fn set_camera(&mut self, on: bool) -> Result<CallUpdate> {
        let Some(handle) = self.handle.clone() else {
            return Err(anyhow!("the call is not up"));
        };
        self.note_transition(if on {
            "camera on requested"
        } else {
            "camera off requested"
        });
        if on {
            let Some(pipe) = self.video_pipe.as_mut() else {
                return Err(anyhow!("this call has no video"));
            };
            // The local capture comes up before the engine is asked to carry it. If the engine
            // refuses, the capture that was just started is rolled back: leaving it running would
            // be a camera holding the device and frames going to nobody, while the screen still
            // reads the call as video-off.
            pipe.resume_camera(self.camera.clone()).await?;
            let (source, sink) = (pipe.source(), pipe.sink());
            // The camera stream exists now, so the graph has relinked around it.
            if let Some(speaker) = &self.speaker {
                speaker.relink();
            }
            if let Err(error) = handle.resume_video(source, sink).await {
                if let Some(pipe) = self.video_pipe.as_mut() {
                    pipe.pause_camera();
                }
                self.camera_wanted = false;
                self.media_stream_changed();
                self.note_transition("camera on rolled back");
                return Err(anyhow!("the camera could not be enabled: {error}"));
            }
            self.camera_wanted = true;
        } else {
            // The local capture is what must not outlive the press, so it is stopped whatever the
            // engine says. A failure to tell the peer is still surfaced, but it cannot be allowed
            // to leave the camera capturing and sending: the request was to turn it off.
            let notified = handle.stop_video().await;
            if let Some(pipe) = self.video_pipe.as_mut() {
                pipe.pause_camera();
            }
            self.camera_wanted = false;
            self.media_stream_changed();
            if let Err(error) = notified {
                self.note_transition("camera off, peer not told");
                return Err(anyhow!(
                    "the camera was stopped, but the peer was not told: {error}"
                ));
            }
        }
        log::info!("[CALL] camera enabled={on}");
        self.note_transition(if on {
            "camera on settled"
        } else {
            "camera off settled"
        });
        Ok(self.update())
    }

    /// Asks the peer for video on a call that started as voice, or turns the camera back on when
    /// this is already a video call.
    ///
    /// The upgrade is the real one: `CallHandle::start_video` attaches the endpoints, enables the
    /// media plane and sends `<video state=11>`, and the peer's answer arrives as video state
    /// signaling. Returns the frame channel the UI should start draining.
    ///
    /// The camera arrives already resolved, the same way a call's startup resolves it: upgrading a
    /// voice call on a fresh install opens the machine's own camera instead of failing on a `None`
    /// that only ever meant "default device".
    pub async fn upgrade_to_video(
        &mut self,
        camera: Option<String>,
    ) -> Result<Option<async_channel::Receiver<VideoTick>>> {
        let Some(handle) = self.handle.clone() else {
            return Err(anyhow!("the call is not up"));
        };
        self.note_transition("video requested");
        if self.video_pipe.is_none() {
            let request = self.peer_video_request.take();
            let (pipe, frames) = VideoPipeline::start(camera.clone()).await?;
            self.media_stream_changed();
            self.camera_wanted = pipe.camera_running();
            self.video_pipe = Some(pipe);
            self.camera = camera.clone();
            self.video = true;
            let (source, sink) = {
                let pipe = self.video_pipe.as_ref().expect("just installed");
                (pipe.source(), pipe.sink())
            };
            // A peer who asked for video is answered; otherwise the peer is asked.
            let started = match request {
                Some(token) => handle.accept_video(token, source, sink).await,
                None => handle.start_video(source, sink).await,
            };
            if let Err(error) = started {
                self.abandon_video();
                return Err(anyhow!("the peer could not be asked for video: {error}"));
            }
            log::info!("[CALL] video upgrade requested call_id={}", self.call_id);
            self.note_transition("video requested ok");
            return Ok(Some(frames));
        }
        self.set_camera(true).await?;
        Ok(None)
    }

    /// Gives back everything an upgrade that the peer refused had built.
    ///
    /// The peer was never asked, so there is no video call to be in: leaving the pipeline installed
    /// would be a call the screen draws as sending a picture that was never announced, with a camera
    /// holding the device and the frames going to nobody. The camera is stopped, the pipeline is
    /// dropped, and the selection is cleared so the next press starts from the machine rather than
    /// from a device this attempt had already taken.
    fn abandon_video(&mut self) {
        if let Some(mut pipe) = self.video_pipe.take() {
            pipe.shutdown();
        }
        self.video = false;
        self.camera = None;
        self.camera_wanted = false;
        self.media_stream_changed();
        log::warn!("[CALL] video upgrade refused call_id={}", self.call_id);
    }

    /// Rebinds the microphone. The engine's channel is untouched.
    pub fn set_microphone(&mut self, device: Option<String>) -> CallUpdate {
        self.microphone = device.clone();
        self.lost_devices.clear();
        if let Some(mic) = &self.mic {
            mic.bind(device);
        }
        log::info!("[CALL] microphone device {:?}", self.microphone);
        self.update()
    }

    /// Rebinds the speaker. The engine's channel is untouched.
    pub fn set_speaker(&mut self, device: Option<String>) -> CallUpdate {
        self.speaker_device = device.clone();
        self.lost_devices.clear();
        if let Some(speaker) = &self.speaker {
            speaker.bind(device);
        }
        log::info!("[CALL] speaker device {:?}", self.speaker_device);
        self.update()
    }

    /// Switches the camera. A running camera is restarted on the new node.
    ///
    /// The device arrives already resolved: `None` is the picker's "First camera", and a camera
    /// has no system default, so the caller reads it as the first node the machine reports — exactly
    /// as [`resolve_devices`] reads it when a call starts. Resolving it here instead would enumerate
    /// devices on whatever thread is driving the call, which is the worker's async loop.
    pub async fn set_camera_device(&mut self, device: Option<String>) -> Result<CallUpdate> {
        self.camera = device.clone();
        self.lost_devices.clear();
        // Said before the switch is attempted, because the stream is going either way: the old
        // camera is already released by the time a new one is opened, so the graph relinks whether
        // or not the new node turns out to be usable.
        if self
            .video_pipe
            .as_ref()
            .is_some_and(VideoPipeline::camera_running)
        {
            self.media_stream_changed();
        }
        if let Some(pipe) = self.video_pipe.as_mut()
            && pipe.camera_running()
        {
            pipe.resume_camera(device).await?;
        }
        log::info!("[CALL] camera device {:?}", self.camera);
        Ok(self.update())
    }

    /// Whether the peer's picture is the one to show right now.
    ///
    /// The flag itself is private so it can only be set by a frame arriving and cleared by the
    /// peer's own signaling; this is how the frame path asks before handing a picture to the UI.
    pub fn peer_video_visible(&self) -> bool {
        self.remote_video
    }

    /// Notes that the peer's picture arrived, so the UI can stop saying it is waiting.
    pub fn saw_remote_video(&mut self) -> bool {
        if self.remote_video {
            return false;
        }
        self.remote_video = true;
        log::info!("[CALL] remote video received call_id={}", self.call_id);
        true
    }

    /// Notes that the peer's picture stopped, so the UI stops showing the last frame it had.
    ///
    /// `remote_video` only ever became true, and nothing turned it back: a peer who turned their
    /// camera off left the call reported as sending video and the decoder's last frame on screen as
    /// if it were live. The state that says otherwise is the peer's own `<video state=N>` signaling.
    /// Returns whether this was a change, so only a change is published.
    pub fn remote_video_stopped(&mut self) -> bool {
        if !self.remote_video {
            return false;
        }
        self.remote_video = false;
        log::info!("[CALL] remote video stopped call_id={}", self.call_id);
        true
    }
}

/// How a `<reject>`'s reason reads as an outcome. The peer refusing for a reason we do not know is
/// still a refusal.
fn rejection_outcome(reason: Option<&str>) -> CallOutcome {
    match reason {
        Some("busy") => CallOutcome::Busy,
        Some("enc") => CallOutcome::Failed,
        _ => CallOutcome::Declined,
    }
}

/// How a `<terminate>`'s reason reads as an outcome, given whether the call had been up.
///
/// The reasons that talk about the call's own progress are read against whether it had been up,
/// because a reason is what the peer says and the timer is what happened here: a call that was up
/// has an answer to record whatever arrives. Reported from a real call, a
/// `<terminate reason="timeout">` four seconds after the camera came on turned a nineteen-second
/// conversation into a log entry reading "No answer", with its duration sitting right beside it.
/// Those two facts cannot both be true, and the duration is the one this side measured. The
/// `*_elsewhere` reasons are facts about the account's other devices and stand either way.
fn termination_outcome(reason: Option<&str>, connected: bool) -> CallOutcome {
    match reason {
        // Another of this account's devices took the call over, whichever state this one was in.
        Some("accepted_elsewhere") => CallOutcome::AnsweredElsewhere,
        Some("rejected_elsewhere") => CallOutcome::DeclinedElsewhere,
        // The peer's phone giving up on a call that was ringing, for the one that never answered.
        Some("timeout") => {
            if connected {
                CallOutcome::ConnectionLost
            } else {
                CallOutcome::NoAnswer
            }
        }
        // A group call that ended under a one-to-one call's member: the call was answered only if
        // there was a call here to answer.
        Some("group_call_ended") => {
            if connected {
                CallOutcome::Answered
            } else {
                CallOutcome::NoAnswer
            }
        }
        _ if connected => CallOutcome::Answered,
        _ => CallOutcome::NoAnswer,
    }
}

/// The engine's silence reason under this app's own name, so the model never leaks the library's
/// enum into the interface.
fn silence_reason(reason: whatsapp_rust::voip_control::MediaSilenceReason) -> SilenceReason {
    use whatsapp_rust::voip_control::MediaSilenceReason as Engine;
    match reason {
        Engine::NoDecoderForNegotiatedCodec => SilenceReason::NoDecoder,
        Engine::AuthenticationFailing => SilenceReason::AuthenticationFailing,
        Engine::UnexpectedPayloadType => SilenceReason::UnexpectedPayloadType,
        Engine::CodecRejectingFrames => SilenceReason::CodecRejectingFrames,
        Engine::CodecFlapping => SilenceReason::CodecFlapping,
        _ => SilenceReason::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine with one of each, named the way a picker would show them.
    fn machine() -> DeviceList {
        DeviceList {
            microphones: vec![AudioDevice {
                id: "alsa_input.usb-Generic_USB_Headset-00.analog-mono".to_owned(),
                label: "USB Headset Microphone".to_owned(),
            }],
            speakers: vec![AudioDevice {
                id: "alsa_output.usb-Generic_USB_Headset-00.analog-stereo".to_owned(),
                label: "USB Headset Stereo".to_owned(),
            }],
            cameras: vec![CameraDevice {
                id: "0".to_owned(),
                label: "Laptop Camera".to_owned(),
            }],
        }
    }

    /// The pickers list what rodio reports, each device named and labelled.
    #[test]
    fn the_audio_pickers_name_every_device() {
        let listed = super::devices();
        for device in listed.microphones.iter().chain(&listed.speakers) {
            assert!(
                !device.id.is_empty(),
                "a device is named by what rodio reports"
            );
            assert!(
                !device.label.is_empty(),
                "a device is labelled for the picker"
            );
        }
        for device in &listed.cameras {
            assert!(!device.id.is_empty(), "a camera is named by its node");
            assert!(
                !device.label.is_empty(),
                "a camera is labelled for the picker"
            );
        }
    }

    /// Changing the camera, the microphone or the speaker leaves the call's audio alone.
    ///
    /// This is the regression the video direction once caused: turning the camera on took the
    /// remote audio with it. The devices below belong to the test rather than the machine, so this
    /// holds on a build host with no camera and no sound at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn changing_media_devices_leaves_the_calls_audio_running() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, speaker) = crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (input, frames) = AudioInput::spawn(None);
        let (output, _tx) = AudioOutput::spawn(None);
        call.mic = Some(input);
        call.speaker = Some(output);

        // The microphone is delivering before anything else is touched.
        assert!(
            frames.recv().await.is_ok(),
            "the microphone is delivering before the camera is touched"
        );

        call.set_camera_device(Some("9".to_owned()))
            .await
            .expect("a camera can be selected");
        call.set_camera_device(None)
            .await
            .expect("a camera can be cleared");
        call.set_speaker(Some("Second speaker".to_owned()));
        call.set_microphone(Some("Second microphone".to_owned()));

        // The engine's microphone channel is the same one it was, and it is still carrying frames.
        tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
            .await
            .expect("the microphone channel survived the changes")
            .expect("the frames are still coming");
        // Only the rebind opened a device: the camera change never touches the audio at all.
        let started = std::time::Instant::now();
        while speaker.opened_count() < 2 && started.elapsed() < std::time::Duration::from_secs(5) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            speaker.opened(),
            vec![None, Some("Second speaker".to_owned())],
            "the camera changes opened no audio device"
        );
        call.cleanup();
    }

    /// A video direction with a camera that reads as running, without a device behind it.
    ///
    /// Enough to drive the paths that only ask whether a camera is capturing, which is what a
    /// machine with no camera cannot exercise otherwise.
    fn pipeline_with_a_running_camera() -> VideoPipeline {
        let (ticks, _ticks_rx) = async_channel::bounded::<VideoTick>(2);
        let (remote_tx, _remote_rx) = async_channel::bounded::<VideoFrame>(8);
        let (_frames, timed) = async_channel::bounded::<TimedVideoFrame>(4);
        // No thread behind it: its end of the channel is already dropped.
        let (_done, finished) = std::sync::mpsc::channel();
        VideoPipeline {
            ticks,
            remote_tx,
            camera: Some(CameraCapture {
                timed,
                running: Arc::new(AtomicBool::new(true)),
                keyframe: Arc::new(AtomicBool::new(false)),
                finished,
            }),
            device: None,
        }
    }

    /// Opening or closing a camera tells the speaker the audio graph is relinking.
    ///
    /// A camera is a second stream on the same graph the call's audio runs on, and the graph
    /// relinks every stream on it while the new one settles. Without this the sink is judged stalled
    /// through that pause and recovers by throwing the peer's audio away, which is the reported
    /// "camera on and the other side went quiet" with everything else still working.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_camera_change_tells_the_speaker_the_graph_is_relinking() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, _speaker) =
            crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (output, _frames) = AudioOutput::spawn(None);
        call.speaker = Some(output);
        call.video = true;
        call.video_pipe = Some(pipeline_with_a_running_camera());
        assert!(
            !call.speaker.as_ref().is_some_and(AudioOutput::relink_open),
            "nothing has relinked yet"
        );
        // Whether this machine has a tenth camera is not what is under test: the stream is
        // torn down either way, so the sink is told either way.
        let _ = call.set_camera_device(Some("9".to_owned())).await;
        assert!(
            call.speaker.as_ref().is_some_and(AudioOutput::relink_open),
            "switching the camera tells the sink the graph is relinking"
        );
        call.cleanup();
    }

    /// An upgrade the peer refused leaves no half-installed video behind it.
    ///
    /// The call would otherwise draw itself as video with a camera holding the device, the frames
    /// going to nobody, and the peer never having been asked, so the next press of the camera would
    /// have to undo a state the user never asked for.
    #[test]
    fn a_refused_upgrade_gives_back_everything_it_built() {
        let mut call = dialing();
        call.video = true;
        call.camera = Some("0".to_owned());
        call.camera_wanted = true;
        call.abandon_video();
        assert!(!call.is_video(), "the call is a voice call again");
        assert!(call.camera.is_none(), "and it holds no camera");
        assert!(!call.camera_wanted, "so none is wanted");
        assert!(call.video_pipe.is_none(), "and the pipeline is gone");
    }

    /// The engine's mute gate, as the engine's own media backend applies it to the frames a call
    /// hands it.
    ///
    /// The zeroing is the engine's rather than this module's: its media backend reads the call's
    /// microphone channel and replaces every frame that is exactly the engine's 60 ms one with zeros
    /// while the call is muted, passing anything else through untouched. The engine then turns an
    /// all-zero frame into a one-byte MLOW DTX comfort-noise packet instead of a gap, so the media
    /// stream stays fed and the peer does not re-negotiate the transport.
    ///
    /// Spelled out here because that type is private to the engine, and what the test below drives
    /// into it is the real microphone path: a frame that is not the engine's own size is the one
    /// thing that would leak the microphone while the peer is told this side is muted, and it fails
    /// here.
    fn engine_mute_gate(frame: &mut [i16], muted: bool) {
        if muted && frame.len() == crate::call_audio::FRAME_SAMPLES {
            frame.fill(0);
        }
    }

    /// A mute really silences what the engine is fed, without silencing the microphone or stopping
    /// the stream, and an unmute brings the audio back on the channel the call already had.
    ///
    /// The device is a test one, so this holds on a build host with no sound card and without a
    /// peer: what is under test is the audio path a call runs, not a second implementation of it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_mute_silences_the_engine_feed_and_an_unmute_brings_it_back() {
        // A ramp, so a frame that is not silent carries a signal nobody could mistake for silence.
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, _speaker) =
            crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let (input, frames) = AudioInput::spawn(None);
        let mut heard: Vec<Vec<i16>> = Vec::new();
        let mut quiet: Vec<Vec<i16>> = Vec::new();
        let mut microphone_while_muted: Vec<i16> = Vec::new();
        for step in 0..3 {
            let muted = step == 1;
            for _ in 0..4 {
                let mut frame =
                    tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
                        .await
                        .expect("the engine's channel stays open")
                        .expect("the microphone keeps delivering");
                assert_eq!(
                    frame.len(),
                    crate::call_audio::FRAME_SAMPLES,
                    "a frame the engine cannot size is one its mute gate would pass through"
                );
                if muted {
                    microphone_while_muted.extend_from_slice(&frame);
                }
                engine_mute_gate(&mut frame, muted);
                if muted {
                    quiet.push(frame);
                } else {
                    heard.push(frame);
                }
            }
        }
        let opens = input.opens.load(Ordering::Relaxed);
        drop(input);

        // Unmuted, the engine is fed what the microphone delivered.
        assert!(
            heard.iter().flatten().any(|sample| *sample != 0),
            "the engine hears the microphone while the call is not muted"
        );
        // Muted, everything it is fed is silent ...
        assert_eq!(quiet.len(), 4, "frames kept arriving while muted");
        assert!(
            quiet.iter().flatten().all(|sample| *sample == 0),
            "every frame the engine is fed while muted is silent"
        );
        // ... while the microphone behind the gate is still delivering, which is what makes the
        // silence the mute rather than a device that went away.
        assert!(
            microphone_while_muted.iter().any(|sample| *sample != 0),
            "the microphone is still delivering while muted"
        );
        // The reader was never restarted by any of it: one open, and the frames above are still
        // coming from it.
        assert_eq!(opens, 1, "the mute and the unmute restarted no reader");
        // And an unmute needs no restart either, which is the frame above.
        assert!(
            heard.len() == 8,
            "the microphone came back on the same channel after the unmute"
        );
    }

    /// Waits for something a pump does on its own task, so a test does not assert before the
    /// rebind it asked for has had a chance to happen. Returns as soon as the answer is yes, and
    /// leaves the last answer for the caller to assert on.
    async fn until(mut ready: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if ready() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        ready()
    }

    /// A call that is up stays up through a mute: the engine's channel keeps carrying whole frames,
    /// a device change made while muted rebinds the streams rather than restarting the call, and a
    /// camera that cannot be turned on leaves the audio exactly as it was.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_muted_call_keeps_its_audio_and_its_phase() {
        let samples: Vec<f32> = (0..960).map(|sample| sample as f32 / 1_920.0).collect();
        let (_devices, speaker) = crate::call_audio::fake::install(samples, 2, 48_000, (48_000, 2));
        let mut call = dialing();
        let (input, frames) = AudioInput::spawn(None);
        let (output, _tx) = AudioOutput::spawn(None);
        call.mic = Some(input);
        call.speaker = Some(output);
        call.signaling(&accept()).expect("the peer answered");
        call.media(&CallEvent::RelayAllocated);
        assert_eq!(call.phase(), CallPhase::Active, "the call is up");

        // Muted, and everything the screen can do meanwhile is done to it.
        call.set_microphone(Some("Second microphone".to_owned()));
        call.set_speaker(Some("Second speaker".to_owned()));
        call.set_camera_device(Some("9".to_owned()))
            .await
            .expect("a camera can be selected");
        assert!(
            call.set_camera(true).await.is_err(),
            "a call with no video refuses the camera rather than ending the call"
        );

        let mut quiet: Vec<Vec<i16>> = Vec::new();
        let mut heard: Vec<Vec<i16>> = Vec::new();
        for step in 0..2 {
            let muted = step == 0;
            for _ in 0..3 {
                let mut frame =
                    tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
                        .await
                        .expect("the engine's channel stays open through the mute")
                        .expect("the microphone keeps delivering");
                assert_eq!(frame.len(), crate::call_audio::FRAME_SAMPLES);
                engine_mute_gate(&mut frame, muted);
                if muted {
                    quiet.push(frame);
                } else {
                    heard.push(frame);
                }
            }
        }
        assert_eq!(quiet.len(), 3);
        assert!(
            quiet.iter().flatten().all(|sample| *sample == 0),
            "every frame the engine is fed while muted is silent"
        );
        assert!(
            heard.iter().flatten().any(|sample| *sample != 0),
            "unmuting brings the microphone back"
        );
        // Neither the mute, nor the device changes, nor the refused camera moved the call.
        assert_eq!(call.phase(), CallPhase::Active, "the call is still up");
        // A rebind is the pump's own work, on its own task, so the device it opened is waited for
        // rather than assumed to be there the moment the reader asked for it.
        assert!(
            until(|| speaker
                .opened()
                .iter()
                .any(|device| device.as_deref() == Some("Second speaker")))
            .await,
            "the speaker device change was a rebind"
        );
        // The device change opened the microphone again behind the very channel above, which is
        // what a rebind is: the read moved, the engine's port did not.
        assert!(
            until(|| call
                .mic
                .as_ref()
                .expect("the call still has its microphone")
                .opens
                .load(Ordering::Relaxed)
                == 2)
            .await,
            "one open for the call and one for the device change"
        );
        call.cleanup();
    }

    /// Opens this machine's camera and drives the pipeline the way a call does: start, pause,
    /// resume, and end. A machine with no camera has nothing to drive and says so.
    /// `cargo test --lib -- --ignored --nocapture calls::tests::hardware`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "opens this machine's real camera"]
    async fn hardware_pipeline_starts_pauses_and_stops_the_camera() {
        let Some((device, label)) = crate::camera::cameras().into_iter().next() else {
            eprintln!("no camera on this machine: nothing to drive");
            return;
        };
        eprintln!("driving {device} ({label})");
        let (mut pipe, ticks) = VideoPipeline::start(Some(device.clone()))
            .await
            .expect("the camera opens");
        assert!(pipe.camera_running(), "a started camera reads as on");
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ticks.recv())
            .await
            .expect("the camera delivers within five seconds")
            .expect("the preview channel is open");
        assert!(matches!(frame, VideoTick::Local(_)), "our own preview");
        pipe.pause_camera();
        assert!(!pipe.camera_running(), "a pause reads as off at once");
        pipe.resume_camera(Some(device))
            .await
            .expect("the camera starts again");
        assert!(pipe.camera_running(), "a resumed camera reads as on again");
        pipe.shutdown();
        assert!(
            !pipe.camera_running(),
            "a call that has ended leaves no camera running"
        );
    }

    /// A video call goes on without a camera that cannot be opened: the other side's picture
    /// still arrives, and our own camera reads as off.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_video_call_goes_on_without_a_camera_it_cannot_open() {
        let (pipe, _ticks) = VideoPipeline::start(Some("no such camera".to_owned()))
            .await
            .expect("the decoder side starts");
        assert!(!pipe.camera_running(), "the camera reads as off");
    }

    fn snapshot(phase: CallPhase, camera_on: bool) -> CallUpdate {
        CallUpdate {
            generation: 7,
            chat: "15551234567@s.whatsapp.net".to_owned(),
            direction: CallDirection::Outgoing,
            video: true,
            phase,
            started: None,
            muted: false,
            camera_on,
            remote_video: false,
            video_requested: false,
            outcome: None,
            peer_audio: None,
            lost_devices: Vec::new(),
            microphone: None,
            speaker: None,
            camera: None,
        }
    }

    #[test]
    fn a_call_with_no_camera_named_opens_with_the_machines_own() {
        // No camera picked is what the picker calls "First camera". For a camera
        // that has to mean the first one the machine reports, or a first video call could never
        // start: there is no system default camera to fall back on.
        let resolved = resolve_devices(&machine(), None, None, None);
        assert_eq!(resolved.camera.as_deref(), Some("0"));
        assert!(
            resolved.lost_devices.is_empty(),
            "nothing moved, so the screen has nothing to say"
        );
        // A machine that reports no camera at all still resolves to none, which is what makes the
        // video pipeline refuse the call rather than offer the peer an empty stream.
        let none = resolve_devices(&DeviceList::default(), None, None, None);
        assert_eq!(none.camera, None);
    }

    /// Turning video on mid-call reads the default the same way a call's startup does.
    ///
    /// The two paths that change a camera while a call is up — upgrading a voice call to video and
    /// picking a device in the call screen — used to pass the stored `None` straight to
    /// `VideoPipeline::start`, which rejects it. A fresh install could then not turn its camera on
    /// at all, even with a working camera attached, while starting a video call worked fine. Both
    /// now go through `default_camera`, so the reading is the same one `resolve_devices` applies.
    #[test]
    fn a_camera_turned_on_mid_call_opens_the_machines_own() {
        let machine = machine();
        assert_eq!(
            default_camera(&machine, None).as_deref(),
            Some("0"),
            "the picker's default means the first camera the machine reports"
        );
        // A camera that is still there is kept.
        assert_eq!(
            default_camera(&machine, Some("0".to_owned())).as_deref(),
            Some("0")
        );
        // One the machine no longer has falls back rather than failing the switch.
        assert_eq!(
            default_camera(&machine, Some("7".to_owned())).as_deref(),
            Some("0")
        );
        // No camera named and none reported stays none, so the caller refuses rather than invents.
        assert_eq!(default_camera(&DeviceList::default(), None), None);
        // An empty list means discovery could not run, not that nothing is plugged in: the choice
        // stands rather than being silently swapped for nothing.
        assert_eq!(
            default_camera(&DeviceList::default(), Some("3".to_owned())).as_deref(),
            Some("3")
        );
    }

    /// A peer who turns their camera off stops being reported as sending video.
    ///
    /// `remote_video` only ever became true, and nothing turned it back, so a peer who stopped
    /// their camera left the call reported as carrying video and the decoder's last frame on screen
    /// as if it were live. The peer's own `<video state=N>` signaling is what says otherwise.
    #[test]
    fn a_peer_that_stops_its_camera_stops_being_reported_as_sending_video() {
        use whatsapp_rust::voip::VideoState;
        let peer = |state| CallEvent::PeerVideoStateChanged {
            source: creator(),
            call_creator: creator(),
            state,
            orientation: None,
            upgrade_token: None,
        };
        let mut call = dialing();
        call.video = true;
        assert!(!call.remote_video);
        assert!(!call.remote_video_stopped(), "nothing to stop");
        assert!(call.saw_remote_video());
        assert!(call.remote_video);
        // The peer's own state saying video is inactive is what clears the flag.
        let update = call.media(&peer(VideoState::Stopped));
        assert!(update.is_some(), "the screen is told the picture stopped");
        assert!(
            !call.remote_video,
            "a stopped peer camera is not a live picture"
        );
        // A repeated inactive state publishes nothing further.
        assert!(
            call.media(&peer(VideoState::Disabled)).is_none(),
            "only a change is published"
        );
        // A state that is still active leaves it alone.
        call.saw_remote_video();
        assert!(
            call.media(&peer(VideoState::Enabled)).is_none(),
            "an active state is not a stop"
        );
        assert!(call.remote_video, "the picture is still there");
    }

    /// A peer who turns their camera on in a voice call asks; the screen says so, and a
    /// withdrawn request is forgotten.
    #[test]
    fn a_peer_asking_for_video_is_shown_until_it_withdraws() {
        use whatsapp_rust::voip::{VideoState, VideoUpgradeToken};
        let token = VideoUpgradeToken::builder().generation(1).epoch(1).build();
        let mut call = dialing();
        let asked = call
            .media(&CallEvent::PeerVideoStateChanged {
                source: creator(),
                call_creator: creator(),
                state: VideoState::UpgradeRequestV2,
                orientation: None,
                upgrade_token: Some(token),
            })
            .expect("the request reaches the screen");
        assert!(asked.video_requested);
        assert!(!asked.video, "asking is not yet a video call");
        let withdrawn = call
            .media(&CallEvent::PeerVideoStateChanged {
                source: creator(),
                call_creator: creator(),
                state: VideoState::UpgradeCancel,
                orientation: None,
                upgrade_token: None,
            })
            .expect("the withdrawal reaches the screen");
        assert!(!withdrawn.video_requested);
    }

    #[test]
    fn the_devices_a_person_picked_are_the_ones_a_call_opens_with() {
        let machine = machine();
        let resolved = resolve_devices(
            &machine,
            Some(machine.microphones[0].id.clone()),
            Some(machine.speakers[0].id.clone()),
            Some("0".to_owned()),
        );
        assert_eq!(resolved.microphone, Some(machine.microphones[0].id.clone()));
        assert_eq!(resolved.speaker, Some(machine.speakers[0].id.clone()));
        assert_eq!(resolved.camera.as_deref(), Some("0"));
        assert!(
            resolved.lost_devices.is_empty(),
            "nothing moved, so the screen has nothing to say"
        );
    }

    #[test]
    fn a_headset_that_is_gone_falls_back_to_the_default_and_is_reported() {
        let resolved = resolve_devices(
            &machine(),
            Some("bluez_input.AC_12_34_56".to_owned()),
            Some("bluez_output.AC_12_34_56".to_owned()),
            Some("7".to_owned()),
        );
        assert_eq!(
            resolved.microphone, None,
            "the microphone moves to the default"
        );
        assert_eq!(resolved.speaker, None, "and so does the speaker");
        assert_eq!(
            resolved.camera.as_deref(),
            Some("0"),
            "a camera the machine does not have falls back to the one it does"
        );
        assert_eq!(
            resolved.lost_devices,
            vec![
                LostDevice {
                    kind: DeviceKind::Microphone,
                    name: "bluez_input.AC_12_34_56".to_owned(),
                },
                LostDevice {
                    kind: DeviceKind::Speaker,
                    name: "bluez_output.AC_12_34_56".to_owned(),
                },
                LostDevice {
                    kind: DeviceKind::Camera,
                    name: "7".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn an_empty_list_is_a_broken_tool_rather_than_an_empty_machine() {
        let resolved = resolve_devices(
            &DeviceList::default(),
            Some("bluez_input.AC".to_owned()),
            Some("bluez_output.AC".to_owned()),
            Some("0".to_owned()),
        );
        assert_eq!(resolved.microphone.as_deref(), Some("bluez_input.AC"));
        assert_eq!(resolved.speaker.as_deref(), Some("bluez_output.AC"));
        assert_eq!(resolved.camera.as_deref(), Some("0"));
        assert!(
            resolved.lost_devices.is_empty(),
            "a device list that could not be read says nothing about the devices"
        );
    }

    #[test]
    fn a_device_is_named_the_way_the_picker_named_it() {
        let machine = machine();
        assert_eq!(
            device_name(&machine, DeviceKind::Camera, "0"),
            "Laptop Camera"
        );
        assert_eq!(
            device_name(
                &machine,
                DeviceKind::Microphone,
                "alsa_input.usb-Generic_USB_Headset-00.analog-mono"
            ),
            "USB Headset Microphone"
        );
        // Nothing in the list to name it by, which is what a device that just went away looks
        // like: the node itself is the honest answer.
        assert_eq!(
            device_name(&machine, DeviceKind::Microphone, "mic-gone"),
            "mic-gone"
        );
    }

    #[test]
    fn answering_is_not_the_same_as_dialing() {
        // An outgoing call the peer answered, and an incoming one we picked up, are both live and
        // both past the ringing stage, and neither has a running timer until the media comes up.
        for phase in [
            CallPhase::Connecting,
            CallPhase::Accepted,
            CallPhase::Active,
        ] {
            assert!(phase.is_live(), "{phase:?}");
            assert!(phase.is_connected(), "{phase:?}");
            assert!(phase.is_answered(), "{phase:?}");
        }
        for phase in [CallPhase::Dialing, CallPhase::Ringing, CallPhase::Incoming] {
            assert!(phase.is_live(), "{phase:?}");
            assert!(!phase.is_connected(), "{phase:?}");
            assert!(!phase.is_answered(), "{phase:?}");
        }
        for phase in [CallPhase::Ended, CallPhase::Failed] {
            assert!(!phase.is_live(), "{phase:?}");
            assert!(!phase.is_connected(), "{phase:?}");
        }
    }

    #[test]
    fn a_rejection_is_read_for_what_the_peer_meant() {
        assert_eq!(rejection_outcome(Some("busy")), CallOutcome::Busy);
        assert_eq!(rejection_outcome(Some("enc")), CallOutcome::Failed);
        assert_eq!(rejection_outcome(None), CallOutcome::Declined);
        // A reason we do not know is still a refusal, not a failure to connect.
        assert_eq!(rejection_outcome(Some("unknown")), CallOutcome::Declined);
    }

    #[test]
    fn a_termination_is_read_against_whether_the_call_was_up() {
        assert_eq!(
            termination_outcome(Some("accepted_elsewhere"), false),
            CallOutcome::AnsweredElsewhere
        );
        assert_eq!(
            termination_outcome(Some("rejected_elsewhere"), true),
            CallOutcome::DeclinedElsewhere
        );
        assert_eq!(
            termination_outcome(Some("timeout"), false),
            CallOutcome::NoAnswer
        );
        // A timeout on a call that was up is the line going away, not a call nobody answered: its
        // own duration is proof it was answered, and a log entry cannot say both. Reported from a
        // real nineteen-second call that ended four seconds after the camera came on.
        assert_eq!(
            termination_outcome(Some("timeout"), true),
            CallOutcome::ConnectionLost
        );
        // A plain terminate means the peer ended a call that was up, or gave up before it was.
        assert_eq!(termination_outcome(None, true), CallOutcome::Answered);
        assert_eq!(termination_outcome(None, false), CallOutcome::NoAnswer);
    }

    /// The NAL unit types in one Annex-B access unit.
    fn nal_types(unit: &[u8]) -> Vec<u8> {
        unit.windows(4)
            .enumerate()
            .filter(|(_, window)| window[..3] == [0, 0, 1])
            .filter_map(|(at, _)| unit.get(at + 3).map(|header| header & 0x1F))
            .collect()
    }

    /// What the camera hands the engine is what WhatsApp decodes: Constrained Baseline access
    /// units with their parameter sets on every keyframe, a keyframe on request, and pictures the
    /// decoder the call screen uses reads back at the size they were sent.
    #[test]
    fn the_camera_encoder_makes_access_units_a_decoder_reads() {
        use openh264::formats::YUVSource as _;
        let (width, height) = (320, 240);
        let luma: Vec<u8> = (0..width * height).map(|at| (at % width) as u8).collect();
        let chroma = vec![128u8; width * height / 4];
        let picture = Yuv420 {
            y: &luma,
            u: &chroma,
            v: &chroma,
            size: (width, height),
        };
        let mut encoder = video_encoder().expect("the encoder starts");
        let mut decoder = openh264::decoder::Decoder::new().expect("the decoder starts");
        let mut units = Vec::new();
        for frame in 0..4 {
            if frame == 3 {
                encoder.force_intra_frame();
            }
            units.push(encoder.encode(&picture).expect("a frame encodes").to_vec());
        }
        let first = nal_types(&units[0]);
        assert!(
            first.contains(&7) && first.contains(&8) && first.contains(&5),
            "the first unit is a keyframe with its parameter sets: {first:?}"
        );
        let sps = units[0]
            .windows(4)
            .position(|window| window[..3] == [0, 0, 1] && window[3] & 0x1F == 7)
            .expect("a sequence parameter set");
        assert_eq!(units[0][sps + 4], 66, "Baseline profile");
        assert!(!nal_types(&units[1]).contains(&5), "then delta frames");
        assert!(
            nal_types(&units[3]).contains(&5),
            "and a keyframe when asked"
        );
        let decoded = decoder
            .decode(&units[0])
            .expect("the decoder reads it")
            .expect("a picture");
        assert_eq!(decoded.dimensions(), (width, height));
    }

    #[test]
    fn the_corner_preview_keeps_the_capture_shape() {
        assert_eq!(preview_size((1280, 720)), (320, 180));
        assert_eq!(preview_size((640, 480)), (240, 180));
        assert_eq!(preview_size((720, 1280)), (180, 320));
    }

    #[test]
    fn the_periodic_check_only_republishes_a_real_change() {
        // The worker compares each fresh snapshot with the one the UI was handed, so a camera that
        // stopped on its own reaches the screen and a call that is merely ticking does not.
        assert_eq!(
            snapshot(CallPhase::Active, false),
            snapshot(CallPhase::Active, false)
        );
        assert_ne!(
            snapshot(CallPhase::Active, true),
            snapshot(CallPhase::Active, false)
        );
        assert_ne!(
            snapshot(CallPhase::Accepted, false),
            snapshot(CallPhase::Active, false)
        );
    }

    // The state machine, driven by the stanzas and media events themselves rather than by an engine.
    //
    // Nothing here needs a peer, a relay, or a socket: the pieces under test are the rules that
    // decide what a call *is* once a stanza arrives, and those rules are the same whether the bytes
    // came off a real relay or were handed over directly. Every field is set, so a new one breaks
    // these tests rather than silently going untested.

    const CALL_ID: &str = "3EB0CALLID";
    const PEER: &str = "15551234567@s.whatsapp.net";

    /// An outgoing call as it looks right after the offer went out, with no engine attached.
    fn dialing() -> Call {
        Call {
            generation: 1,
            call_id: CALL_ID.to_owned(),
            chat: PEER.to_owned(),
            direction: CallDirection::Outgoing,
            video: false,
            phase: CallPhase::Dialing,
            started: None,
            outcome: None,
            peer_audio: None,
            incoming: None,
            handle: None,
            media_ready: false,
            mic: None,
            speaker: None,
            video_pipe: None,
            remote_video: false,
            microphone: None,
            speaker_device: None,
            camera: None,
            lost_devices: Vec::new(),
            camera_wanted: false,
            peer_video_request: None,
            began: crate::util::now(),
        }
    }

    fn creator() -> Jid {
        PEER.parse().expect("peer jid")
    }

    fn preaccept() -> CallAction {
        CallAction::PreAccept {
            call_id: CALL_ID.to_owned(),
            call_creator: creator(),
            audio: Vec::new(),
        }
    }

    fn accept() -> CallAction {
        CallAction::Accept {
            call_id: CALL_ID.to_owned(),
            call_creator: creator(),
            audio: Vec::new(),
        }
    }

    fn terminate(reason: Option<&str>) -> CallAction {
        CallAction::Terminate {
            call_id: CALL_ID.to_owned(),
            call_creator: creator(),
            reason: reason.map(str::to_owned),
            duration: None,
            audio_duration: None,
        }
    }

    #[test]
    fn ringing_is_told_from_dialing_and_dialing_shows_no_duration() {
        let mut call = dialing();
        assert!(call.phase().is_live());
        assert!(!call.phase().is_connected(), "nobody has answered yet");
        let update = call.signaling(&preaccept()).expect("the peer is ringing");
        assert_eq!(update.phase, CallPhase::Ringing);
        assert_eq!(update.started, None, "the timer has not begun");
        // A second preaccept says nothing new.
        assert!(call.signaling(&preaccept()).is_none());
    }

    #[test]
    fn a_relay_alone_never_makes_a_call_active() {
        // The allocation lands as soon as our offer is acked, while the peer's phone is still
        // ringing. Reading it as an answer would show "Connected" to a call nobody picked up.
        let mut call = dialing();
        call.signaling(&preaccept()).expect("ringing");
        assert!(
            call.media(&CallEvent::RelayAllocated).is_none(),
            "nothing is published for a call that has not been answered"
        );
        assert_eq!(call.phase(), CallPhase::Ringing);
        assert!(call.started.is_none(), "the timer still has not begun");
    }

    #[test]
    fn the_answer_and_the_media_plane_together_make_a_call_active() {
        // In this order: the peer answers before the relay is up, which is the usual one.
        let mut call = dialing();
        let connecting = call.signaling(&accept()).expect("the peer answered");
        assert_eq!(connecting.phase, CallPhase::Connecting);
        assert_eq!(
            connecting.started, None,
            "there is no media path yet, so there is no call to time"
        );
        let active = call
            .media(&CallEvent::RelayAllocated)
            .expect("the media plane is live");
        assert_eq!(active.phase, CallPhase::Active);
        assert!(active.started.is_some(), "the duration starts now");
        assert!(call.phase().is_connected());
    }

    #[test]
    fn the_answer_arriving_second_still_makes_a_call_active() {
        // In this order: the relay came up first, then the peer answered.
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        let update = call.signaling(&accept()).expect("the peer answered");
        assert_eq!(update.phase, CallPhase::Active);
        assert!(update.started.is_some());
    }

    #[test]
    fn a_stanza_for_another_call_is_left_alone() {
        let mut call = dialing();
        let other = CallAction::Accept {
            call_id: "someone-elses-call".to_owned(),
            call_creator: creator(),
            audio: Vec::new(),
        };
        assert!(call.signaling(&other).is_none());
        assert_eq!(call.phase(), CallPhase::Dialing);
    }

    #[test]
    fn a_rejection_says_which_kind_it_was_and_releases_the_call() {
        for (reason, expected) in [
            (Some("busy"), CallOutcome::Busy),
            (Some("enc"), CallOutcome::Failed),
            (None, CallOutcome::Declined),
        ] {
            let mut call = dialing();
            call.media(&CallEvent::RelayAllocated);
            let update = call
                .signaling(&CallAction::Reject {
                    call_id: CALL_ID.to_owned(),
                    call_creator: creator(),
                    reason: reason.map(str::to_owned),
                })
                .expect("the peer rejected the call");
            assert_eq!(update.phase, CallPhase::Failed);
            assert_eq!(update.outcome, Some(expected));
            assert!(call.handle.is_none(), "the media is released");
            assert!(
                call.started.is_none(),
                "a call that was never answered has no length"
            );
        }
    }

    #[test]
    fn a_peer_who_never_answered_reads_as_no_answer_and_a_lost_line_as_connection_lost() {
        let mut call = dialing();
        call.signaling(&preaccept()).expect("ringing");
        let update = call
            .signaling(&terminate(Some("timeout")))
            .expect("the peer gave up");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));

        // A call that was up and lost its media is a different thing from one that never came up.
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        call.signaling(&accept()).expect("the peer answered");
        let update = call
            .media(&CallEvent::Closed(
                whatsapp_rust::voip_control::MediaCloseReason::RelayDisconnected,
            ))
            .expect("the media is gone");
        assert_eq!(update.phase, CallPhase::Ended);
        assert_eq!(update.outcome, Some(CallOutcome::ConnectionLost));
    }

    #[test]
    fn a_call_that_had_been_up_ends_as_answered_with_its_timer_running() {
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        call.signaling(&accept()).expect("the peer answered");
        // The length counts from the moment the call became active, not from the button press, and
        // the monotonic start is what carries it.
        let started = call.started.expect("an active call has started");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let update = call.signaling(&terminate(None)).expect("the peer hung up");
        assert_eq!(update.phase, CallPhase::Ended);
        assert_eq!(update.outcome, Some(CallOutcome::Answered));
        assert_eq!(
            update.started,
            Some(started),
            "an answered call keeps its start"
        );
    }

    #[test]
    fn the_call_this_one_resolved_elsewhere_reads_as_the_right_kind_of_ending() {
        // An outgoing call answered on another of the account's devices: it was not ours to have,
        // and it was not a call nobody answered either.
        let mut call = dialing();
        let update = call
            .resolved_elsewhere(None)
            .expect("the other device took the call");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));

        // A ringing incoming call resolved elsewhere: this device never picked it up.
        let ringing = || {
            let mut incoming = dialing();
            incoming.direction = CallDirection::Incoming;
            incoming.phase = CallPhase::Incoming;
            incoming
        };
        let update = ringing()
            .resolved_elsewhere(None)
            .expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::Missed));

        // The phone took it, or turned it down, while it rang here: said as such.
        let update = ringing()
            .resolved_elsewhere(Some(CallOutcome::AnsweredElsewhere))
            .expect("answered on the phone");
        assert_eq!(update.outcome, Some(CallOutcome::AnsweredElsewhere));
        let update = ringing()
            .resolved_elsewhere(Some(CallOutcome::DeclinedElsewhere))
            .expect("declined on the phone");
        assert_eq!(update.outcome, Some(CallOutcome::DeclinedElsewhere));

        // A call that was up and then resolved elsewhere was answered, just not here.
        let mut live = dialing();
        live.media(&CallEvent::RelayAllocated);
        live.signaling(&accept()).expect("the peer answered");
        let update = live.resolved_elsewhere(None).expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Ended);
        // The other device answered, so this is neither an answered call here nor one nobody
        // answered: it is its own kind of ending, and the outcome is what says so rather than the
        // local timer, which did start.
        assert_eq!(update.outcome, Some(CallOutcome::AnsweredElsewhere));
        assert!(
            live.started.is_some(),
            "the call had been live here as well"
        );
    }

    #[test]
    fn a_call_resolved_elsewhere_before_it_is_active_is_not_answered_elsewhere() {
        // The peer answered and the media plane is still negotiating, so this device never reached
        // Active: a resolve now is not a call another device took with a zero local duration.
        let mut call = dialing();
        call.signaling(&accept()).expect("the peer answered");
        assert_eq!(call.phase(), CallPhase::Connecting);
        let update = call.resolved_elsewhere(None).expect("resolved elsewhere");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));
        assert!(call.started.is_none(), "never answered here");
    }

    #[tokio::test]
    async fn a_call_that_is_over_keeps_the_ending_it_recorded() {
        let mut call = dialing();
        call.media(&CallEvent::RelayAllocated);
        let rejected = call
            .signaling(&CallAction::Reject {
                call_id: CALL_ID.to_owned(),
                call_creator: creator(),
                reason: None,
            })
            .expect("the peer rejected the call");
        assert_eq!(rejected.outcome, Some(CallOutcome::Declined));

        // The terminate a rejecting peer sends next must not relabel the call as answered
        // nowhere: the log keeps the reason the call really ended.
        assert!(call.signaling(&terminate(Some("timeout"))).is_none());
        assert_eq!(call.outcome, Some(CallOutcome::Declined));
        let update = call.hangup(None).await;
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::Declined));

        // The media plane failing under a call that is already over says nothing new either.
        assert!(call.media(&CallEvent::RelayAllocateFailed(1)).is_none());
        assert!(call.media_ended().is_none());
    }

    #[test]
    fn a_video_call_is_refused_rather_than_quietly_downgraded() {
        // A voice call needs no camera, so no pipeline is not a failure.
        assert!(require_video(false, false).is_ok());
        assert!(require_video(false, true).is_ok());
        assert!(require_video(true, true).is_ok());
        // Video was asked for and did not start: the call is refused, not turned into voice.
        assert!(require_video(true, false).is_err());
    }

    #[test]
    fn a_peer_terminate_before_the_call_is_active_is_not_an_answered_call() {
        // The peer answered, so we are past ringing, but the media plane never came up: the call
        // was never Active, so a terminate here is not a call of zero seconds that was answered.
        let mut call = dialing();
        call.signaling(&accept()).expect("the peer answered");
        assert_eq!(call.phase(), CallPhase::Connecting);
        assert!(call.started.is_none(), "no media path, so no duration");
        let update = call.signaling(&terminate(None)).expect("the peer hung up");
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::NoAnswer));
    }

    #[test]
    fn media_that_disappears_before_the_call_is_active_is_not_an_answered_call() {
        let mut call = dialing();
        call.signaling(&accept()).expect("the peer answered");
        assert_eq!(call.phase(), CallPhase::Connecting);
        let update = call
            .media(&CallEvent::Closed(
                whatsapp_rust::voip_control::MediaCloseReason::RelayDisconnected,
            ))
            .expect("the media is gone");
        // The cause is still a lost line, but the call never came up: it is Failed, not Ended, and
        // it is not booked as an answered call of zero seconds.
        assert_eq!(update.phase, CallPhase::Failed);
        assert_eq!(update.outcome, Some(CallOutcome::ConnectionLost));
        assert!(call.started.is_none(), "the call never came up");
    }
}

/// The device discovery, on the machine it is running on.
///
/// Ignored by default: it reads this computer's real audio devices, so its result depends on the
/// hardware. Run it with `cargo test --lib -- --ignored --nocapture calls::hardware_tests` to see
/// what a call would be offered, which is how a report about a device that is missing from the
/// pickers gets answered.
#[cfg(test)]
mod hardware_tests {
    #[test]
    #[ignore = "reads this machine's real devices"]
    fn the_machine_this_runs_on_lists_its_devices() {
        let list = super::devices();
        eprintln!("microphones: {:#?}", list.microphones);
        eprintln!("speakers: {:#?}", list.speakers);
        eprintln!("cameras: {:#?}", list.cameras);
        for device in list.microphones.iter().chain(&list.speakers) {
            assert!(
                !device.id.is_empty(),
                "a device is named by what the platform reports"
            );
            assert!(
                !device.id.ends_with(".monitor"),
                "a monitor of a sink is not an input anybody can speak into"
            );
        }
        for camera in &list.cameras {
            assert!(
                !camera.id.is_empty(),
                "a camera has an id: {}",
                camera.label
            );
        }
    }
}
