//! Transcribing voice messages on this computer with whisper.cpp.
//!
//! WhatsApp sends linked devices no transcript: the phone makes its own. Here
//! the audio stays on this computer, as on the phone. The only new traffic is
//! the model, downloaded once from Hugging Face, through the proxy in
//! Settings, and checked against its SHA-256 before it is used.
//!
//! One thread transcribes one message at a time, in the order they were
//! asked for; each can be cancelled while it waits, downloads or runs. The
//! interface reads each message's progress with [`Transcriber::progress`] and
//! takes finished transcripts with [`Transcriber::take_finished`]; the backend
//! keeps them in the archive.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::backend::Waker;
use crate::i18n::tr;
use crate::model::ChatId;

/// whisper.cpp's sample rate.
const RATE: usize = 16_000;
/// How long a loaded model stays in memory with nothing left to transcribe.
const IDLE: Duration = Duration::from_secs(120);

/// A Whisper model ZapFast offers, all multilingual.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    Base,
    #[default]
    Small,
    Medium,
}

impl Model {
    pub const ALL: [Self; 3] = [Self::Base, Self::Small, Self::Medium];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Base => "ggml-base.bin",
            Self::Small => "ggml-small.bin",
            Self::Medium => "ggml-medium.bin",
        }
    }

    /// The file's size in bytes.
    pub fn size(self) -> u64 {
        match self {
            Self::Base => 147_951_465,
            Self::Small => 487_601_967,
            Self::Medium => 1_533_763_059,
        }
    }

    /// The file's SHA-256, as Hugging Face lists it.
    fn sha256(self) -> &'static str {
        match self {
            Self::Base => "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
            Self::Small => "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            Self::Medium => "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        }
    }

    fn url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }

    /// The model's name in the interface.
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "Base",
            Self::Small => "Small",
            Self::Medium => "Medium",
        }
    }
}

/// The languages Settings offers to fix, as Whisper's codes: the interface's
/// own languages. Any other is still found when the language is detected.
pub const LANGUAGES: [&str; 8] = ["pt", "en", "es", "fr", "de", "it", "ru", "zh"];

/// A language's name in the interface language, from Whisper's code.
pub fn language_name(code: &str) -> String {
    let name = match code {
        "pt" => tr("Portuguese"),
        "en" => tr("English"),
        "es" => tr("Spanish"),
        "fr" => tr("French"),
        "de" => tr("German"),
        "it" => tr("Italian"),
        "ru" => tr("Russian"),
        "zh" => tr("Chinese"),
        _ => {
            // Whisper's own English name, as "portuguese".
            let full = whisper_rs::get_lang_id(code)
                .and_then(whisper_rs::get_lang_str_full)
                .unwrap_or(code);
            let mut chars = full.chars();
            return chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
        }
    };
    name.to_owned()
}

/// A finished transcript, as the archive keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    /// Whisper's code for the language heard, like "pt".
    pub language: String,
    /// The model's file name.
    pub model: String,
    /// Unix seconds.
    pub at: i64,
}

/// A message: its chat and its id.
pub type Key = (ChatId, String);

/// Where a message's transcription is.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Behind another message.
    Waiting,
    /// Downloading the model first.
    Downloading {
        received: u64,
        total: u64,
    },
    /// Running, in percent.
    Transcribing(u8),
    Failed(String),
}

struct Job {
    key: Key,
    audio: PathBuf,
    model: Model,
    /// A Whisper code, or `None` to detect the language.
    language: Option<String>,
    cancel: Arc<AtomicBool>,
}

enum Request {
    Transcribe(Job),
    /// Deletes every downloaded model, once none is in use.
    DeleteModels,
}

#[derive(Default)]
struct Shared {
    progress: HashMap<Key, Progress>,
    cancels: HashMap<Key, Arc<AtomicBool>>,
    finished: Vec<(Key, Transcript)>,
}

/// The transcription thread and what it reports.
pub struct Transcriber {
    folder: PathBuf,
    waker: Waker,
    shared: Arc<Mutex<Shared>>,
    requests: Option<mpsc::Sender<Request>>,
}

