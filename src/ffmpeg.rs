//! Video playback through the system's `ffmpeg`, on Linux.
//!
//! The built-in player reads H.264 in MP4 with AAC-LC sound. When the
//! `ffmpeg` and `ffprobe` programs are installed, a video can play through
//! them instead: HEVC, AV1, VP9, 10-bit H.264, HE-AAC, Opus, AC-3 and the
//! rest of what FFmpeg reads. One process sends scaled RGBA frames at a
//! constant rate and another sends the sound as 48 kHz stereo samples, both
//! through pipes, so no FFmpeg library is linked and any installed version
//! works. The built-in player stays the fallback.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::Duration;

use egui::ColorImage;

/// The installed programs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// Graphics-card decoding to ask for, if any.
    pub accel: Option<Accel>,
}

/// Graphics-card video decoding FFmpeg can use, in the order it is preferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accel {
    /// NVIDIA's decoder, through CUDA.
    Nvdec,
    /// VAAPI, as AMD and Intel drivers offer it.
    Vaapi,
    /// Vulkan video decoding.
    Vulkan,
}

impl Accel {
    /// FFmpeg's name for it, after `-hwaccel`.
    fn flag(self) -> &'static str {
        match self {
            Accel::Nvdec => "cuda",
            Accel::Vaapi => "vaapi",
            Accel::Vulkan => "vulkan",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Accel::Nvdec => "NVIDIA NVDEC",
            Accel::Vaapi => "VAAPI",
            Accel::Vulkan => "Vulkan",
        }
    }
}

/// The graphics-card decoding this computer and `ffmpeg` both have:
/// NVDEC with an NVIDIA driver, then VAAPI, then Vulkan on a render node.
/// Asked once per `ffmpeg` program.
pub fn accel(tools: &Tools) -> Option<Accel> {
    static KNOWN: std::sync::Mutex<Option<(PathBuf, Option<Accel>)>> = std::sync::Mutex::new(None);
    let mut known = KNOWN.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((path, accel)) = known.as_ref()
        && *path == tools.ffmpeg
    {
        return *accel;
    }
    let listed = Command::new(&tools.ffmpeg)
        .args(["-hide_banner", "-hwaccels"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let offered = |name: &str| listed.split_whitespace().any(|word| word == name);
    let nvidia = Path::new("/proc/driver/nvidia/version").exists();
    let render_node = std::fs::read_dir("/dev/dri").is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
    });
    let accel = if nvidia && offered("cuda") {
        Some(Accel::Nvdec)
    } else if render_node && offered("vaapi") {
        Some(Accel::Vaapi)
    } else if render_node && offered("vulkan") {
        Some(Accel::Vulkan)
    } else {
        None
    };
    *known = Some((tools.ffmpeg.clone(), accel));
    accel
}

/// `ffmpeg` and `ffprobe` from the search path, on Linux outside a Flatpak
/// sandbox, which cannot see the system's programs.
pub fn tools() -> Option<Tools> {
    if !cfg!(target_os = "linux") || sandboxed() {
        return None;
    }
    Some(Tools {
        ffmpeg: find("ffmpeg")?,
        ffprobe: find("ffprobe")?,
        accel: None,
    })
}

/// Whether ZapFast runs inside a Flatpak sandbox.
pub fn sandboxed() -> bool {
    Path::new("/.flatpak-info").exists()
}

fn find(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| executable(candidate))
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

/// How this Linux distribution installs FFmpeg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    /// Arch and its derivatives, such as EndeavourOS and CachyOS.
    Pacman,
    /// Fedora, whose own `ffmpeg-free` leaves codecs out; RPM Fusion has
    /// the full build.
    Dnf,
    /// Debian and its derivatives, such as Ubuntu and Linux Mint.
    Apt,
}

impl Install {
    pub fn command(self) -> &'static str {
        match self {
            Install::Pacman => "sudo pacman -S ffmpeg",
            Install::Dnf => "sudo dnf install ffmpeg --allowerasing",
            Install::Apt => "sudo apt install ffmpeg",
        }
    }
}

/// The package manager of the running distribution, from `/etc/os-release`.
pub fn install() -> Option<Install> {
    let release = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok()?;
    install_for(&release)
}

