//! The interface fonts offered besides the bundled Inter: a short list of
//! families common on a Linux desktop, those of them installed here, and
//! the platform's own interface font under its name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use egui::FontData;
use fastframe_fonts::Weight;
use skrifa::MetadataProvider;

/// The families offered when installed, in alphabetical order.
pub const OFFERED: [&str; 10] = [
    "Adwaita Sans",
    "Cantarell",
    "Carlito",
    "Hack",
    "Liberation Sans",
    "Liberation Serif",
    "Nimbus Sans",
    "Noto Sans",
    "Noto Serif",
    "Open Sans",
];

/// The optical size a variable face is drawn at, as fastframe-fonts does
/// for the platform's face.
const TEXT_OPTICAL_SIZE: f32 = 14.0;

/// Faces read from one collection file at most.
const MAX_FACES: u32 = 64;

/// The family the platform draws its interface with: on KDE the font set in
/// its settings (Noto Sans unless changed), elsewhere on Linux fontconfig's
/// `system-ui`, and Segoe UI on Windows. Found once.
pub fn system_family() -> Option<&'static str> {
    static FAMILY: OnceLock<Option<String>> = OnceLock::new();
    FAMILY.get_or_init(find_system_family).as_deref()
}

/// The families the font menu offers after Inter: the [`OFFERED`] ones
/// installed here and the platform's own, in alphabetical order. Found once.
pub fn installed() -> &'static [String] {
    static INSTALLED: OnceLock<Vec<String>> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let mut families: Vec<String> = OFFERED
            .iter()
            .filter(|family| is_installed(family))
            .map(|family| (*family).to_owned())
            .collect();
        if let Some(system) = system_family()
            && !families
                .iter()
                .any(|family| family.eq_ignore_ascii_case(system))
        {
            families.push(system.to_owned());
        }
        families.sort_by_key(|family| family.to_lowercase());
        families
    })
}

/// `family` at each [`Weight`], in [`Weight::ALL`] order, or `None` when it
/// is not installed or cannot be read. Read once per family.
pub fn faces(family: &str) -> Option<Arc<Vec<Arc<FontData>>>> {
    #[allow(clippy::type_complexity, reason = "a cache keyed by family")]
    static LOADED: OnceLock<Mutex<HashMap<String, Option<Arc<Vec<Arc<FontData>>>>>>> =
        OnceLock::new();
    let loaded = LOADED.get_or_init(Mutex::default);
    let key = family.to_lowercase();
    if let Some(found) = loaded.lock().ok()?.get(&key) {
        return found.clone();
    }
    let found = load(family).map(Arc::new);
    if let Ok(mut loaded) = loaded.lock() {
        loaded.insert(key, found.clone());
    }
    found
}

/// Whether `family` has an upright face here. fontconfig's list is
/// enough on Linux; elsewhere the files are read.
fn is_installed(family: &str) -> bool {
    if cfg!(all(unix, not(target_os = "macos"))) {
        !family_files(family).is_empty()
    } else {
        !candidates(family).is_empty()
    }
}

/// One upright face of a family.
#[derive(Clone, Debug, PartialEq)]
struct Candidate {
    path: PathBuf,
    index: u32,
    weight: f32,
    variable: bool,
}

fn load(family: &str) -> Option<Vec<Arc<FontData>>> {
    let candidates = candidates(family);
    let mut files: HashMap<PathBuf, Arc<Vec<u8>>> = HashMap::new();
    Weight::ALL
        .iter()
        .map(|weight| {
            let chosen = choose(&candidates, weight.value())?;
            let bytes = match files.get(&chosen.path) {
                Some(bytes) => bytes.clone(),
                None => {
                    let bytes = Arc::new(std::fs::read(&chosen.path).ok()?);
                    files.insert(chosen.path.clone(), bytes.clone());
                    bytes
                }
            };
            let mut data = FontData::from_owned(bytes.as_ref().clone());
            data.index = chosen.index;
            if chosen.variable {
                data.tweak.coords = egui::epaint::text::VariationCoords::new(coords(
                    &bytes,
                    chosen.index,
                    weight.value(),
                ));
            }
            Some(Arc::new(data))
        })
        .collect()
}

