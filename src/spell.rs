//! Spell checking for the composer, in Brazilian Portuguese only.
//!
//! ZapFast ships no dictionary: it reads the Hunspell pair `pt_BR.aff` and
//! `pt_BR.dic` from its own dictionaries folder, so the reader can bring or
//! update one, and otherwise from the system's Hunspell folders. Without
//! either, spell checking stays off. Words learned go to `pessoal.dic` in the
//! dictionaries folder, one per line.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::Waker;

/// The dictionary's file name, without the extension.
const LANGUAGE: &str = "pt_BR";
/// The reader's own words.
const PERSONAL: &str = "pessoal.dic";
/// Suggestions a menu offers.
const SUGGESTIONS: usize = 5;

/// Where the system keeps Hunspell dictionaries.
fn system_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if cfg!(target_os = "linux") {
        for folder in [
            "/usr/share/hunspell",
            "/usr/share/myspell",
            "/usr/share/myspell/dicts",
            "/usr/local/share/hunspell",
        ] {
            folders.push(PathBuf::from(folder));
        }
    }
    if cfg!(target_os = "macos") {
        folders.push(PathBuf::from("/Library/Spelling"));
        if let Some(home) = std::env::var_os("HOME") {
            folders.push(PathBuf::from(home).join("Library/Spelling"));
        }
    }
    folders
}

/// The dictionary pair in a folder, when both files are there.
fn pair_in(folder: &Path) -> Option<(PathBuf, PathBuf)> {
    let aff = folder.join(format!("{LANGUAGE}.aff"));
    let dic = folder.join(format!("{LANGUAGE}.dic"));
    (aff.is_file() && dic.is_file()).then_some((aff, dic))
}

/// The dictionary to use: the dictionaries folder first, then the system's.
pub fn find(own: &Path) -> Option<(PathBuf, PathBuf)> {
    std::iter::once(own.to_path_buf())
        .chain(system_folders())
        .find_map(|folder| pair_in(&folder))
}

/// A dictionary file as text. Older dictionaries are in Latin-1, not UTF-8.
fn read_text(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => error.into_bytes().iter().map(|&b| b as char).collect(),
    })
}

/// A loaded dictionary and what the reader taught it.
pub struct Speller {
    dictionary: spellbook::Dictionary,
    /// The `.dic` file in use.
    pub source: PathBuf,
    personal_file: PathBuf,
    personal: Mutex<HashSet<String>>,
    ignored: Mutex<HashSet<String>>,
    known: Mutex<HashMap<String, bool>>,
}

impl Speller {
    fn load(aff: &Path, dic: &Path, personal_file: PathBuf) -> Result<Self, String> {
        let aff_text = read_text(aff).map_err(|error| error.to_string())?;
        let dic_text = read_text(dic).map_err(|error| error.to_string())?;
        let dictionary =
            spellbook::Dictionary::new(&aff_text, &dic_text).map_err(|error| error.to_string())?;
        let personal = std::fs::read_to_string(&personal_file)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(Self {
            dictionary,
            source: dic.to_path_buf(),
            personal_file,
            personal: Mutex::new(personal),
            ignored: Mutex::new(HashSet::new()),
            known: Mutex::new(HashMap::new()),
        })
    }

    /// Whether a word is spelled right, or the reader said so.
    pub fn check(&self, word: &str) -> bool {
        let word = normalize(word);
        if lock(&self.personal).contains(&word) || lock(&self.ignored).contains(&word) {
            return true;
        }
        if let Some(&known) = lock(&self.known).get(&word) {
            return known;
        }
        let right = self.dictionary.check(&word);
        lock(&self.known).insert(word, right);
        right
    }

    /// The words of a text that are misspelled, as character ranges.
    pub fn misspelled(&self, text: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = text.chars().collect();
        words(text)
            .into_iter()
            .filter(|&(start, end)| !self.check(&chars[start..end].iter().collect::<String>()))
            .collect()
    }

