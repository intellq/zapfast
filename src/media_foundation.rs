//! Video playback through Windows' own decoders (Media Foundation).
//!
//! The built-in player reads H.264 in MP4 with AAC-LC sound. On Windows a
//! video can play through Media Foundation instead, which reads what the
//! system has decoders for: H.264 of every profile, HE-AAC, AC-3 and MP3 out
//! of the box, and HEVC, VP9 and AV1 once their free extensions from the
//! Microsoft Store are installed, in MP4, MOV and the other containers
//! Windows opens. A Source Reader on the decoding thread hands over frames
//! as 32-bit RGB at the size asked for, and another on its own thread hands
//! over the sound as float samples. The built-in player stays the fallback.
//!
//! Media Foundation's DLLs are loaded when first needed rather than linked:
//! Windows N editions ship without them until the Media Feature Pack is
//! installed, and ZapFast must still start there.

use std::ffi::c_void;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::Duration;

use egui::ColorImage;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFActivate, IMFAttributes, IMFMediaType, IMFSample, IMFSourceReader,
    MF_E_INVALIDSTREAMNUMBER, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_MT_VIDEO_ROTATION,
    MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READER_MEDIASOURCE,
    MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READERF_ERROR, MF_VERSION, MFAudioFormat_Float, MFMediaType_Audio, MFMediaType_Video,
    MFSTARTUP_LITE, MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG_ALL, MFT_REGISTER_TYPE_INFO,
    MFVideoFormat_AV1, MFVideoFormat_HEVC, MFVideoFormat_RGB32, MFVideoFormat_VP90,
};
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{GUID, HRESULT, HSTRING, Interface, PCWSTR, s, w};

/// Samples per sound chunk handed to rodio.
const CHUNKS: usize = 16;

/// What `GetProcAddress` returns, before the cast to the real signature.
type Proc = unsafe extern "system" fn() -> isize;
type Startup = unsafe extern "system" fn(u32, u32) -> HRESULT;
type Shutdown = unsafe extern "system" fn() -> HRESULT;
type CreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type CreateMediaType = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type EnumTransforms = unsafe extern "system" fn(
    GUID,
    u32,
    *const MFT_REGISTER_TYPE_INFO,
    *const MFT_REGISTER_TYPE_INFO,
    *mut *mut *mut c_void,
    *mut u32,
) -> HRESULT;
type CreateReader = unsafe extern "system" fn(PCWSTR, *mut c_void, *mut *mut c_void) -> HRESULT;

/// The Media Foundation functions ZapFast calls, found at run time.
struct Api {
    startup: Startup,
    shutdown: Shutdown,
    create_attributes: CreateAttributes,
    create_media_type: CreateMediaType,
    enum_transforms: EnumTransforms,
    create_reader: CreateReader,
}

/// The functions, or `None` where Media Foundation is not installed.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let api = load();
        if api.is_none() {
            log::info!("Media Foundation is not available; videos use the built-in player");
        }
        api
    })
    .as_ref()
}