/// The variable face if there is one, else the face nearest `weight`, the
/// heavier on a tie.
fn choose(candidates: &[Candidate], weight: f32) -> Option<&Candidate> {
    if let Some(variable) = candidates.iter().find(|face| face.variable) {
        return Some(variable);
    }
    candidates.iter().min_by(|one, other| {
        let distance = |face: &Candidate| (face.weight - weight).abs();
        distance(one)
            .total_cmp(&distance(other))
            .then(other.weight.total_cmp(&one.weight))
    })
}

/// `wght` at the weight and `opsz` at the text size, for the axes the face
/// has, each clamped to its range.
fn coords(bytes: &[u8], index: u32, weight: f32) -> Vec<([u8; 4], f32)> {
    let Ok(font) = skrifa::FontRef::from_index(bytes, index) else {
        return Vec::new();
    };
    let mut coords = Vec::new();
    for axis in font.axes().iter() {
        let clamp = |value: f32| value.clamp(axis.min_value(), axis.max_value());
        match &axis.tag().to_be_bytes() {
            b"wght" => coords.push((*b"wght", clamp(weight))),
            b"opsz" => coords.push((*b"opsz", clamp(TEXT_OPTICAL_SIZE))),
            _ => {}
        }
    }
    coords
}

/// Every upright face of `family` installed here.
fn candidates(family: &str) -> Vec<Candidate> {
    let mut found = Vec::new();
    for path in family_files(family) {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        for (index, font) in file_faces(&data) {
            let attributes = font.attributes();
            if attributes.style != skrifa::attribute::Style::Normal
                || attributes.stretch != skrifa::attribute::Stretch::NORMAL
                || !named(&font, family)
            {
                continue;
            }
            let variable = font
                .axes()
                .iter()
                .any(|axis| axis.tag().to_be_bytes() == *b"wght");
            found.push(Candidate {
                path: path.clone(),
                index,
                weight: font.attributes().weight.value(),
                variable,
            });
        }
    }
    found.sort_by(|one, other| (&one.path, one.index).cmp(&(&other.path, other.index)));
    found.dedup();
    found
}

/// Every face in a font file.
fn file_faces(data: &[u8]) -> Vec<(u32, skrifa::FontRef<'_>)> {
    match skrifa::raw::FileRef::new(data) {
        Ok(skrifa::raw::FileRef::Font(font)) => vec![(0, font)],
        Ok(skrifa::raw::FileRef::Collection(collection)) => (0..collection.len().min(MAX_FACES))
            .filter_map(|index| collection.get(index).ok().map(|font| (index, font)))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Whether a face belongs to `family`, under its typographic family name or
/// its family name, ignoring case (`Noto Sans Medium` is typographic family
/// `Noto Sans`).
fn named(font: &skrifa::FontRef<'_>, family: &str) -> bool {
    [
        skrifa::string::StringId::TYPOGRAPHIC_FAMILY_NAME,
        skrifa::string::StringId::FAMILY_NAME,
    ]
    .iter()
    .any(|id| {
        font.localized_strings(*id)
            .english_or_first()
            .is_some_and(|name| name.to_string().eq_ignore_ascii_case(family))
    })
}

/// The font files that may hold `family`: fontconfig's answer on Linux, and
/// elsewhere the files in the font directories named after it
/// (`NotoSans-Bold.ttf`, `OpenSans[wdth,wght].ttf`).
fn family_files(family: &str) -> Vec<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let pattern = format!(":family={}:slant=0:width=100", escape(family));
        command_output("fc-list", &["--format", "%{file}\n", &pattern])
            .map(|out| {
                let mut files: Vec<PathBuf> = out
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(PathBuf::from)
                    .collect();
                files.sort();
                files.dedup();
                files
            })
            .unwrap_or_default()
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let key = file_key(family);
        let mut files = Vec::new();
        for dir in fastframe_fonts::system::font_directories() {
            walk(&dir, 0, &key, &mut files);
        }
        files
    }
}

/// A family name as the start of its file names: letters and digits,
/// lowercase.
#[cfg_attr(all(unix, not(target_os = "macos")), allow(dead_code))]
fn file_key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// Whether a font file name starts with the family's key and the family's
/// name ends there (`NotoSans-Bold`, not `NotoSansArabic-Bold`).
#[cfg_attr(all(unix, not(target_os = "macos")), allow(dead_code))]
fn file_named(path: &Path, key: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let compact: String = stem
        .chars()
        .filter(|character| *character != ' ')
        .map(|character| character.to_ascii_lowercase())
        .collect();
    compact.strip_prefix(key).is_some_and(|rest| {
        rest.chars()
            .next()
            .is_none_or(|next| !next.is_ascii_alphanumeric())
    })
}

