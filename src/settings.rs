//! User preferences stored in JSON.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Verifying the locked-chat code costs about 20 ms, paid once per distinct
/// typed string. ponytail: fixed cost, revisit if it lags the search field.
const CHAT_LOCK_ROUNDS: std::num::NonZeroU32 = std::num::NonZeroU32::new(200_000).unwrap();

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Option<Vec<u8>> {
    value
        .len()
        .is_multiple_of(2)
        .then(|| {
            (0..value.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&value[at..at + 2], 16).ok())
                .collect()
        })
        .flatten()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => crate::i18n::n_("Dark"),
            Self::Light => crate::i18n::n_("Light"),
            Self::System => crate::i18n::n_("Follow system"),
        }
    }
}

/// Background colours offered by WhatsApp's wallpaper picker, after the
/// active theme's own chat colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperColor {
    /// The active palette's `chat` colour, so a custom theme sets the wallpaper.
    #[default]
    Theme,
    Beige,
    Cruise,
    Scandal,
    MonteCarlo,
    HawkesBlue,
    Downy,
    Seagull,
    Quartz,
    VeryLightGrey,
    Orinoco,
    Tusk,
    CapeHoney,
    Caramel,
    RoseBud,
    Bittersweet,
    RadicalRed,
    MandarinOrange,
    Flamingo,
    Buccaneer,
    BreakerBay,
    Pelorous,
    ToryBlue,
    Fiord,
    Cinder,
    Tolopea,
    Solitude,
    Canary,
    WillowBrook,
    Black,
    Nordic,
    CardinGreen,
    Tangaroa,
    Tiber,
    BlackRussian,
    Nero,
    Marshland,
    Maire,
    BlackMagic,
    CocoaBrown,
    WoodBark,
    SealBrown,
    SealBrownDarker,
    SealBrownLight,
    Cyprus,
    BlueWhale,
    BlackPearl,
    DarkTolopea,
    Woodsmoke,
    MaireTwo,
}

impl WallpaperColor {
    pub const LIGHT: [Self; 29] = [
        Self::Theme,
        Self::Beige,
        Self::Cruise,
        Self::Scandal,
        Self::MonteCarlo,
        Self::HawkesBlue,
        Self::Downy,
        Self::Seagull,
        Self::Quartz,
        Self::VeryLightGrey,
        Self::Orinoco,
        Self::Tusk,
        Self::CapeHoney,
        Self::Caramel,
        Self::RoseBud,
        Self::Bittersweet,
        Self::RadicalRed,
        Self::MandarinOrange,
        Self::Flamingo,
        Self::Buccaneer,
        Self::BreakerBay,
        Self::Pelorous,
        Self::ToryBlue,
        Self::Fiord,
        Self::Cinder,
        Self::Tolopea,
        Self::Solitude,
        Self::Canary,
        Self::WillowBrook,
    ];

    pub const DARK: [Self; 22] = [
        Self::Theme,
        Self::Black,
        Self::Nordic,
        Self::CardinGreen,
        Self::Tangaroa,
        Self::Tiber,
        Self::BlackRussian,
        Self::Nero,
        Self::Marshland,
        Self::Maire,
        Self::BlackMagic,
        Self::CocoaBrown,
        Self::WoodBark,
        Self::SealBrown,
        Self::SealBrownDarker,
        Self::SealBrownLight,
        Self::Cyprus,
        Self::BlueWhale,
        Self::BlackPearl,
        Self::DarkTolopea,
        Self::Woodsmoke,
        Self::MaireTwo,
    ];