fn load() -> Option<Api> {
    unsafe {
        let platform = LoadLibraryExW(w!("mfplat.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let readwrite =
            LoadLibraryExW(w!("mfreadwrite.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        let find = |module: HMODULE, name: windows::core::PCSTR| GetProcAddress(module, name);
        // Each pointer is cast to the function's documented signature.
        Some(Api {
            startup: std::mem::transmute::<Proc, Startup>(find(platform, s!("MFStartup"))?),
            shutdown: std::mem::transmute::<Proc, Shutdown>(find(platform, s!("MFShutdown"))?),
            create_attributes: std::mem::transmute::<Proc, CreateAttributes>(find(
                platform,
                s!("MFCreateAttributes"),
            )?),
            create_media_type: std::mem::transmute::<Proc, CreateMediaType>(find(
                platform,
                s!("MFCreateMediaType"),
            )?),
            enum_transforms: std::mem::transmute::<Proc, EnumTransforms>(find(
                platform,
                s!("MFTEnumEx"),
            )?),
            create_reader: std::mem::transmute::<Proc, CreateReader>(find(
                readwrite,
                s!("MFCreateSourceReaderFromURL"),
            )?),
        })
    }
}

/// Whether Media Foundation can be used on this computer.
pub fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| Runtime::start().is_ok())
}

/// Optional video decoders, each installed from the Microsoft Store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Codecs {
    pub hevc: bool,
    pub vp9: bool,
    pub av1: bool,
}

/// Which optional decoders this computer has. Asked once per session: an
/// extension installed meanwhile counts from the next start.
pub fn codecs() -> Codecs {
    static CODECS: OnceLock<Codecs> = OnceLock::new();
    *CODECS.get_or_init(|| {
        let Ok(_runtime) = Runtime::start() else {
            return Codecs::default();
        };
        Codecs {
            hevc: has_decoder(MFVideoFormat_HEVC),
            vp9: has_decoder(MFVideoFormat_VP90),
            av1: has_decoder(MFVideoFormat_AV1),
        }
    })
}

fn has_decoder(subtype: GUID) -> bool {
    let Some(api) = api() else {
        return false;
    };
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype,
    };
    let mut activates: *mut *mut c_void = std::ptr::null_mut();
    let mut count = 0u32;
    let found = unsafe {
        (api.enum_transforms)(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_ALL.0 as u32,
            &input,
            std::ptr::null(),
            &mut activates,
            &mut count,
        )
    };
    if found.is_err() || activates.is_null() {
        return false;
    }
    unsafe {
        for index in 0..count as usize {
            let activate = *activates.add(index);
            if !activate.is_null() {
                drop(IMFActivate::from_raw(activate));
            }
        }
        CoTaskMemFree(Some(activates as *const c_void));
    }
    count > 0
}

/// COM and Media Foundation, started on this thread for as long as it lives.
struct Runtime {
    api: &'static Api,
    com: bool,
}

impl Runtime {
    fn start() -> Result<Self, String> {
        let api = api().ok_or("Media Foundation is not installed")?;
        // A thread already in another apartment keeps it; only a call that
        // succeeded is balanced on drop.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let started = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_LITE) };
        if let Err(error) = started.ok() {
            if com {
                unsafe { CoUninitialize() };
            }
            return Err(format!("Media Foundation did not start: {error}"));
        }
        Ok(Self { api, com })
    }

    fn attributes(&self, size: u32) -> Result<IMFAttributes, String> {
        let mut raw = std::ptr::null_mut();
        unsafe { (self.api.create_attributes)(&mut raw, size) }
            .ok()
            .map_err(|error| error.to_string())?;
        Ok(unsafe { IMFAttributes::from_raw(raw) })
    }

    fn media_type(&self) -> Result<IMFMediaType, String> {
        let mut raw = std::ptr::null_mut();
        unsafe { (self.api.create_media_type)(&mut raw) }
            .ok()
            .map_err(|error| error.to_string())?;
        Ok(unsafe { IMFMediaType::from_raw(raw) })
    }

    /// A reader of `path` with only its first stream of the kind asked for
    /// selected. `None` when the file has no such stream.
    fn reader(
        &self,
        path: &Path,
        stream: u32,
        video: bool,
    ) -> Result<Option<IMFSourceReader>, String> {
        let attributes = self.attributes(2)?;
        unsafe {
            attributes
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .map_err(|error| error.to_string())?;
            if video {
                // Converts to RGB and scales on the way out.
                attributes
                    .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
                    .map_err(|error| error.to_string())?;
            }
        }
        let url = HSTRING::from(path.as_os_str());
        let mut raw = std::ptr::null_mut();
        unsafe { (self.api.create_reader)(PCWSTR(url.as_ptr()), attributes.as_raw(), &mut raw) }
            .ok()
            .map_err(|error| format!("Media Foundation cannot open the file: {error}"))?;
        let reader = unsafe { IMFSourceReader::from_raw(raw) };
        unsafe {
            reader
                .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                .map_err(|error| error.to_string())?;
            match reader.SetStreamSelection(stream, true) {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_INVALIDSTREAMNUMBER => return Ok(None),
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(Some(reader))
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.api.shutdown)();
            if self.com {
                CoUninitialize();
            }
        }
    }
}