    /// A few corrections for a word, the likeliest first.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut found = Vec::new();
        self.dictionary.suggest(&normalize(word), &mut found);
        // Hunspell also proposes splitting a word in two, which is never what was meant.
        found.retain(|suggestion| !suggestion.contains(' '));
        found.truncate(SUGGESTIONS);
        found
    }

    /// Keeps a word as right from now on, in the personal dictionary.
    pub fn learn(&self, word: &str) -> std::io::Result<()> {
        let word = normalize(word);
        if !lock(&self.personal).insert(word.clone()) {
            return Ok(());
        }
        if let Some(parent) = self.personal_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.personal_file)?;
        writeln!(file, "{word}")
    }

    /// Lets a word pass until ZapFast closes.
    pub fn ignore(&self, word: &str) {
        lock(&self.ignored).insert(normalize(word));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The typographic apostrophe reads as the plain one the dictionary uses.
fn normalize(word: &str) -> String {
    word.replace('\u{2019}', "'")
}

/// What the composer's spell checking is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Off,
    Loading,
    Ready(PathBuf),
    Missing,
    Failed(String),
}

type Slot = Arc<OnceLock<Result<Arc<Speller>, Option<String>>>>;

/// The composer's spell checker, loaded in the background.
pub struct Spelling {
    folder: PathBuf,
    slot: Option<Slot>,
}

impl Spelling {
    pub fn new(folder: PathBuf) -> Self {
        Self { folder, slot: None }
    }

    /// The folder the reader puts a dictionary in.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Looks for a dictionary again and loads it.
    pub fn start(&mut self, waker: Waker) {
        let slot: Slot = Arc::new(OnceLock::new());
        self.slot = Some(slot.clone());
        let folder = self.folder.clone();
        let spawned = std::thread::Builder::new()
            .name("spell-load".into())
            .spawn(move || {
                let result = match find(&folder) {
                    None => Err(None),
                    Some((aff, dic)) => Speller::load(&aff, &dic, folder.join(PERSONAL))
                        .map(Arc::new)
                        .map_err(|error| {
                            log::warn!("could not load the dictionary {}: {error}", dic.display());
                            Some(error)
                        }),
                };
                if let Ok(speller) = &result {
                    log::info!("spell checking with {}", speller.source.display());
                }
                let _ = slot.set(result);
                waker.wake();
            });
        if let Err(error) = spawned {
            log::warn!("could not start loading the dictionary: {error}");
            self.slot = None;
        }
    }

    pub fn stop(&mut self) {
        self.slot = None;
    }

    pub fn is_started(&self) -> bool {
        self.slot.is_some()
    }

    /// The checker, once loaded.
    pub fn ready(&self) -> Option<Arc<Speller>> {
        match self.slot.as_ref()?.get()? {
            Ok(speller) => Some(speller.clone()),
            Err(_) => None,
        }
    }

    pub fn status(&self) -> Status {
        let Some(slot) = &self.slot else {
            return Status::Off;
        };
        match slot.get() {
            None => Status::Loading,
            Some(Ok(speller)) => Status::Ready(speller.source.clone()),
            Some(Err(None)) => Status::Missing,
            Some(Err(Some(error))) => Status::Failed(error.clone()),
        }
    }
}