#[cfg_attr(all(unix, not(target_os = "macos")), allow(dead_code))]
fn walk(dir: &Path, depth: usize, key: &str, found: &mut Vec<PathBuf>) {
    if depth >= 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, depth + 1, key, found);
        } else if is_font_file(&path) && file_named(&path, key) {
            found.push(path);
        }
    }
}

#[cfg_attr(all(unix, not(target_os = "macos")), allow(dead_code))]
fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

fn find_system_family() -> Option<String> {
    #[cfg(windows)]
    {
        let fonts = std::env::var_os("SystemRoot")
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
            .join("Fonts");
        let family = if fonts.join("SegUIVar.ttf").is_file() {
            "Segoe UI Variable"
        } else {
            "Segoe UI"
        };
        Some(family.to_owned())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
            desktop
                .split(':')
                .any(|part| part.eq_ignore_ascii_case("KDE"))
        });
        if kde {
            let configured = directories::BaseDirs::new()
                .and_then(|dirs| std::fs::read_to_string(dirs.config_dir().join("kdeglobals")).ok())
                .and_then(|text| kde_font(&text));
            // Plasma draws with Noto Sans until the font is changed.
            let family = configured.unwrap_or_else(|| "Noto Sans".to_owned());
            if is_installed(&family) {
                return Some(family);
            }
        }
        command_output("fc-match", &[r"system\-ui", "--format", "%{family[0]}"])
            .map(|family| family.trim().to_owned())
            .filter(|family| !family.is_empty())
    }
    #[cfg(target_os = "macos")]
    {
        None
    }
}

/// The family of KDE's general font, the first field of `font=` in
/// `[General]` of `kdeglobals` (`Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1`).
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn kde_font(kdeglobals: &str) -> Option<String> {
    let mut general = false;
    for line in kdeglobals.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            general = line == "[General]";
            continue;
        }
        if general && let Some(value) = line.strip_prefix("font=") {
            let family = value.split(',').next()?.trim();
            return (!family.is_empty()).then(|| family.to_owned());
        }
    }
    None
}

/// A family name inside a fontconfig pattern, where `-`, `:`, `,` and `\`
/// have meanings of their own.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn escape(family: &str) -> String {
    let mut escaped = String::with_capacity(family.len());
    for character in family.chars() {
        if matches!(character, '-' | ':' | ',' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(all(unix, not(target_os = "macos")))]
fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offered_families_are_in_alphabetical_order() {
        let mut sorted = OFFERED;
        sorted.sort_by_key(|family| family.to_lowercase());
        assert_eq!(sorted, OFFERED);
    }

    #[test]
    fn kde_names_its_general_font() {
        let text = "[Colors:View]\nfont=Wrong,1\n[General]\nColorScheme=Breeze\n\
                    font=Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1\n";
        assert_eq!(kde_font(text).as_deref(), Some("Noto Sans"));
        assert_eq!(kde_font("[General]\nColorScheme=Breeze\n"), None);
    }

    #[test]
    fn file_names_match_their_family_only() {
        let key = file_key("Noto Sans");
        assert!(file_named(Path::new("/f/NotoSans-Bold.ttf"), &key));
        assert!(file_named(Path::new("/f/NotoSans[wdth,wght].ttf"), &key));
        assert!(!file_named(Path::new("/f/NotoSansArabic-Bold.ttf"), &key));
        assert!(file_named(
            Path::new("/f/OpenSans-Regular.ttf"),
            &file_key("Open Sans")
        ));
        assert!(!file_named(Path::new("/f/Hackney.ttf"), &file_key("Hack")));
    }

    #[test]
    fn fontconfig_patterns_escape_their_specials() {
        assert_eq!(escape("Noto Sans"), "Noto Sans");
        assert_eq!(escape("system-ui"), r"system\-ui");
    }

    #[test]
    fn the_nearest_static_weight_is_chosen_and_ties_go_heavier() {
        let face = |weight: f32| Candidate {
            path: PathBuf::from(format!("/f/{weight}.ttf")),
            index: 0,
            weight,
            variable: false,
        };
        let faces = [face(400.0), face(700.0)];
        assert_eq!(choose(&faces, 500.0).unwrap().weight, 400.0);
        assert_eq!(choose(&faces, 550.0).unwrap().weight, 700.0);
        assert_eq!(choose(&faces, 600.0).unwrap().weight, 700.0);
    }
}