/// Moves `reader` to `at`. Decoding resumes from the key frame before it.
fn seek(reader: &IMFSourceReader, at: Duration) -> Result<(), String> {
    if at.is_zero() {
        return Ok(());
    }
    let mut position = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::default();
    unsafe {
        let value = &mut position.Anonymous.Anonymous;
        value.vt = VT_I8;
        value.Anonymous.hVal = ticks(at) as i64;
        reader
            .SetCurrentPosition(&GUID::zeroed(), &position)
            .map_err(|error| error.to_string())
    }
}

fn ticks(at: Duration) -> u64 {
    (at.as_nanos() / 100).min(u128::from(u64::MAX)) as u64
}

fn time(ticks: i64) -> Duration {
    Duration::from_nanos(ticks.max(0) as u64 * 100)
}

/// Packs a width and height the way `MF_MT_FRAME_SIZE` stores them.
fn pack(width: u32, height: u32) -> u64 {
    (u64::from(width) << 32) | u64::from(height)
}

fn unpack(size: u64) -> (u32, u32) {
    ((size >> 32) as u32, size as u32)
}

/// A video's frames, decoded on the calling thread.
pub struct Video {
    // Declared first: a reader goes before the runtime it runs on.
    reader: IMFSourceReader,
    stream: u32,
    width: u32,
    height: u32,
    /// Bytes from one row to the next when the buffer has no 2D view;
    /// negative for pictures stored bottom-up.
    stride: i32,
    /// Clockwise quarter turns that stand the picture upright.
    turns: u8,
    duration: Duration,
    _runtime: Runtime,
}