    pub fn choices(dark: bool) -> &'static [Self] {
        if dark { &Self::DARK } else { &Self::LIGHT }
    }

    /// The colour's English name, marked for translation. The interface
    /// translates [`Self::Theme`]'s with its own context.
    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Beige => crate::i18n::n_("Beige"),
            Self::Cruise => crate::i18n::n_("Cruise"),
            Self::Scandal => crate::i18n::n_("Scandal"),
            Self::MonteCarlo => crate::i18n::n_("Monte Carlo"),
            Self::HawkesBlue => crate::i18n::n_("Hawkes Blue"),
            Self::Downy => crate::i18n::n_("Downy"),
            Self::Seagull => crate::i18n::n_("Seagull"),
            Self::Quartz => crate::i18n::n_("Quartz"),
            Self::VeryLightGrey => crate::i18n::n_("Very light grey"),
            Self::Orinoco => crate::i18n::n_("Orinoco"),
            Self::Tusk => crate::i18n::n_("Tusk"),
            Self::CapeHoney => crate::i18n::n_("Cape honey"),
            Self::Caramel => crate::i18n::n_("Caramel"),
            Self::RoseBud => crate::i18n::n_("Rose bud"),
            Self::Bittersweet => crate::i18n::n_("Bittersweet"),
            Self::RadicalRed => crate::i18n::n_("Radical red"),
            Self::MandarinOrange => crate::i18n::n_("Mandarin Orange"),
            Self::Flamingo => crate::i18n::n_("Flamingo"),
            Self::Buccaneer => crate::i18n::n_("Buccaneer"),
            Self::BreakerBay => crate::i18n::n_("Breaker Bay"),
            Self::Pelorous => crate::i18n::n_("Pelorous"),
            Self::ToryBlue => crate::i18n::n_("Tory Blue"),
            Self::Fiord => crate::i18n::n_("Fiord"),
            Self::Cinder => crate::i18n::n_("Cinder"),
            Self::Tolopea => crate::i18n::n_("Tolopea"),
            Self::Solitude => crate::i18n::n_("Solitude"),
            Self::Canary => crate::i18n::n_("Canary"),
            Self::WillowBrook => crate::i18n::n_("Willow Brook"),
            Self::Black => crate::i18n::n_("Black"),
            Self::Nordic => crate::i18n::n_("Nordic"),
            Self::CardinGreen => crate::i18n::n_("Cardin Green"),
            Self::Tangaroa => crate::i18n::n_("Tangaroa"),
            Self::Tiber => crate::i18n::n_("Tiber"),
            Self::BlackRussian => crate::i18n::n_("Black Russian"),
            Self::Nero => crate::i18n::n_("Nero"),
            Self::Marshland => crate::i18n::n_("Marshland"),
            Self::Maire => crate::i18n::n_("Maire"),
            Self::BlackMagic => crate::i18n::n_("Black Magic"),
            Self::CocoaBrown => crate::i18n::n_("Cocoa Brown"),
            Self::WoodBark => crate::i18n::n_("Wood Bark"),
            Self::SealBrown => crate::i18n::n_("Seal Brown"),
            Self::SealBrownDarker => crate::i18n::n_("Seal Brown Darker"),
            Self::SealBrownLight => crate::i18n::n_("Seal Brown Light"),
            Self::Cyprus => crate::i18n::n_("Cyprus"),
            Self::BlueWhale => crate::i18n::n_("Blue Whale"),
            Self::BlackPearl => crate::i18n::n_("Black Pearl"),
            Self::DarkTolopea => crate::i18n::n_("Tolopea"),
            Self::Woodsmoke => crate::i18n::n_("Woodsmoke"),
            Self::MaireTwo => crate::i18n::n_("Maire 2"),
        }
    }

    /// A fixed colour's value; `None` for [`Self::Theme`], which follows the
    /// palette.
    pub fn rgb(self) -> Option<[u8; 3]> {
        Some(match self {
            Self::Theme => return None,
            Self::Beige => [245, 241, 235],
            Self::Cruise => [187, 228, 229],
            Self::Scandal => [174, 216, 199],
            Self::MonteCarlo => [122, 203, 165],
            Self::HawkesBlue => [203, 218, 236],
            Self::Downy => [102, 210, 213],
            Self::Seagull => [99, 189, 207],
            Self::Quartz => [214, 208, 240],
            Self::VeryLightGrey => [206, 206, 206],
            Self::Orinoco => [209, 218, 190],
            Self::Tusk => [230, 225, 177],
            Self::CapeHoney => [254, 239, 169],
            Self::Caramel => [254, 210, 151],
            Self::RoseBud => [253, 154, 155],
            Self::Bittersweet => [253, 103, 105],
            Self::RadicalRed => [251, 70, 104],
            Self::MandarinOrange => [146, 32, 64],
            Self::Flamingo => [220, 110, 79],
            Self::Buccaneer => [100, 77, 82],
            Self::BreakerBay => [81, 126, 126],
            Self::Pelorous => [49, 144, 187],
            Self::ToryBlue => [53, 85, 138],
            Self::Fiord => [85, 98, 111],
            Self::Cinder => [29, 35, 38],
            Self::Tolopea => [48, 30, 52],
            Self::Solitude => [236, 240, 241],
            Self::Canary => [255, 254, 162],
            Self::WillowBrook => [231, 232, 210],
            Self::Black => [22, 23, 23],
            Self::Nordic => [15, 36, 36],
            Self::CardinGreen => [18, 38, 31],
            Self::Tangaroa => [17, 30, 39],
            Self::Tiber => [14, 33, 37],
            Self::BlackRussian => [31, 29, 37],
            Self::Nero => [33, 33, 33],
            Self::Marshland => [31, 33, 28],
            Self::Maire => [35, 35, 27],
            Self::BlackMagic => [38, 36, 25],
            Self::CocoaBrown => [38, 31, 23],
            Self::WoodBark => [38, 23, 23],
            Self::SealBrown => [38, 15, 16],
            Self::SealBrownDarker => [25, 5, 11],
            Self::SealBrownLight => [33, 16, 12],
            Self::Cyprus => [10, 29, 37],
            Self::BlueWhale => [13, 21, 35],
            Self::BlackPearl => [13, 15, 17],
            Self::DarkTolopea => [17, 11, 18],
            Self::Woodsmoke => [30, 31, 31],
            Self::MaireTwo => [35, 35, 31],
        })
    }

    /// The colour drawn under `palette`: [`Self::Theme`] is its chat colour,
    /// read at draw time so a theme switch or Omarchy reload shows at once.
    pub fn color32(self, palette: &crate::theme::Palette) -> egui::Color32 {
        match self.rgb() {
            Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
            None => palette.chat,
        }
    }
}