impl Transcriber {
    /// Keeps models in `folder`. The thread starts with the first request.
    pub fn new(folder: PathBuf, waker: Waker) -> Self {
        Self {
            folder,
            waker,
            shared: Arc::default(),
            requests: None,
        }
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Transcribes the audio file of a message, unless it is already waiting
    /// or running. A failed one starts again.
    pub fn transcribe(&mut self, key: Key, audio: PathBuf, model: Model, language: Option<String>) {
        {
            let mut shared = self.shared();
            if matches!(
                shared.progress.get(&key),
                Some(Progress::Waiting | Progress::Downloading { .. } | Progress::Transcribing(_))
            ) {
                return;
            }
            shared.progress.insert(key.clone(), Progress::Waiting);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.shared()
            .cancels
            .insert(key.clone(), Arc::clone(&cancel));
        let job = Job {
            key: key.clone(),
            audio,
            model,
            language,
            cancel,
        };
        if let Err(error) = self.send(Request::Transcribe(job)) {
            self.shared().progress.insert(key, Progress::Failed(error));
        }
    }

    /// Stops a message's transcription, or forgets its failure.
    pub fn cancel(&mut self, key: &Key) {
        let mut shared = self.shared();
        if let Some(cancel) = shared.cancels.remove(key) {
            cancel.store(true, Ordering::Relaxed);
        }
        shared.progress.remove(key);
    }

    /// Where a message's transcription is, if it was asked for and has not
    /// finished.
    pub fn progress(&self, key: &Key) -> Option<Progress> {
        self.shared().progress.get(key).cloned()
    }

    /// Whether anything is waiting, downloading or running.
    pub fn busy(&self) -> bool {
        self.shared()
            .progress
            .values()
            .any(|progress| !matches!(progress, Progress::Failed(_)))
    }

    /// The transcripts finished since the last call.
    pub fn take_finished(&mut self) -> Vec<(Key, Transcript)> {
        std::mem::take(&mut self.shared().finished)
    }

    /// Whether the model is downloaded.
    pub fn downloaded(&self, model: Model) -> bool {
        std::fs::metadata(self.folder.join(model.file_name()))
            .is_ok_and(|file| file.len() == model.size())
    }

    /// The downloaded models' combined size in bytes.
    pub fn downloaded_size(&self) -> u64 {
        Model::ALL
            .into_iter()
            .filter(|model| self.downloaded(*model))
            .map(Model::size)
            .sum()
    }

    /// Deletes the downloaded models, after the transcription running now.
    pub fn delete_models(&mut self) {
        if let Err(error) = self.send(Request::DeleteModels) {
            log::warn!("could not delete the transcription models: {error}");
        }
    }

    fn send(&mut self, request: Request) -> Result<(), String> {
        if self.requests.is_none() {
            let (sender, receiver) = mpsc::channel();
            let worker = Worker {
                folder: self.folder.clone(),
                shared: Arc::clone(&self.shared),
                waker: self.waker.clone(),
                loaded: None,
            };
            std::thread::Builder::new()
                .name("transcribe".to_owned())
                .spawn(move || worker.run(&receiver))
                .map_err(|error| error.to_string())?;
            self.requests = Some(sender);
        }
        let sent = self
            .requests
            .as_ref()
            .map(|requests| requests.send(request));
        match sent {
            Some(Ok(())) => Ok(()),
            _ => {
                // The thread ended; the next request starts another.
                self.requests = None;
                Err(tr("Could not start the transcription").to_owned())
            }
        }
    }
}

/// Why a job ended without a transcript.
enum Stop {
    Cancelled,
    Failed(String),
}

impl From<String> for Stop {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

struct Worker {
    folder: PathBuf,
    shared: Arc<Mutex<Shared>>,
    waker: Waker,
    loaded: Option<(Model, whisper_rs::WhisperContext)>,
}

impl Worker {
    fn run(mut self, requests: &mpsc::Receiver<Request>) {
        // whisper.cpp logs through `log`, not onto the terminal.
        whisper_rs::install_logging_hooks();
        loop {
            let request = match requests.recv_timeout(IDLE) {
                Ok(request) => request,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Gives the model's memory back.
                    self.loaded = None;
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            match request {
                Request::Transcribe(job) => self.job(job),
                Request::DeleteModels => {
                    self.loaded = None;
                    for model in Model::ALL {
                        let path = self.folder.join(model.file_name());
                        match std::fs::remove_file(&path) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => {
                                log::warn!("could not delete {}: {error}", path.display())
                            }
                        }
                    }
                    self.waker.wake();
                }
            }
        }
    }

    fn report(&self, key: &Key, progress: Progress) {
        let mut shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        // A cancelled job no longer reports.
        if shared.progress.contains_key(key) {
            shared.progress.insert(key.clone(), progress);
        }
        drop(shared);
        self.waker.wake();
    }

    fn job(&mut self, job: Job) {
        if job.cancel.load(Ordering::Relaxed) {
            return;
        }
        let outcome = self.transcribe(&job);
        let mut shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if shared
            .cancels
            .get(&job.key)
            .is_some_and(|cancel| Arc::ptr_eq(cancel, &job.cancel))
        {
            shared.cancels.remove(&job.key);
        }
        match outcome {
            Ok(transcript) => {
                shared.progress.remove(&job.key);
                shared.finished.push((job.key, transcript));
            }
            Err(Stop::Cancelled) => {}
            Err(Stop::Failed(error)) => {
                log::warn!("transcription failed: {error}");
                if shared.progress.contains_key(&job.key) {
                    shared.progress.insert(job.key, Progress::Failed(error));
                }
            }
        }
        drop(shared);
        self.waker.wake();
    }

    fn transcribe(&mut self, job: &Job) -> Result<Transcript, Stop> {
        supported()?;
        let model_path = self.model(job)?;
        if job.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        self.report(&job.key, Progress::Transcribing(0));
        let samples = crate::audio::decode_file(&job.audio)?;
        let samples = to_whisper_rate(&samples);
        if self
            .loaded
            .as_ref()
            .is_none_or(|(model, _)| *model != job.model)
        {
            // One model in memory at a time.
            self.loaded = None;
            let context = whisper_rs::WhisperContext::new_with_params(
                &model_path,
                whisper_rs::WhisperContextParameters::default(),
            )
            .map_err(|error| {
                format!("{}: {error}", tr("Could not load the transcription model"))
            })?;
            self.loaded = Some((job.model, context));
        }
        let Some((_, context)) = &self.loaded else {
            return Err(Stop::Failed(
                tr("Could not load the transcription model").to_owned(),
            ));
        };
        let mut state = context.create_state().map_err(|error| error.to_string())?;
        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        let threads = std::thread::available_parallelism().map_or(4, |count| count.get().min(8));
        params.set_n_threads(threads as i32);
        params.set_language(Some(job.language.as_deref().unwrap_or("auto")));
        params.set_translate(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        {
            let shared = Arc::clone(&self.shared);
            let waker = self.waker.clone();
            let key = job.key.clone();
            params.set_progress_callback_safe(move |percent: i32| {
                let mut shared = shared.lock().unwrap_or_else(|p| p.into_inner());
                if shared.progress.contains_key(&key) {
                    shared.progress.insert(
                        key.clone(),
                        Progress::Transcribing(percent.clamp(0, 100) as u8),
                    );
                }
                drop(shared);
                waker.wake();
            });
        }
        // whisper-rs's own abort closure reads its box as another type, so
        // the flag goes to whisper.cpp directly. It outlives `full`, which
        // is the only caller.
        let cancel = Arc::clone(&job.cancel);
        unsafe {
            params.set_abort_callback(Some(abort_requested));
            params.set_abort_callback_user_data(Arc::as_ptr(&cancel).cast_mut().cast());
        }
        let result = state.full(params, &samples);
        drop(cancel);
        if job.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        result.map_err(|error| format!("{}: {error}", tr("Could not transcribe")))?;
        let text = state
            .as_iter()
            .filter_map(|segment| segment.to_str_lossy().ok().map(|text| text.into_owned()))
            .collect::<Vec<_>>()
            .join(" ");
        let text = tidy(&text);
        if text.is_empty() {
            return Err(Stop::Failed(tr("No speech found").to_owned()));
        }
        let language = whisper_rs::get_lang_str(state.full_lang_id_from_state())
            .or(job.language.as_deref())
            .unwrap_or_default()
            .to_owned();
        Ok(Transcript {
            text,
            language,
            model: job.model.file_name().to_owned(),
            at: crate::util::now(),
        })
    }

    /// The model's file, downloaded first if it is missing.
    fn model(&self, job: &Job) -> Result<PathBuf, Stop> {
        let path = self.folder.join(job.model.file_name());
        if std::fs::metadata(&path).is_ok_and(|file| file.len() == job.model.size()) {
            return Ok(path);
        }
        std::fs::create_dir_all(&self.folder).map_err(|error| error.to_string())?;
        let partial = self.folder.join(format!("{}.part", job.model.file_name()));
        let total = job.model.size();
        self.report(&job.key, Progress::Downloading { received: 0, total });
        let downloaded = download(job.model, &partial, &job.cancel, |received| {
            self.report(&job.key, Progress::Downloading { received, total });
        });
        match downloaded {
            Ok(()) => {
                std::fs::rename(&partial, &path).map_err(|error| error.to_string())?;
                Ok(path)
            }
            Err(stop) => {
                let _ = std::fs::remove_file(&partial);
                Err(stop)
            }
        }
    }
}

unsafe extern "C" fn abort_requested(flag: *mut std::ffi::c_void) -> bool {
    // SAFETY: the pointer is the job's cancel flag, alive while `full` runs.
    unsafe { &*flag.cast::<AtomicBool>() }.load(Ordering::Relaxed)
}

/// Stops early on a processor whisper.cpp was not built for: the build asks
/// for AVX2, FMA and F16C on x86-64, which would otherwise crash.
fn supported() -> Result<(), String> {
    #[cfg(target_arch = "x86_64")]
    if !(std::arch::is_x86_feature_detected!("avx2")
        && std::arch::is_x86_feature_detected!("fma")
        && std::arch::is_x86_feature_detected!("f16c"))
    {
        return Err(
            tr("This processor lacks the AVX2 instructions transcription needs").to_owned(),
        );
    }
    Ok(())
}

/// Downloads the model into `partial`, reporting every megabyte, and checks
/// its SHA-256.
fn download(
    model: Model,
    partial: &Path,
    cancel: &AtomicBool,
    mut received: impl FnMut(u64),
) -> Result<(), Stop> {
    let failed = |error: String| {
        Stop::Failed(format!(
            "{}: {error}",
            tr("Could not download the transcription model")
        ))
    };
    let response = crate::proxy::agent()
        .get(&model.url())
        .call()
        .map_err(|error| failed(error.to_string()))?;
    let mut body = response.into_body().into_reader();
    let mut file = std::fs::File::create(partial).map_err(|error| failed(error.to_string()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut count = 0u64;
    let mut reported = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        let read = match body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(failed(error.to_string())),
        };
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|error| failed(error.to_string()))?;
        count += read as u64;
        if count - reported >= 1 << 20 {
            reported = count;
            received(count);
        }
    }
    file.sync_all().map_err(|error| failed(error.to_string()))?;
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if count != model.size() || hex != model.sha256() {
        return Err(failed(
            tr("the file differs from the published one").to_owned(),
        ));
    }
    Ok(())
}

/// Mono 48 kHz samples at Whisper's 16 kHz: a low-pass below 8 kHz, then
/// every third sample.
pub fn to_whisper_rate(samples: &[f32]) -> Vec<f32> {
    const FACTOR: usize = crate::voice::RATE as usize / RATE;
    const TAPS: usize = 31;
    // A Hann-windowed sinc cut at 7.2 kHz.
    let taps: Vec<f32> = {
        let cutoff = 7_200.0 / crate::voice::RATE as f32;
        let middle = (TAPS / 2) as f32;
        let raw: Vec<f32> = (0..TAPS)
            .map(|index| {
                let x = index as f32 - middle;
                let sinc = if x == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
                };
                let window = 0.5
                    - 0.5 * (2.0 * std::f32::consts::PI * index as f32 / (TAPS - 1) as f32).cos();
                sinc * window
            })
            .collect();
        let sum: f32 = raw.iter().sum();
        raw.into_iter().map(|tap| tap / sum).collect()
    };
    let half = TAPS / 2;
    (0..samples.len().div_ceil(FACTOR))
        .map(|out| {
            let centre = out * FACTOR;
            taps.iter()
                .enumerate()
                .filter_map(|(index, tap)| {
                    (centre + index)
                        .checked_sub(half)
                        .and_then(|at| samples.get(at))
                        .map(|sample| sample * tap)
                })
                .sum()
        })
        .collect()
}

/// Whisper's segments as one text: single spaces, no bracketed sound tags
/// like "[Música]" standing alone.
fn tidy(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let joined = words.join(" ");
    let only_tag = joined.starts_with(['[', '('])
        && joined.ends_with([']', ')'])
        && !joined[1..].contains(['[', '(']);
    if only_tag { String::new() } else { joined }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_keeps_its_level_at_16_khz_and_a_third_of_its_length() {
        let tone: Vec<f32> = (0..48_000)
            .map(|index| (index as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48_000.0).sin())
            .collect();
        let out = to_whisper_rate(&tone);
        assert_eq!(out.len(), 16_000);
        let peak = out[100..15_900]
            .iter()
            .fold(0f32, |peak, sample| peak.max(sample.abs()));
        assert!((0.95..1.05).contains(&peak), "peak {peak}");
    }

    #[test]
    fn sounds_above_8_khz_are_filtered_out() {
        let hiss: Vec<f32> = (0..48_000)
            .map(|index| (index as f32 * 2.0 * std::f32::consts::PI * 12_000.0 / 48_000.0).sin())
            .collect();
        let out = to_whisper_rate(&hiss);
        let peak = out[100..15_900]
            .iter()
            .fold(0f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak < 0.05, "peak {peak}");
    }

    #[test]
    fn tidy_joins_segments_and_drops_a_lone_sound_tag() {
        assert_eq!(
            tidy("  Oi, tudo bem?\n  Até amanhã. "),
            "Oi, tudo bem? Até amanhã."
        );
        assert_eq!(tidy(" [Música] "), "");
        assert_eq!(tidy("(risos)"), "");
        assert_eq!(tidy("[Música] Oi [risos]"), "[Música] Oi [risos]");
    }

    #[test]
    fn models_are_listed_with_their_published_sizes() {
        assert_eq!(Model::default(), Model::Small);
        for model in Model::ALL {
            assert_eq!(model.sha256().len(), 64);
            assert!(model.url().ends_with(model.file_name()));
        }
        assert_eq!(
            serde_json::to_string(&Model::Small).unwrap(),
            "\"small\"",
            "the settings file names it"
        );
    }

    /// Downloads the Base model (148 MB) into a temporary folder and
    /// transcribes the voice message in `ZAPFAST_TRANSCRIBE_SAMPLE`:
    ///
    /// `ZAPFAST_TRANSCRIBE_SAMPLE=voice.ogg cargo test --release -- --ignored transcribes_a_real_voice_message`
    #[test]
    #[ignore = "downloads a model; needs ZAPFAST_TRANSCRIBE_SAMPLE"]
    fn transcribes_a_real_voice_message() {
        let sample = PathBuf::from(std::env::var_os("ZAPFAST_TRANSCRIBE_SAMPLE").unwrap());
        let folder = tempfile::tempdir().unwrap();
        let mut transcriber = Transcriber::new(folder.path().to_path_buf(), Waker::default());
        let key: Key = ("1@s.whatsapp.net".into(), "voice".to_owned());
        transcriber.transcribe(key.clone(), sample, Model::Base, None);
        let started = std::time::Instant::now();
        let finished = loop {
            let finished = transcriber.take_finished();
            if !finished.is_empty() {
                break finished;
            }
            if let Some(Progress::Failed(error)) = transcriber.progress(&key) {
                panic!("{error}");
            }
            assert!(started.elapsed() < Duration::from_secs(600), "too slow");
            std::thread::sleep(Duration::from_millis(100));
        };
        let (done, transcript) = &finished[0];
        assert_eq!(done, &key);
        eprintln!("{transcript:?} in {:?}", started.elapsed());
        assert!(!transcript.text.is_empty());
        assert!(transcriber.downloaded(Model::Base));
        transcriber.delete_models();
        std::thread::sleep(Duration::from_millis(500));
        assert!(!transcriber.downloaded(Model::Base));
    }

    #[test]
    fn known_languages_have_their_names() {
        assert_eq!(language_name("en"), "English");
        assert_eq!(language_name("nl"), "Dutch");
    }
}