impl Video {
    /// Opens `path`'s first video stream, with frames no larger than
    /// `max_side` on their longest side.
    pub fn open(path: &Path, max_side: u32) -> Result<Self, String> {
        let runtime = Runtime::start()?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let reader = runtime
            .reader(path, stream, true)?
            .ok_or("the file has no video stream")?;
        let native =
            unsafe { reader.GetNativeMediaType(stream, 0) }.map_err(|error| error.to_string())?;
        let (width, height) = unpack(
            unsafe { native.GetUINT64(&MF_MT_FRAME_SIZE) }.map_err(|error| error.to_string())?,
        );
        let turns = match unsafe { native.GetUINT32(&MF_MT_VIDEO_ROTATION) }.unwrap_or(0) {
            90 => 1,
            180 => 2,
            270 => 3,
            _ => 0,
        };
        let (fit_width, fit_height) = crate::video::fitted(width, height, max_side);
        let output = runtime.media_type()?;
        unsafe {
            output
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|error| error.to_string())?;
            output
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .map_err(|error| error.to_string())?;
            // Even sizes: some converters refuse odd ones.
            let sized = output
                .SetUINT64(
                    &MF_MT_FRAME_SIZE,
                    pack(fit_width.max(2) & !1, fit_height.max(2) & !1),
                )
                .is_ok()
                && reader.SetCurrentMediaType(stream, None, &output).is_ok();
            if !sized {
                // Without scaling, at the stored size.
                let _ = output.DeleteItem(&MF_MT_FRAME_SIZE);
                reader
                    .SetCurrentMediaType(stream, None, &output)
                    .map_err(|error| {
                        format!("Media Foundation cannot decode the video: {error}")
                    })?;
            }
        }
        let duration = unsafe {
            reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        }
        .map(|value| time(unsafe { value.Anonymous.Anonymous.Anonymous.uhVal } as i64))
        .unwrap_or_default();
        let mut video = Self {
            reader,
            stream,
            width: 0,
            height: 0,
            stride: 0,
            turns,
            duration,
            _runtime: runtime,
        };
        video.read_format()?;
        Ok(video)
    }

    /// Takes the size and row stride of the frames the reader hands over.
    fn read_format(&mut self) -> Result<(), String> {
        let current = unsafe { self.reader.GetCurrentMediaType(self.stream) }
            .map_err(|error| error.to_string())?;
        let (width, height) = unpack(
            unsafe { current.GetUINT64(&MF_MT_FRAME_SIZE) }.map_err(|error| error.to_string())?,
        );
        self.width = width;
        self.height = height;
        self.stride = unsafe { current.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .map_or(width as i32 * 4, |stride| stride as i32);
        Ok(())
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Moves to `at`; frames before it still arrive, from the key frame
    /// before, until [`Video::next`] passes them.
    pub fn seek(&mut self, at: Duration) -> Result<(), String> {
        seek(&self.reader, at)
    }

    /// The next frame at or after `from` and its time, or `None` at the end.
    /// Earlier frames are decoded but not converted.
    pub fn next(&mut self, from: Duration) -> Result<Option<(Duration, ColorImage)>, String> {
        loop {
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample: Option<IMFSample> = None;
            unsafe {
                self.reader.ReadSample(
                    self.stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )
            }
            .map_err(|error| error.to_string())?;
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                return Err("Media Foundation stopped decoding the video".to_owned());
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                self.read_format()?;
            }
            if let Some(sample) = sample {
                let at = time(timestamp);
                if at >= from {
                    return Ok(Some((at, self.picture(&sample)?)));
                }
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
        }
    }

    /// Copies a sample's RGB32 picture into an upright RGBA image.
    fn picture(&self, sample: &IMFSample) -> Result<ColorImage, String> {
        let buffer =
            unsafe { sample.ConvertToContiguousBuffer() }.map_err(|error| error.to_string())?;
        let (width, height) = (self.width as usize, self.height as usize);
        let mut rgba = vec![0u8; width * height * 4];
        let mut copy = |first: *const u8, pitch: isize| {
            for y in 0..height {
                let row = unsafe {
                    std::slice::from_raw_parts(first.offset(y as isize * pitch), width * 4)
                };
                let out = &mut rgba[y * width * 4..(y + 1) * width * 4];
                // Stored as blue, green, red and an unused byte.
                for (pixel, source) in out
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(row.as_chunks::<4>().0)
                {
                    *pixel = [source[2], source[1], source[0], 255];
                }
            }
        };
        if let Ok(planar) = buffer.cast::<IMF2DBuffer>() {
            let mut first = std::ptr::null_mut();
            let mut pitch = 0i32;
            unsafe { planar.Lock2D(&mut first, &mut pitch) }.map_err(|error| error.to_string())?;
            copy(first, pitch as isize);
            let _ = unsafe { planar.Unlock2D() };
        } else {
            let mut start = std::ptr::null_mut();
            let mut length = 0u32;
            unsafe { buffer.Lock(&mut start, None, Some(&mut length)) }
                .map_err(|error| error.to_string())?;
            let pitch = self.stride as isize;
            let needed = pitch.unsigned_abs() * height;
            if (length as usize) < needed {
                let _ = unsafe { buffer.Unlock() };
                return Err("Media Foundation sent a short frame".to_owned());
            }
            let first = if pitch < 0 {
                unsafe { start.add(pitch.unsigned_abs() * (height - 1)) }
            } else {
                start
            };
            copy(first, pitch);
            let _ = unsafe { buffer.Unlock() };
        }
        let (width, height, rgba) = turn(width, height, rgba, self.turns);
        Ok(ColorImage::from_rgba_unmultiplied([width, height], &rgba))
    }
}

/// Turns an RGBA picture clockwise by `turns` quarter turns.
fn turn(width: usize, height: usize, rgba: Vec<u8>, turns: u8) -> (usize, usize, Vec<u8>) {
    if turns.is_multiple_of(4) {
        return (width, height, rgba);
    }
    let (out_width, out_height) = if turns % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    };
    let mut out = vec![0u8; rgba.len()];
    for y in 0..height {
        for x in 0..width {
            let (to_x, to_y) = match turns % 4 {
                1 => (height - 1 - y, x),
                2 => (width - 1 - x, height - 1 - y),
                _ => (y, width - 1 - x),
            };
            let from = (y * width + x) * 4;
            let to = (to_y * out_width + to_x) * 4;
            out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    (out_width, out_height, out)
}

/// A video's sound as float samples, decoded on a thread of its own.
pub struct Samples {
    chunks: Receiver<Vec<f32>>,
    chunk: Vec<f32>,
    at: usize,
    channels: u16,
    rate: u32,
}

/// Starts sending `path`'s sound from `from`. A video without sound ends at
/// once, and plays silently.
pub fn samples(path: &Path, from: Duration) -> Result<Samples, String> {
    let (format, formats) = std::sync::mpsc::sync_channel(1);
    let (sender, chunks) = std::sync::mpsc::sync_channel(CHUNKS);
    let path = path.to_owned();
    std::thread::Builder::new()
        .name("video-sound".into())
        .spawn(move || {
            if let Err(error) = pump(&path, from, &format, &sender) {
                let _ = format.send(Err(error));
            }
        })
        .map_err(|error| error.to_string())?;
    let (channels, rate) = formats
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "Media Foundation did not open the sound".to_owned())??;
    Ok(Samples {
        chunks,
        chunk: Vec::new(),
        at: 0,
        channels,
        rate,
    })
}

/// Reads the sound until it ends or rodio drops it. The format goes out
/// first, before any samples.
fn pump(
    path: &Path,
    from: Duration,
    format: &SyncSender<Result<(u16, u32), String>>,
    chunks: &SyncSender<Vec<f32>>,
) -> Result<(), String> {
    let runtime = Runtime::start()?;
    let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    let Some(reader) = runtime.reader(path, stream, false)? else {
        // No sound: an empty source that ends at once.
        let _ = format.send(Ok((2, 48_000)));
        return Ok(());
    };
    let output = runtime.media_type()?;
    unsafe {
        output
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
            .map_err(|error| error.to_string())?;
        output
            .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
            .map_err(|error| error.to_string())?;
        reader
            .SetCurrentMediaType(stream, None, &output)
            .map_err(|error| format!("Media Foundation cannot decode the sound: {error}"))?;
    }
    let current =
        unsafe { reader.GetCurrentMediaType(stream) }.map_err(|error| error.to_string())?;
    let channels = unsafe { current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) }
        .map_err(|error| error.to_string())?
        .clamp(1, u32::from(u16::MAX)) as u16;
    let rate = unsafe { current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) }
        .map_err(|error| error.to_string())?
        .max(1);
    seek(&reader, from)?;
    if format.send(Ok((channels, rate))).is_err() {
        return Ok(());
    }
    loop {
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample: Option<IMFSample> = None;
        unsafe {
            reader.ReadSample(
                stream,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )
        }
        .map_err(|error| error.to_string())?;
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            return Err("Media Foundation stopped decoding the sound".to_owned());
        }
        if let Some(sample) = sample {
            let buffer =
                unsafe { sample.ConvertToContiguousBuffer() }.map_err(|error| error.to_string())?;
            let mut start = std::ptr::null_mut();
            let mut length = 0u32;
            unsafe { buffer.Lock(&mut start, None, Some(&mut length)) }
                .map_err(|error| error.to_string())?;
            let bytes = unsafe { std::slice::from_raw_parts(start, length as usize) };
            let mut chunk: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|sample| f32::from_le_bytes(*sample))
                .collect();
            let _ = unsafe { buffer.Unlock() };
            // After a seek the sound starts at the packet before `from`.
            let early = from.saturating_sub(time(timestamp));
            let skip = (early.as_secs_f64() * f64::from(rate)) as usize * usize::from(channels);
            chunk.drain(..skip.min(chunk.len()));
            if !chunk.is_empty() && chunks.send(chunk).is_err() {
                return Ok(());
            }
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(());
        }
    }
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
        std::num::NonZero::new(self.channels).expect("at least one channel")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        std::num::NonZero::new(self.rate).expect("a sample rate")
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_sizes_pack_as_media_foundation_stores_them() {
        assert_eq!(pack(1920, 1080), 0x0000_0780_0000_0438);
        assert_eq!(unpack(pack(1920, 1080)), (1920, 1080));
    }

    #[test]
    fn quarter_turns_stand_the_picture_up() {
        // Two pixels side by side, red then blue.
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let picture = [red, blue].concat();
        assert_eq!(turn(2, 1, picture.clone(), 1), (1, 2, picture.clone()));
        assert_eq!(turn(2, 1, picture.clone(), 2), (2, 1, [blue, red].concat()));
        assert_eq!(turn(2, 1, picture.clone(), 3), (1, 2, [blue, red].concat()));
        assert_eq!(turn(2, 1, picture.clone(), 0), (2, 1, picture));
    }

    #[test]
    fn times_convert_to_ticks_and_back() {
        assert_eq!(ticks(Duration::from_millis(1500)), 15_000_000);
        assert_eq!(time(15_000_000), Duration::from_millis(1500));
        assert_eq!(time(-5), Duration::ZERO);
    }
}