/// The sound a new-message notification makes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSound {
    /// Pidgin's message sound, the default for new messages.
    #[default]
    #[serde(alias = "chime")]
    Receive,
    /// Pidgin's alert sound, the default for mentions and replies to us.
    #[serde(alias = "ripple")]
    Alert,
    /// Whatever the operating system plays for notifications.
    System,
    /// No sound.
    None,
    /// An audio file ZapFast plays itself.
    Custom(std::path::PathBuf),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// [`SETTINGS_VERSION`] when written; missing, and so 0, in older files.
    #[serde(default)]
    pub version: u32,
    pub theme: ThemeChoice,
    /// Interface language. `None` follows the operating system's locale.
    pub interface_language: Option<crate::i18n::Locale>,
    /// Filename of the selected local JSON palette.
    pub custom_theme: Option<String>,
    #[serde(
        default,
        deserialize_with = "fastframe_theme::read_cached_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_theme_cache: Option<crate::theme::CustomTheme>,
    #[serde(
        default,
        deserialize_with = "fastframe_theme::read_cached_theme",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_theme_cache: Option<crate::theme::CustomTheme>,
    /// egui zoom factor.
    pub zoom: f32,
    pub sidebar_width: f32,
    /// Width of the search pane beside the open chat.
    pub search_pane_width: f32,
    /// Whether Enter sends. Off, Enter adds a line and Ctrl+Enter (Cmd+Enter
    /// on macOS) sends.
    pub enter_sends: bool,
    /// Whether typing `:` and a name suggests emoji that Enter or Tab puts in
    /// place of the text. Off, what is typed stays as typed.
    pub emoji_shortcuts: bool,
    /// Draw emoji with an installed WhatsApp emoji font instead of the
    /// desktop's (or the bundled) Noto Color Emoji.
    pub whatsapp_emoji: bool,
    /// Play videos through the system's FFmpeg when it is installed (Linux),
    /// or through Windows' own decoders (Media Foundation) on Windows.
    pub ffmpeg_video: bool,
    /// Let FFmpeg decode videos on the graphics card when it can.
    pub ffmpeg_gpu: bool,
    /// Send read receipts, subject to the account privacy setting.
    pub send_read_receipts: bool,
    pub keep_deleted_messages: bool,
    /// Send typing state while composing.
    pub send_typing: bool,
    /// Fetch the page of a link typed in the composer and send its preview.
    pub link_previews: bool,
    /// The former switch that downloaded every kind of file, read once and
    /// folded into the per-kind switches by [`Settings::load`].
    #[serde(alias = "auto_download_images", skip_serializing)]
    pub auto_download: Option<bool>,
    /// Download these kinds of attachment when they enter view instead of on
    /// click. Static stickers follow images.
    pub auto_download_audio: bool,
    pub auto_download_video: bool,
    pub auto_download_image: bool,
    pub auto_download_animated_sticker: bool,
    pub auto_download_document: bool,
    /// Maximum attachment size, in MiB (the application supports 1..=64).
    pub attachment_limit_mib: u32,
    /// Show the default doodle wallpaper behind conversations.
    pub show_wallpaper: bool,
    /// Colour selected in the wallpaper picker.
    pub wallpaper_color: WallpaperColor,
    /// Colour selected for the dark wallpaper picker.
    pub dark_wallpaper_color: WallpaperColor,
    /// ZapFast's own copy of the chosen wallpaper image, drawn in place of the
    /// colour and doodles in light and dark mode alike.
    pub wallpaper_image: Option<std::path::PathBuf>,
    /// Last open chat, restored at startup.
    pub last_chat: Option<String>,
    /// The hint bar under the composer, hidden with its × and shown again
    /// from the Keyboard shortcuts dialog.
    pub show_shortcut_hints: bool,
    /// Recently used emoji, newest first.
    pub recent_emoji: Vec<String>,
    /// Emoji reaction usage on this device, most-used first (recent breaks ties).
    pub reaction_emoji: Vec<(String, u32)>,
    /// User GIPHY API key. Empty uses the optional built-in key.
    pub giphy_key: String,
    /// Keep the app linked in the tray when the window closes.
    pub keep_running_in_background: bool,
    /// Start at login with only the tray icon, without a window.
    pub start_minimized: bool,
    /// Desktop notifications while away from the chat.
    pub notifications: bool,
    /// Sound for new messages, in one-to-one chats and groups alike.
    pub message_sound: NotificationSound,
    /// Sound for group messages that mention us or reply to one of ours, as
    /// Pidgin alerts when someone says your name in a chat.
    pub mention_sound: NotificationSound,
    /// Play the message sound for ordinary group messages. Mentions and
    /// replies to us sound either way.
    pub group_sounds: bool,
    /// Folder for new downloads. `None` keeps them in the cache. Files
    /// already downloaded stay where they are when this changes.
    pub download_folder: Option<std::path::PathBuf>,
    /// Proxy for WhatsApp, media, and updates, such as
    /// `socks5h://127.0.0.1:9050`. Empty follows `ALL_PROXY` / `HTTPS_PROXY`.
    pub proxy: String,
    /// Ask GitHub once a day whether a newer release exists.
    pub check_for_updates: bool,
    /// Download verified updates in the background; restarting remains explicit.
    pub download_updates_automatically: bool,
    /// Voice and audio playback speed multiplier.
    pub voice_speed: f32,
    /// Pause other apps' media while recording, or while a voice message,
    /// audio, or video plays with sound.
    pub pause_other_media: bool,
    /// The two switches `pause_other_media` replaced, read once and folded
    /// into it by [`Settings::load`].
    #[serde(skip_serializing)]
    pub pause_media_while_recording: Option<bool>,
    #[serde(skip_serializing)]
    pub pause_media_while_playing: Option<bool>,
    /// The last choice of the new-contact dialog's "Save to phone" box, which
    /// starts the next one and applies when a contact is renamed.
    pub save_contacts_to_phone: bool,
    /// Legacy plaintext code, accepted once and rewritten as a verifier.
    #[serde(skip_serializing)]
    pub chat_lock_code: Option<String>,
    /// Salted PBKDF2 verifier for the local locked-chats code, `salt$hash`.
    pub chat_lock_code_hash: Option<String>,
    /// The one-time locked-chat code hint has been opened.
    pub chat_lock_hint_dismissed: bool,
    /// Microphone the next 1:1 call records from, as a PipeWire node name.
    /// `None` follows the system default. Recorded like every other preference, and only ever
    /// applied to ZapFast's own call: the system's default device is never changed.
    pub call_microphone: Option<String>,
    /// Speaker the next 1:1 call plays through, as a PipeWire node name.
    pub call_speaker: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            theme: ThemeChoice::Dark,
            interface_language: None,
            custom_theme: None,
            custom_theme_cache: None,
            system_theme_cache: None,
            zoom: 1.0,
            sidebar_width: 320.0,
            search_pane_width: 380.0,
            enter_sends: true,
            emoji_shortcuts: false,
            whatsapp_emoji: false,
            ffmpeg_video: true,
            ffmpeg_gpu: true,
            send_read_receipts: true,
            keep_deleted_messages: false,
            send_typing: true,
            link_previews: true,
            auto_download: None,
            auto_download_audio: true,
            auto_download_video: false,
            auto_download_image: true,
            auto_download_animated_sticker: true,
            auto_download_document: false,
            attachment_limit_mib: 64,
            show_wallpaper: true,
            wallpaper_color: WallpaperColor::Theme,
            dark_wallpaper_color: WallpaperColor::Theme,
            wallpaper_image: None,
            last_chat: None,
            show_shortcut_hints: true,
            recent_emoji: Vec::new(),
            reaction_emoji: Vec::new(),
            giphy_key: String::new(),
            keep_running_in_background: true,
            start_minimized: false,
            notifications: true,
            message_sound: NotificationSound::Receive,
            mention_sound: NotificationSound::Alert,
            group_sounds: true,
            download_folder: None,
            proxy: String::new(),
            check_for_updates: true,
            download_updates_automatically: true,
            save_contacts_to_phone: true,
            voice_speed: 1.0,
            pause_other_media: true,
            pause_media_while_recording: None,
            pause_media_while_playing: None,
            chat_lock_code: None,
            chat_lock_code_hash: None,
            chat_lock_hint_dismissed: false,
            call_microphone: None,
            call_speaker: None,
        }
    }
}