/// Character ranges of the words worth checking in a text.
///
/// Links, e-mail addresses, mentions, hashtags and code between backticks are
/// left alone, and so are words with digits and acronyms in capitals. A hyphen
/// or an apostrophe between letters stays inside the word ("fazê-lo").
pub fn words(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut code = false;
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && !chars[index].is_whitespace() {
            index += 1;
        }
        let chunk = &chars[start..index];
        let ticks = chunk.iter().filter(|&&c| c == '`').count();
        let chunk_text: String = chunk.iter().collect();
        let skipped = code
            || ticks > 0
            || chunk_text.contains("://")
            || chunk_text.to_lowercase().starts_with("www.")
            || chunk_text.contains('@')
            || chunk_text.starts_with('#')
            || chunk_text.starts_with('/');
        if ticks % 2 == 1 {
            code = !code;
        }
        if skipped {
            continue;
        }
        let mut at = 0;
        while at < chunk.len() {
            if !chunk[at].is_alphanumeric() {
                at += 1;
                continue;
            }
            let word_start = at;
            while at < chunk.len()
                && (chunk[at].is_alphanumeric()
                    || (matches!(chunk[at], '-' | '\'' | '\u{2019}')
                        && chunk.get(at + 1).is_some_and(|c| c.is_alphabetic())
                        && at > word_start))
            {
                at += 1;
            }
            let word = &chunk[word_start..at];
            let digits = word.iter().any(|c| c.is_numeric());
            let letters = word.iter().filter(|c| c.is_alphabetic()).count();
            let acronym = letters > 1
                && word
                    .iter()
                    .filter(|c| c.is_alphabetic())
                    .all(|c| c.is_uppercase());
            if !digits && !acronym && letters > 0 {
                found.push((start + word_start, start + at));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picked(text: &str) -> Vec<String> {
        words(text)
            .into_iter()
            .map(|(start, end)| text.chars().skip(start).take(end - start).collect())
            .collect()
    }

    #[test]
    fn words_keep_hyphens_and_apostrophes_inside() {
        assert_eq!(
            picked("Vou fazê-lo, d'água; -já"),
            ["Vou", "fazê-lo", "d'água", "já"]
        );
    }

    #[test]
    fn links_mentions_code_numbers_and_acronyms_are_left_alone() {
        assert_eq!(
            picked("veja https://exemplo.com.br e www.site.com com @Ana #tag `codigo errado` CPF 2x ok"),
            ["veja", "e", "com", "ok"]
        );
    }

    #[test]
    fn a_dictionary_in_the_own_folder_comes_before_the_system() {
        let folder = tempfile::tempdir().unwrap();
        assert!(pair_in(folder.path()).is_none());
        std::fs::write(folder.path().join("pt_BR.aff"), "SET UTF-8\n").unwrap();
        assert!(pair_in(folder.path()).is_none(), "both files are needed");
        std::fs::write(folder.path().join("pt_BR.dic"), "1\ncasa\n").unwrap();
        assert_eq!(
            find(folder.path()).map(|(_, dic)| dic),
            Some(folder.path().join("pt_BR.dic"))
        );
    }

    #[test]
    fn a_small_dictionary_checks_suggests_and_learns() {
        let folder = tempfile::tempdir().unwrap();
        let aff = folder.path().join("pt_BR.aff");
        let dic = folder.path().join("pt_BR.dic");
        std::fs::write(&aff, "SET UTF-8\nTRY aeiouãç\n").unwrap();
        std::fs::write(&dic, "3\ncasa\nentão\nação\n").unwrap();
        let personal = folder.path().join(PERSONAL);
        let speller = Speller::load(&aff, &dic, personal.clone()).unwrap();
        assert_eq!(speller.misspelled("a casa entao"), [(0, 1), (7, 12)]);
        assert!(speller.suggest("entao").contains(&"então".to_owned()));
        speller.learn("zapfast").unwrap();
        assert!(speller.check("zapfast"));
        assert_eq!(std::fs::read_to_string(&personal).unwrap(), "zapfast\n");
        speller.ignore("blá");
        assert!(speller.check("blá"));
        let again = Speller::load(&aff, &dic, personal).unwrap();
        assert!(again.check("zapfast"), "learned words last");
        assert!(!again.check("blá"), "ignored ones do not");
    }

    #[test]
    fn a_latin1_dictionary_still_reads() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("x.dic");
        std::fs::write(&path, [b'a', 0xE7, 0xE3, b'o']).unwrap();
        assert_eq!(read_text(&path).unwrap(), "ação");
    }
}