fn install_for(release: &str) -> Option<Install> {
    let field = |key: &str| {
        release
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(|value| value.trim().trim_matches(['"', '\'']).to_ascii_lowercase())
            .unwrap_or_default()
    };
    let id = field("ID");
    let like = field("ID_LIKE");
    let names: Vec<&str> = std::iter::once(id.as_str())
        .chain(like.split_whitespace())
        .collect();
    let any = |wanted: &[&str]| names.iter().any(|name| wanted.contains(name));
    if any(&[
        "arch",
        "endeavouros",
        "cachyos",
        "manjaro",
        "garuda",
        "artix",
    ]) {
        Some(Install::Pacman)
    } else if any(&["fedora"]) {
        Some(Install::Dnf)
    } else if any(&["debian", "ubuntu", "linuxmint", "pop", "zorin"]) {
        Some(Install::Apt)
    } else {
        None
    }
}

/// What playback needs to know about a video before its first frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    pub duration: Duration,
    /// Displayed size, after any rotation the file asks for.
    pub width: u32,
    pub height: u32,
    /// Frames per second the frames are sent at.
    pub rate: f64,
}

/// Reads the first video stream's size, rotation, and frame rate.
pub fn probe(tools: &Tools, path: &Path) -> Result<Probe, String> {
    let output = Command::new(&tools.ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg("stream=width,height,avg_frame_rate,r_frame_rate:stream_side_data=rotation:stream_tags=rotate:format=duration")
        .args(["-of", "json"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("ffprobe did not start: {error}"))?;
    if !output.status.success() {
        return Err("ffprobe could not read the file".to_owned());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    parse_probe(&json)
}

fn parse_probe(json: &serde_json::Value) -> Result<Probe, String> {
    let stream = json["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .ok_or("no video stream")?;
    let number = |value: &serde_json::Value| {
        value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| *number > 0)
    };
    let width = number(&stream["width"]).ok_or("no video width")?;
    let height = number(&stream["height"]).ok_or("no video height")?;
    let rotation = stream["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|data| data["rotation"].as_f64())
        .or_else(|| stream["tags"]["rotate"].as_str()?.parse().ok())
        .unwrap_or(0.0);
    // FFmpeg turns the picture upright itself.
    let sideways = (rotation.round() as i64).rem_euclid(180) == 90;
    let (width, height) = if sideways {
        (height, width)
    } else {
        (width, height)
    };
    let rate = [&stream["avg_frame_rate"], &stream["r_frame_rate"]]
        .into_iter()
        .find_map(|value| fraction(value.as_str()?))
        .filter(|rate| (1.0..=240.0).contains(rate))
        .unwrap_or(30.0);
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|seconds| seconds.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map_or(Duration::ZERO, Duration::from_secs_f64);
    Ok(Probe {
        duration,
        width,
        height,
        rate,
    })
}

/// Reads a rate such as `30000/1001`.
fn fraction(text: &str) -> Option<f64> {
    let (numerator, denominator) = text.split_once('/')?;
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    (denominator > 0.0).then(|| numerator / denominator)
}

/// Frames of a video from a start time, `width` by `height` RGBA each, at a
/// constant rate. The process stops when this is dropped.
pub struct Frames {
    child: Child,
    stdout: ChildStdout,
    width: usize,
    height: usize,
    rate: f64,
    from: Duration,
    sent: u64,
}

/// Starts sending `path`'s frames from `from`, scaled to `width` by `height`.
pub fn frames(
    tools: &Tools,
    path: &Path,
    from: Duration,
    width: u32,
    height: u32,
    rate: f64,
) -> Result<Frames, String> {
    let mut command = Command::new(&tools.ffmpeg);
    command.args(["-nostdin", "-hide_banner", "-v", "error"]);
    // Frames the graphics card decodes come back to memory for the pipe;
    // where it cannot decode a file, FFmpeg decodes it on the processor.
    if let Some(accel) = tools.accel {
        command.args(["-hwaccel", accel.flag()]);
    }
    let mut child = command
        .args(["-ss", &seconds(from)])
        .arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-an", "-sn", "-dn"])
        .args(["-vf", &format!("scale={width}:{height}")])
        // An output rate repeats or drops frames to keep it constant, so
        // each frame's time follows from its number.
        .args(["-r", &format!("{rate}")])
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("ffmpeg did not start: {error}"))?;
    let stdout = child.stdout.take().ok_or("ffmpeg has no output")?;
    Ok(Frames {
        child,
        stdout,
        width: width as usize,
        height: height as usize,
        rate,
        from,
        sent: 0,
    })
}

impl Iterator for Frames {
    type Item = (Duration, ColorImage);

    fn next(&mut self) -> Option<Self::Item> {
        let mut rgba = vec![0; self.width * self.height * 4];
        self.stdout.read_exact(&mut rgba).ok()?;
        let at = self.from + Duration::from_secs_f64(self.sent as f64 / self.rate);
        self.sent += 1;
        Some((
            at,
            ColorImage::from_rgba_unmultiplied([self.width, self.height], &rgba),
        ))
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sound samples of the video as 48 kHz stereo, fed to rodio.
pub struct Samples {
    chunks: Receiver<Vec<f32>>,
    chunk: Vec<f32>,
    at: usize,
}

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Samples per chunk read from the pipe: about 85 ms.
const CHUNK: usize = 8192;

/// Starts sending `path`'s sound from `from`. A video without sound ends at
/// once, and plays silently.
pub fn samples(tools: &Tools, path: &Path, from: Duration) -> Result<Samples, String> {
    let mut child = Command::new(&tools.ffmpeg)
        .args(["-nostdin", "-hide_banner", "-v", "error"])
        .args(["-ss", &seconds(from)])
        .arg("-i")
        .arg(path)
        .args(["-map", "0:a:0?", "-vn", "-sn", "-dn"])
        .args(["-f", "f32le", "-ac", &CHANNELS.to_string()])
        .args(["-ar", &RATE.to_string(), "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("ffmpeg did not start: {error}"))?;
    let stdout = child.stdout.take().ok_or("ffmpeg has no output")?;
    let (sender, chunks) = std::sync::mpsc::sync_channel(16);
    std::thread::Builder::new()
        .name("video-sound".into())
        .spawn(move || pump(child, stdout, &sender))
        .map_err(|error| error.to_string())?;
    Ok(Samples {
        chunks,
        chunk: Vec::new(),
        at: 0,
    })
}

/// Reads samples until the sound ends or rodio drops them, then stops the
/// process.
fn pump(mut child: Child, mut stdout: ChildStdout, chunks: &SyncSender<Vec<f32>>) {
    let mut bytes = vec![0u8; CHUNK * 4];
    loop {
        let mut filled = 0;
        while filled < bytes.len() {
            match stdout.read(&mut bytes[filled..]) {
                Ok(0) | Err(_) => break,
                Ok(read) => filled += read,
            }
        }
        let whole = filled - filled % 4;
        if whole > 0 {
            let chunk = bytes[..whole]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|sample| f32::from_le_bytes(*sample))
                .collect();
            if chunks.send(chunk).is_err() {
                break;
            }
        }
        if filled < bytes.len() {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Iterator for Samples {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.at == self.chunk.len() {
            self.chunk = self.chunks.recv().ok()?;
            self.at = 0;
        }
        let sample = self.chunk.get(self.at).copied();
        self.at += 1;
        sample
    }
}

impl rodio::Source for Samples {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        std::num::NonZero::new(CHANNELS).expect("two channels")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        std::num::NonZero::new(RATE).expect("a sample rate")
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn seconds(at: Duration) -> String {
    format!("{:.3}", at.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_package_manager_of_each_family() {
        for (release, expected) in [
            ("ID=cachyos\nID_LIKE=arch", Some(Install::Pacman)),
            ("ID=endeavouros\nID_LIKE=arch", Some(Install::Pacman)),
            ("ID=arch", Some(Install::Pacman)),
            ("ID=fedora", Some(Install::Dnf)),
            (
                "ID=linuxmint\nID_LIKE=\"ubuntu debian\"",
                Some(Install::Apt),
            ),
            ("ID=ubuntu\nID_LIKE=debian", Some(Install::Apt)),
            ("ID=debian", Some(Install::Apt)),
            ("ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"", None),
        ] {
            assert_eq!(install_for(release), expected, "{release}");
        }
    }

    #[test]
    fn probe_turns_sideways_videos_upright() {
        let json = serde_json::json!({
            "streams": [{
                "width": 1920, "height": 1080, "avg_frame_rate": "30000/1001",
                "side_data_list": [{"rotation": -90}]
            }],
            "format": {"duration": "12.5"}
        });
        let probe = parse_probe(&json).unwrap();
        assert_eq!((probe.width, probe.height), (1080, 1920));
        assert!((probe.rate - 29.97).abs() < 0.01);
        assert_eq!(probe.duration, Duration::from_millis(12_500));
    }
}