/// The settings file's format. Files without a version predate it and are
/// migrated once by [`Settings::load`]:
///
/// 1. The wallpaper colours that were the defaults (Beige, and Black in dark
///    mode) become [`WallpaperColor::Theme`], the new default.
pub const SETTINGS_VERSION: u32 = 1;

/// Optional build-time GIPHY key from `ZAPFAST_GIPHY_KEY`.
/// The previous name remains accepted for existing build setups.
pub const BUILT_IN_GIPHY_KEY: Option<&str> = match option_env!("ZAPFAST_GIPHY_KEY") {
    Some(key) if !key.is_empty() => Some(key),
    _ => option_env!("FASTSAPP_GIPHY_KEY"),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoDownloadKind {
    /// Pictures, including static stickers and interactive cards' images.
    Image,
    Video,
    Audio,
    Document,
    AnimatedSticker,
}

impl Settings {
    pub fn attachment_limit_bytes(&self) -> u64 {
        u64::from(self.attachment_limit_mib.clamp(1, 64)) * 1024 * 1024
    }

    pub fn auto_downloads(&self, kind: AutoDownloadKind) -> bool {
        match kind {
            AutoDownloadKind::Image => self.auto_download_image,
            AutoDownloadKind::Video => self.auto_download_video,
            AutoDownloadKind::Audio => self.auto_download_audio,
            AutoDownloadKind::Document => self.auto_download_document,
            AutoDownloadKind::AnimatedSticker => self.auto_download_animated_sticker,
        }
    }

    pub fn wallpaper_color_for(&self, dark: bool) -> WallpaperColor {
        if dark {
            self.dark_wallpaper_color
        } else {
            self.wallpaper_color
        }
    }

    /// The wallpaper colour for `palette`, with Theme resolved against it.
    pub fn wallpaper_background(&self, palette: &crate::theme::Palette) -> egui::Color32 {
        self.wallpaper_color_for(palette.dark).color32(palette)
    }

    pub(crate) fn cached_palette(&self) -> Option<crate::theme::Palette> {
        let theme = if self.custom_theme.is_some() {
            self.custom_theme_cache.as_ref()
        } else if self.theme == ThemeChoice::System {
            self.system_theme_cache.as_ref()
        } else {
            None
        };
        theme.map(|theme| theme.palette)
    }

    /// Returns the user key, built-in key, or `None`.
    pub fn effective_giphy_key(&self) -> Option<String> {
        let own = self.giphy_key.trim();
        if !own.is_empty() {
            return Some(own.to_owned());
        }
        BUILT_IN_GIPHY_KEY
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str::<Self>(&contents) {
                Ok(mut settings) => {
                    settings.migrate();
                    settings.fold_legacy_media_pause();
                    settings.fold_legacy_auto_download();
                    if let Some(code) = settings.chat_lock_code.take() {
                        settings.set_chat_lock_code(Some(&code));
                        if let Err(error) = settings.save(path) {
                            log::warn!("could not replace the legacy locked-chat code: {error}");
                        }
                    }
                    settings
                }
                Err(_error) => {
                    log::warn!("settings file is unreadable, using defaults");
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("could not read settings: {error}");
                Self::default()
            }
        }
    }

    /// Atomically replaces the settings file through a temporary file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, contents)?;
        std::fs::rename(&temp, path)
    }

    /// Brings a file written before [`SETTINGS_VERSION`] up to date. Each
    /// step runs once: the next save records the version, so a colour chosen
    /// again afterwards is kept.
    fn migrate(&mut self) {
        if self.version < 1 {
            // Old files store every field, so a default colour cannot be told
            // from a chosen one; the old defaults move to the new one.
            if self.wallpaper_color == WallpaperColor::Beige {
                self.wallpaper_color = WallpaperColor::Theme;
            }
            if self.dark_wallpaper_color == WallpaperColor::Black {
                self.dark_wallpaper_color = WallpaperColor::Theme;
            }
        }
        self.version = self.version.max(SETTINGS_VERSION);
    }

    /// Folds the former recording and playback switches into
    /// `pause_other_media`. A file that still has them was last written by a
    /// version without the merged switch, so they win: media keeps pausing
    /// only if neither was turned off, which never pauses music for someone
    /// who asked it not to be. The next save drops them.
    fn fold_legacy_media_pause(&mut self) {
        let recording = self.pause_media_while_recording.take();
        let playing = self.pause_media_while_playing.take();
        if recording.is_some() || playing.is_some() {
            self.pause_other_media = recording.unwrap_or(true) && playing.unwrap_or(true);
        }
    }

    /// Folds the former download-everything switch into the per-kind ones.
    /// On, every kind downloads. Off, documents, which only it covered, stay
    /// off, and animated stickers keep downloading as stickers always did;
    /// the kinds already chosen stay as they were. The next save drops it.
    fn fold_legacy_auto_download(&mut self) {
        match self.auto_download.take() {
            Some(true) => {
                self.auto_download_audio = true;
                self.auto_download_video = true;
                self.auto_download_image = true;
                self.auto_download_animated_sticker = true;
                self.auto_download_document = true;
            }
            Some(false) => {
                self.auto_download_animated_sticker = true;
                self.auto_download_document = false;
            }
            None => {}
        }
    }

    pub fn set_chat_lock_code(&mut self, code: Option<&str>) {
        self.chat_lock_code = None;
        self.chat_lock_code_hash = code
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .map(Self::chat_lock_verifier);
    }

    /// Checking is deliberately slow, so callers memoize the answer.
    pub fn verifies_chat_lock_code(&self, code: &str) -> bool {
        let Some((salt, expected)) = self
            .chat_lock_code_hash
            .as_deref()
            .and_then(|stored| stored.split_once('$'))
        else {
            return false;
        };
        let (Some(salt), Some(expected)) = (unhex(salt), unhex(expected)) else {
            return false;
        };
        ring::pbkdf2::verify(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            CHAT_LOCK_ROUNDS,
            &salt,
            code.trim().as_bytes(),
            &expected,
        )
        .is_ok()
    }

    /// `salt$hash`, both hex. Codes are short enough to be guessed offline,
    /// so the stored form is salted and slow rather than a bare digest.
    fn chat_lock_verifier(code: &str) -> String {
        let salt: [u8; 16] = ring::rand::generate(&ring::rand::SystemRandom::new())
            .expect("the system random generator is unavailable")
            .expose();
        let mut hash = [0u8; 32];
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            CHAT_LOCK_ROUNDS,
            &salt,
            code.as_bytes(),
            &mut hash,
        );
        format!("{}${}", hex(&salt), hex(&hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_devices_a_call_used_come_back_after_a_restart() {
        let picked = Settings {
            call_microphone: Some("alsa_input.usb-Generic_USB_Headset-00.analog-mono".into()),
            call_speaker: Some("bluez_output.AC_12_34_56.1".into()),
            ..Settings::default()
        };
        let text = serde_json::to_string(&picked).expect("settings serialize");
        let loaded: Settings = serde_json::from_str(&text).expect("settings load");
        assert_eq!(loaded.call_microphone, picked.call_microphone);
        assert_eq!(loaded.call_speaker, picked.call_speaker);
    }

    #[test]
    fn a_call_with_no_device_picked_follows_the_system_default() {
        // A file written before calls could choose devices loads with every choice unset.
        let loaded: Settings = serde_json::from_str("{}").expect("settings load");
        assert_eq!(loaded.call_microphone, None);
        assert_eq!(loaded.call_speaker, None);
    }

    #[test]
    fn earlier_bundled_sound_names_still_load() {
        let settings: Settings =
            serde_json::from_str(r#"{"message_sound":"chime","mention_sound":"ripple"}"#).unwrap();
        assert_eq!(settings.message_sound, NotificationSound::Receive);
        assert_eq!(settings.mention_sound, NotificationSound::Alert);
    }

    #[test]
    fn the_former_group_sound_gives_way_to_mentions_and_quiet_groups() {
        // Group sound used to play for every group message; it is dropped,
        // and groups follow the message sound until they are silenced.
        let settings: Settings =
            serde_json::from_str(r#"{"message_sound":"system","group_sound":"none"}"#).unwrap();
        assert_eq!(settings.message_sound, NotificationSound::System);
        assert_eq!(settings.mention_sound, NotificationSound::Alert);
        assert!(settings.group_sounds);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let parsed: Settings =
            serde_json::from_str(r#"{"theme":"light","future_field":1}"#).expect("parses");
        assert_eq!(parsed.theme, ThemeChoice::Light);
        assert!(parsed.enter_sends);
        assert!(parsed.check_for_updates);
        assert!(parsed.download_updates_automatically);
        assert!(parsed.show_wallpaper);
        assert_eq!(parsed.wallpaper_color, WallpaperColor::Theme);
        assert!(parsed.pause_other_media);
        use AutoDownloadKind::{AnimatedSticker, Audio, Document, Image, Video};
        for kind in [Audio, Image, AnimatedSticker] {
            assert!(parsed.auto_downloads(kind));
        }
        assert!(!parsed.auto_downloads(Video) && !parsed.auto_downloads(Document));
    }

    #[test]
    fn automatic_download_filters_and_limit_preserve_old_settings() {
        use AutoDownloadKind::{AnimatedSticker, Audio, Document, Image, Video};
        let every = [Audio, Video, Image, Document, AnimatedSticker];

        // A file from before the filters, with everything off, keeps it off
        // except animated stickers, which always downloaded.
        let mut settings: Settings = serde_json::from_str(
            r#"{"auto_download":false,"auto_download_audio":false,"auto_download_video":false,"auto_download_image":false}"#,
        )
        .unwrap();
        settings.fold_legacy_auto_download();
        for kind in [Audio, Video, Image, Document] {
            assert!(!settings.auto_downloads(kind));
        }
        assert!(settings.auto_downloads(AnimatedSticker));
        assert_eq!(settings.attachment_limit_bytes(), 64 * 1024 * 1024);

        // The former switch on turns every kind on, and is not saved again.
        let mut settings: Settings = serde_json::from_str(r#"{"auto_download":true}"#).unwrap();
        settings.fold_legacy_auto_download();
        for kind in every {
            assert!(settings.auto_downloads(kind));
        }
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved.get("auto_download").is_none());

        // Each switch covers its own kind.
        settings.auto_download_video = false;
        settings.auto_download_document = false;
        assert!(settings.auto_downloads(Audio) && settings.auto_downloads(Image));
        assert!(!settings.auto_downloads(Video) && !settings.auto_downloads(Document));
        settings.attachment_limit_mib = 0;
        assert_eq!(settings.attachment_limit_bytes(), 1024 * 1024);
        settings.attachment_limit_mib = 100;
        assert_eq!(settings.attachment_limit_bytes(), 64 * 1024 * 1024);
    }

    fn load_from(contents: &str) -> (Settings, serde_json::Value) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, contents).unwrap();
        let settings = Settings::load(&path);
        settings.save(&path).unwrap();
        let stored = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        (settings, stored)
    }

    #[test]
    fn removed_settings_are_ignored_and_dropped_on_save() {
        let (settings, stored) = load_from(
            r#"{"enter_sends":false,"label_chips":true,"show_sender_pictures":true,
                "names_from_contacts":false,"collapse_chat_list":false,
                "save_contacts_to_phone":false}"#,
        );
        assert!(!settings.enter_sends, "the rest of the file still loads");
        assert!(
            !settings.save_contacts_to_phone,
            "the old switch starts the new-contact box"
        );
        for key in [
            "label_chips",
            "show_sender_pictures",
            "names_from_contacts",
            "collapse_chat_list",
            "pause_media_while_recording",
            "pause_media_while_playing",
        ] {
            assert!(stored.get(key).is_none(), "{key} is dropped on save");
        }
        assert_eq!(stored["pause_other_media"], true);
    }

    #[test]
    fn the_two_media_pause_switches_merge_into_one() {
        let merged = |contents: &str| {
            let (settings, stored) = load_from(contents);
            assert!(stored.get("pause_media_while_recording").is_none());
            assert!(stored.get("pause_media_while_playing").is_none());
            settings.pause_other_media
        };
        assert!(merged(
            r#"{"pause_media_while_recording":true,"pause_media_while_playing":true}"#
        ));
        assert!(!merged(
            r#"{"pause_media_while_recording":false,"pause_media_while_playing":true}"#
        ));
        assert!(!merged(
            r#"{"pause_media_while_recording":true,"pause_media_while_playing":false}"#
        ));
        assert!(!merged(r#"{"pause_media_while_playing":false}"#));
        assert!(merged(r#"{"pause_media_while_recording":true}"#));
        assert!(merged("{}"), "on by default");
        assert!(!merged(r#"{"pause_other_media":false}"#));
    }

    #[test]
    fn damaged_theme_cache_does_not_discard_other_settings() {
        let settings: Settings = serde_json::from_str(r#"{"custom_theme":"mine.json","custom_theme_cache":{"damaged":true},"enter_sends":false}"#).unwrap();
        assert!(!settings.enter_sends);
        assert!(settings.custom_theme_cache.is_none());
        assert_eq!(settings.custom_theme.as_deref(), Some("mine.json"));
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("zapfast-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let settings = Settings {
            zoom: 1.25,
            enter_sends: false,
            voice_speed: 1.5,
            pause_other_media: false,
            interface_language: Some(crate::i18n::Locale::German),
            message_sound: NotificationSound::None,
            mention_sound: NotificationSound::Custom("/sounds/ding.wav".into()),
            group_sounds: false,
            ..Settings::default()
        };
        settings.save(&path).expect("saves");
        assert_eq!(Settings::load(&path), settings);
        assert!(
            std::fs::read_to_string(&path)
                .expect("reads")
                .contains(r#""interface_language": "de""#),
            "the language keeps its stable serde name"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_locked_chat_verifier_is_salted_and_rejects_other_codes() {
        let mut settings = Settings::default();
        settings.set_chat_lock_code(Some(" 1234 "));
        assert!(settings.verifies_chat_lock_code("1234"));
        assert!(!settings.verifies_chat_lock_code("1235"));
        assert!(!settings.verifies_chat_lock_code(""));
        let first = settings.chat_lock_code_hash.clone();
        settings.set_chat_lock_code(Some("1234"));
        // A fresh salt every time, so the same code never stores the same value.
        assert_ne!(first, settings.chat_lock_code_hash);
        assert!(settings.verifies_chat_lock_code("1234"));
        settings.set_chat_lock_code(None);
        assert!(!settings.verifies_chat_lock_code("1234"));
    }

    #[test]
    fn legacy_locked_chat_code_is_rewritten_as_a_verifier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"chat_lock_code":"1234"}"#).unwrap();
        let settings = Settings::load(&path);
        assert!(settings.verifies_chat_lock_code("1234"));
        // The random hex verifier may contain "1234" by chance, so check
        // fields rather than searching the file for the digits.
        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(stored.get("chat_lock_code").is_none());
        let verifier = stored["chat_lock_code_hash"].as_str().unwrap();
        assert_ne!(verifier.trim(), "1234");
        assert!(verifier.contains('$'));
    }

    #[test]
    fn wallpaper_palette_contains_the_official_colours() {
        // WhatsApp's 28 light and 21 dark colours, after the theme's own.
        assert_eq!(WallpaperColor::LIGHT.len(), 29);
        assert_eq!(WallpaperColor::DARK.len(), 22);
        assert_eq!(WallpaperColor::LIGHT[0], WallpaperColor::Theme);
        assert_eq!(WallpaperColor::DARK[0], WallpaperColor::Theme);
        assert_eq!(WallpaperColor::Beige.rgb(), Some([245, 241, 235]));
        assert_eq!(WallpaperColor::Black.rgb(), Some([22, 23, 23]));
        assert_eq!(WallpaperColor::WillowBrook.label(), "Willow Brook");
    }

    #[test]
    fn the_theme_wallpaper_is_the_palettes_chat_colour() {
        use crate::theme::Palette;
        let settings = Settings::default();
        assert_eq!(settings.wallpaper_color, WallpaperColor::Theme);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Theme);
        let light = Palette::light();
        let dark = Palette::dark();
        assert_eq!(settings.wallpaper_background(&light), light.chat);
        assert_eq!(settings.wallpaper_background(&dark), dark.chat);
        let mut custom = Palette::dark();
        custom.chat = egui::Color32::from_rgb(30, 30, 46);
        assert_eq!(settings.wallpaper_background(&custom), custom.chat);
        // A fixed colour ignores the palette.
        let chosen = Settings {
            dark_wallpaper_color: WallpaperColor::Nordic,
            ..Settings::default()
        };
        assert_eq!(
            chosen.wallpaper_background(&custom),
            egui::Color32::from_rgb(15, 36, 36)
        );
        assert_eq!(chosen.wallpaper_background(&light), light.chat);
    }

    #[test]
    fn the_old_default_wallpaper_colours_become_the_theme_once() {
        let (settings, stored) =
            load_from(r#"{"wallpaper_color":"beige","dark_wallpaper_color":"black"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Theme);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Theme);
        assert_eq!(stored["version"], SETTINGS_VERSION);
        assert_eq!(stored["wallpaper_color"], "theme");

        // Chosen colours survive the migration.
        let (settings, _) =
            load_from(r#"{"wallpaper_color":"cruise","dark_wallpaper_color":"nordic"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Cruise);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Nordic);
        let (settings, _) =
            load_from(r#"{"wallpaper_color":"beige","dark_wallpaper_color":"tiber"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Theme);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Tiber);

        // Once migrated, choosing Beige or Black again sticks.
        let (settings, _) =
            load_from(r#"{"version":1,"wallpaper_color":"beige","dark_wallpaper_color":"black"}"#);
        assert_eq!(settings.wallpaper_color, WallpaperColor::Beige);
        assert_eq!(settings.dark_wallpaper_color, WallpaperColor::Black);

        // A current file round-trips unchanged, image path included.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let chosen = Settings {
            wallpaper_color: WallpaperColor::Beige,
            dark_wallpaper_color: WallpaperColor::Black,
            wallpaper_image: Some(dir.path().join("wallpaper.png")),
            ..Settings::default()
        };
        chosen.save(&path).unwrap();
        assert_eq!(Settings::load(&path), chosen);
        assert_eq!(
            Settings::load(&path),
            chosen,
            "loading again changes nothing"
        );
    }
}

#[cfg(test)]
mod giphy_tests {
    use super::*;

    #[test]
    fn the_users_key_wins_and_is_trimmed() {
        let settings = Settings {
            giphy_key: "  abc  ".into(),
            ..Settings::default()
        };
        assert_eq!(settings.effective_giphy_key().as_deref(), Some("abc"));
    }

    #[test]
    fn without_a_key_of_their_own_the_built_in_one_is_used() {
        let settings = Settings {
            giphy_key: "   ".into(),
            ..Settings::default()
        };
        let expected = BUILT_IN_GIPHY_KEY
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        assert_eq!(settings.effective_giphy_key(), expected);
    }
}
