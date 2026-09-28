//! WhatsApp links that ZapFast opens itself: `wa.me` and `api.whatsapp.com`
//! chat links, the `whatsapp://` scheme the browser hands them on as, and
//! group invites.
//!
//! [`handler`] registers ZapFast as the desktop's app for `whatsapp://`
//! links, which the pages behind `wa.me` open when a browser visits them.

/// A WhatsApp link that ZapFast can open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaLink {
    /// A chat with a phone number, optionally with text for the composer.
    Chat { phone: String, text: Option<String> },
    /// A group invite code.
    Invite(String),
}

/// Longest text a link may put in the composer, as WhatsApp's own limit.
const TEXT_LIMIT: usize = 65_536;

/// Reads a WhatsApp chat or invite link. Anything else, including a `wa.me`
/// link without a number, is left for the browser.
pub fn parse(value: &str) -> Option<WaLink> {
    let value = value.trim();
    if value.chars().any(char::is_control) || value.contains('\\') {
        return None;
    }
    if let Some(code) = crate::safety::group_invite_code(value) {
        return Some(WaLink::Invite(code));
    }
    let url = reqwest::Url::parse(value).ok()?;
    let query = |key: &str| {
        url.query_pairs()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.into_owned())
    };
    let phone = match url.scheme() {
        "whatsapp" => match url.host_str()? {
            "send" => query("phone")?,
            "chat" => {
                let code = query("code")?;
                return crate::safety::group_invite_code(&format!(
                    "https://chat.whatsapp.com/{code}"
                ))
                .map(WaLink::Invite);
            }
            _ => return None,
        },
        "http" | "https" => {
            let host = url.host_str()?.to_ascii_lowercase();
            let host = host.strip_prefix("www.").unwrap_or(&host);
            let mut segments = url.path_segments()?.filter(|segment| !segment.is_empty());
            match host {
                "wa.me" => {
                    let phone = percent_decode(segments.next()?)?;
                    if segments.next().is_some() {
                        return None;
                    }
                    phone
                }
                "api.whatsapp.com" | "web.whatsapp.com" | "whatsapp.com" => {
                    if segments.next()? != "send" || segments.next().is_some() {
                        return None;
                    }
                    query("phone")?
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    let phone = phone_digits(&phone)?;
    let text = query("text")
        .map(|text| {
            text.chars()
                .filter(|&c| !c.is_control() || c == '\n' || c == '\t')
                .take(TEXT_LIMIT)
                .collect::<String>()
        })
        .filter(|text| !text.trim().is_empty());
    Some(WaLink::Chat { phone, text })
}

/// A path segment with its `%XX` escapes decoded.
fn percent_decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = segment.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The digits of an international number written with its usual `+`,
/// spaces, dashes, dots, or parentheses.
fn phone_digits(value: &str) -> Option<String> {
    let mut digits = String::new();
    for character in value.chars() {
        match character {
            '0'..='9' => digits.push(character),
            '+' | ' ' | '-' | '.' | '(' | ')' => {}
            _ => return None,
        }
    }
    ((8..=15).contains(&digits.len()) && !digits.starts_with('0')).then_some(digits)
}

/// Registration as the desktop's app for `whatsapp://` links.
pub mod handler {
    use std::io;

    /// Whether this installation can register itself for WhatsApp links.
    pub fn supported() -> bool {
        cfg!(any(target_os = "linux", windows))
            && std::env::var_os("FLATPAK_ID").is_none()
            && crate::autostart::executable().is_some()
    }

    /// Whether the running executable is the desktop's app for WhatsApp links.
    pub fn registered() -> bool {
        crate::autostart::executable().is_some_and(|executable| platform::registered(&executable))
    }

    /// Makes the running executable the app for WhatsApp links, or gives the
    /// links back when it is.
    pub fn set(enabled: bool) -> io::Result<()> {
        let executable = crate::autostart::executable().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                crate::i18n::tr("cannot locate ZapFast"),
            )
        })?;
        if enabled {
            platform::install(&executable)
        } else {
            platform::remove(&executable)
        }
    }

    #[cfg(target_os = "linux")]
    mod platform {
        use std::io;
        use std::path::{Path, PathBuf};

        /// A hidden entry of its own, so the menu entry a package installs
        /// stays untouched and the link always reaches this executable.
        const DESKTOP_ID: &str = "zapfast-links.desktop";
        const MIME: &str = "x-scheme-handler/whatsapp";
        const DEFAULTS: &str = "[Default Applications]";

        fn home_dir(variable: &str, fallback: &str) -> Option<PathBuf> {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(fallback)))
        }

        fn applications() -> Option<PathBuf> {
            home_dir("XDG_DATA_HOME", ".local/share").map(|data| data.join("applications"))
        }

        fn mimeapps() -> Option<PathBuf> {
            home_dir("XDG_CONFIG_HOME", ".config").map(|config| config.join("mimeapps.list"))
        }

        pub(super) fn desktop_entry(executable: &Path) -> String {
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=ZapFast\n\
                 Comment=Opens WhatsApp links in ZapFast\n\
                 Comment[pt_BR]=Abre links do WhatsApp no ZapFast\n\
                 Exec={} %u\n\
                 Icon=zapfast\n\
                 Terminal=false\n\
                 NoDisplay=true\n\
                 MimeType={MIME};\n",
                crate::autostart::exec_quote(&executable.to_string_lossy())
            )
        }

        pub fn registered(executable: &Path) -> bool {
            let entry = applications().map(|dir| dir.join(DESKTOP_ID));
            let defaults = mimeapps().and_then(|path| std::fs::read_to_string(path).ok());
            entry.and_then(|path| std::fs::read_to_string(path).ok())
                == Some(desktop_entry(executable))
                && defaults.is_some_and(|text| default_of(&text).as_deref() == Some(DESKTOP_ID))
        }

        pub fn install(executable: &Path) -> io::Result<()> {
            let no_folder = || io::Error::other(crate::i18n::tr("no configuration directory"));
            let dir = applications().ok_or_else(no_folder)?;
            let entry = dir.join(DESKTOP_ID);
            let wanted = desktop_entry(executable);
            let mut changed = false;
            if std::fs::read_to_string(&entry).ok().as_deref() != Some(wanted.as_str()) {
                std::fs::create_dir_all(&dir)?;
                std::fs::write(&entry, wanted)?;
                changed = true;
            }
            let list = mimeapps().ok_or_else(no_folder)?;
            let text = read_or_empty(&list)?;
            let updated = with_default(&text, Some(DESKTOP_ID));
            if updated != text {
                std::fs::create_dir_all(list.parent().expect("configuration folder"))?;
                std::fs::write(&list, updated)?;
                changed = true;
            }
            if changed {
                refresh_caches(dir);
            }
            Ok(())
        }

        pub fn remove(_executable: &Path) -> io::Result<()> {
            if let Some(list) = mimeapps() {
                let text = read_or_empty(&list)?;
                if default_of(&text).as_deref() == Some(DESKTOP_ID) {
                    std::fs::write(&list, with_default(&text, None))?;
                }
            }
            if let Some(dir) = applications() {
                match std::fs::remove_file(dir.join(DESKTOP_ID)) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                    _ => refresh_caches(dir),
                }
            }
            Ok(())
        }

        fn read_or_empty(path: &Path) -> io::Result<String> {
            match std::fs::read_to_string(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
                result => result,
            }
        }

        /// The first app `mimeapps.list` names for WhatsApp links.
        pub(super) fn default_of(text: &str) -> Option<String> {
            let mut in_defaults = false;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_defaults = line == DEFAULTS;
                } else if in_defaults
                    && let Some((key, value)) = line.split_once('=')
                    && key.trim() == MIME
                {
                    return value
                        .split(';')
                        .map(str::trim)
                        .find(|id| !id.is_empty())
                        .map(str::to_owned);
                }
            }
            None
        }

        /// `mimeapps.list` with WhatsApp links going to `desktop_id`, or to
        /// no default app. Every other line is kept as it was.
        pub(super) fn with_default(text: &str, desktop_id: Option<&str>) -> String {
            let line = desktop_id.map(|id| format!("{MIME}={id};"));
            let mut out: Vec<String> = Vec::new();
            let mut in_defaults = false;
            let mut placed = false;
            for raw in text.lines() {
                let trimmed = raw.trim();
                if trimmed.starts_with('[') {
                    in_defaults = trimmed == DEFAULTS;
                    out.push(raw.to_owned());
                    if in_defaults
                        && !placed
                        && let Some(line) = &line
                    {
                        out.push(line.clone());
                        placed = true;
                    }
                    continue;
                }
                let ours = in_defaults
                    && trimmed
                        .split_once('=')
                        .is_some_and(|(key, _)| key.trim() == MIME);
                if !ours {
                    out.push(raw.to_owned());
                }
            }
            if !placed && let Some(line) = line {
                if out.last().is_some_and(|last| !last.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push(DEFAULTS.to_owned());
                out.push(line);
            }
            let mut text = out.join("\n");
            text.push('\n');
            text
        }

        /// Asks the desktop to notice the entry; KDE apps only see it once
        /// their service cache is rebuilt. Failures only delay that.
        fn refresh_caches(dir: PathBuf) {
            let _ = std::thread::Builder::new()
                .name("zapfast-links".to_owned())
                .spawn(move || {
                    let quiet = |program: &str, arguments: &[&std::ffi::OsStr]| {
                        let _ = std::process::Command::new(program)
                            .args(arguments)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status();
                    };
                    quiet("update-desktop-database", &[dir.as_os_str()]);
                    quiet("kbuildsycoca6", &[]);
                    quiet("kbuildsycoca5", &[]);
                });
        }
    }

    #[cfg(windows)]
    mod platform {
        use std::io;
        use std::os::windows::ffi::OsStrExt;
        use std::path::Path;
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
        };

        const KEY: &str = r"Software\Classes\whatsapp";
        const COMMAND: &str = r"Software\Classes\whatsapp\shell\open\command";
        const ICON: &str = r"Software\Classes\whatsapp\DefaultIcon";

        fn wide(text: &str) -> Vec<u16> {
            std::ffi::OsStr::new(text)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }

        fn command(executable: &Path) -> String {
            format!("\"{}\" \"%1\"", executable.display())
        }

        fn read(key: &str) -> Option<String> {
            let key = wide(key);
            let mut buffer = vec![0u16; 1024];
            let mut size = (buffer.len() * 2) as u32;
            // SAFETY: null-terminated UTF-16 key, a null value name for the
            // default value, and a buffer whose byte size is passed along.
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    std::ptr::null(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    buffer.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            if status != ERROR_SUCCESS {
                return None;
            }
            let length = (size as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(
                &buffer[..length.min(buffer.len())],
            ))
        }

        fn write(key: &str, value: Option<&str>, data: &str) -> io::Result<()> {
            let key = wide(key);
            let name = value.map(wide);
            let data = wide(data);
            // SAFETY: null-terminated UTF-16 strings; the byte length includes
            // the terminator, as REG_SZ requires.
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr()),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            };
            if status == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(status as i32))
            }
        }

        pub fn registered(executable: &Path) -> bool {
            read(COMMAND).as_deref() == Some(command(executable).as_str())
        }

        pub fn install(executable: &Path) -> io::Result<()> {
            if registered(executable) {
                return Ok(());
            }
            write(KEY, None, "URL:WhatsApp")?;
            write(KEY, Some("URL Protocol"), "")?;
            write(ICON, None, &format!("\"{}\",0", executable.display()))?;
            write(COMMAND, None, &command(executable))
        }

        pub fn remove(executable: &Path) -> io::Result<()> {
            // Another app's registration is not ours to delete.
            if !registered(executable) {
                return Ok(());
            }
            let key = wide(KEY);
            // SAFETY: a null-terminated UTF-16 subkey of HKEY_CURRENT_USER.
            let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
            const ERROR_FILE_NOT_FOUND: u32 = 2;
            if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(status as i32))
            }
        }
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    mod platform {
        use std::io;
        use std::path::Path;

        pub fn registered(_executable: &Path) -> bool {
            false
        }

        pub fn install(_executable: &Path) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }

        pub fn remove(_executable: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    mod tests {
        use super::platform::{default_of, desktop_entry, with_default};

        #[test]
        fn the_entry_hands_the_link_to_this_executable() {
            let entry = desktop_entry(std::path::Path::new("/home/a b/.local/bin/zapfast"));
            assert!(entry.contains("Exec=\"/home/a b/.local/bin/zapfast\" %u\n"));
            assert!(entry.contains("MimeType=x-scheme-handler/whatsapp;\n"));
            assert!(entry.contains("NoDisplay=true\n"));
        }

        #[test]
        fn only_the_whatsapp_default_changes() {
            let text = "[Added Associations]\nx-scheme-handler/whatsapp=other.desktop;\n\n\
                        [Default Applications]\ntext/html=firefox.desktop\n\
                        x-scheme-handler/whatsapp=other.desktop;\n";
            let ours = with_default(text, Some("zapfast-links.desktop"));
            assert_eq!(default_of(&ours).as_deref(), Some("zapfast-links.desktop"));
            assert!(
                ours.contains("[Added Associations]\nx-scheme-handler/whatsapp=other.desktop;")
            );
            assert!(ours.contains("text/html=firefox.desktop"));
            assert_eq!(ours.matches("x-scheme-handler/whatsapp=").count(), 2);
            let removed = with_default(&ours, None);
            assert_eq!(default_of(&removed), None);
            assert!(removed.contains("text/html=firefox.desktop"));
        }

        #[test]
        fn a_missing_section_is_added() {
            let ours = with_default("", Some("zapfast-links.desktop"));
            assert_eq!(
                ours,
                "[Default Applications]\nx-scheme-handler/whatsapp=zapfast-links.desktop;\n"
            );
            let ours = with_default("[Added Associations]\na=b;\n", Some("z.desktop"));
            assert_eq!(
                ours,
                "[Added Associations]\na=b;\n\n[Default Applications]\nx-scheme-handler/whatsapp=z.desktop;\n"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WaLink, parse};

    fn chat(phone: &str, text: Option<&str>) -> Option<WaLink> {
        Some(WaLink::Chat {
            phone: phone.to_owned(),
            text: text.map(str::to_owned),
        })
    }

    #[test]
    fn chat_links_in_every_spelling() {
        assert_eq!(
            parse("https://api.whatsapp.com/send/?phone=559492777990"),
            chat("559492777990", None)
        );
        assert_eq!(
            parse("https://api.whatsapp.com/send?phone=559492777990&text=Ol%C3%A1+mundo"),
            chat("559492777990", Some("Olá mundo"))
        );
        assert_eq!(
            parse("https://wa.me/559492777990?text=Oi%20tudo%20bem"),
            chat("559492777990", Some("Oi tudo bem"))
        );
        assert_eq!(
            parse("http://WA.ME/+55 94 9277-7990/"),
            chat("559492777990", None)
        );
        assert_eq!(
            parse("https://web.whatsapp.com/send?phone=%2B559492777990"),
            chat("559492777990", None)
        );
        assert_eq!(
            parse("whatsapp://send/?phone=559492777990&text=a%0Ab&type=phone_number&app_absent=0"),
            chat("559492777990", Some("a\nb"))
        );
        assert_eq!(
            parse("  whatsapp://send?phone=559492777990&text=%20%20  "),
            chat("559492777990", None)
        );
    }

    #[test]
    fn invites_are_read_from_both_schemes() {
        let code = "AbCdEf1234567890XyZ";
        assert_eq!(
            parse(&format!("https://chat.whatsapp.com/{code}")),
            Some(WaLink::Invite(code.to_owned()))
        );
        assert_eq!(
            parse(&format!("whatsapp://chat?code={code}")),
            Some(WaLink::Invite(code.to_owned()))
        );
    }

    #[test]
    fn anything_else_is_left_for_the_browser() {
        for link in [
            "https://wa.me/",
            "https://wa.me/?text=hi",
            "https://wa.me/message/ABCDEF",
            "https://wa.me/5594/extra",
            "https://wa.me/0559492777990",
            "https://wa.me/1234567",
            "https://wa.me/1234567890123456",
            "https://wa.me/55abc",
            "https://api.whatsapp.com/other?phone=559492777990",
            "https://api.whatsapp.com/send",
            "https://wa.me.evil.example/559492777990",
            "https://evil.example/send?phone=559492777990",
            "whatsapp://call?phone=559492777990",
            "file:///wa.me/559492777990",
            "https://wa.me/5594\n92777990",
            "https:\\\\wa.me\\559492777990",
        ] {
            assert_eq!(parse(link), None, "{link}");
        }
    }
}
