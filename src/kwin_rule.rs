//! Remembering where the main window was on KDE Plasma under Wayland.
//!
//! Wayland lets no program read or choose where its window is, so eframe saves
//! the size but no position there, and KWin places each new window by its own
//! policy (in the middle of the screen, by default). KWin can remember a
//! window's position itself through a window rule, as its "Window Rules"
//! settings do. ZapFast adds one rule, for its main window only, to
//! `kwinrulesrc` and asks KWin to reload its rules. The rule is left alone
//! once it exists, since KWin writes the remembered position into it;
//! `packaging/linux/uninstall.sh` removes it.

/// The rule's group in `kwinrulesrc`. Fixed, so ZapFast finds its own rule;
/// `uninstall.sh` names the same one.
pub const RULE_ID: &str = "5f0c2d9e-8b1a-4c7e-9a43-2e6d1f7b0a58";

/// KWin's "Remember" for a rule's value.
const REMEMBER: u8 = 4;

/// `config`, the text of `kwinrulesrc`, with ZapFast's rule added and listed,
/// or `None` when it already holds the rule. Everything else is kept as it
/// was.
///
/// The rule matches the window whose class (Wayland's app id) is `app_id` and
/// whose title is exactly "ZapFast", which leaves the call window out.
pub fn with_rule(config: &str, app_id: &str, description: &str) -> Option<String> {
    let has_group = config
        .lines()
        .any(|line| line.trim() == format!("[{RULE_ID}]"));
    let listed = general_value(config, "rules")
        .is_some_and(|rules| rules.split(',').any(|id| id.trim() == RULE_ID));
    if has_group && listed {
        return None;
    }
    let mut out = if listed {
        config.to_owned()
    } else {
        list_rule(config)
    };
    if !has_group {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&format!(
            "[{RULE_ID}]\n\
             Description={description}\n\
             positionrule={REMEMBER}\n\
             title=ZapFast\n\
             titlematch=1\n\
             types=1\n\
             wmclass={app_id}\n\
             wmclassmatch=1\n"
        ));
    }
    Some(out)
}

/// A key of the `[General]` group.
fn general_value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    let mut general = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            general = line == "[General]";
        } else if general
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim());
        }
    }
    None
}

/// `config` with the rule's id at the end of `[General]`'s `rules` and
/// `count` set to the length of that list.
fn list_rule(config: &str) -> String {
    let mut ids: Vec<String> = general_value(config, "rules")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    ids.push(RULE_ID.to_owned());
    let rules = format!("rules={}", ids.join(","));
    let count = format!("count={}", ids.len());
    let mut out = String::with_capacity(config.len() + rules.len() + 16);
    let mut general = false;
    let mut seen_general = false;
    let mut wrote = (false, false);
    let finish_general = |out: &mut String, wrote: &mut (bool, bool)| {
        if !wrote.0 {
            out.push_str(&count);
            out.push('\n');
        }
        if !wrote.1 {
            out.push_str(&rules);
            out.push('\n');
        }
        *wrote = (true, true);
    };
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if general {
                // Before the blank line that ends the group, if there is one.
                let blank = out.ends_with("\n\n");
                if blank {
                    out.pop();
                }
                finish_general(&mut out, &mut wrote);
                if blank {
                    out.push('\n');
                }
            }
            general = trimmed == "[General]";
            seen_general |= general;
        } else if general && let Some((name, _)) = trimmed.split_once('=') {
            match name.trim() {
                "count" => {
                    out.push_str(&count);
                    out.push('\n');
                    wrote.0 = true;
                    continue;
                }
                "rules" => {
                    out.push_str(&rules);
                    out.push('\n');
                    wrote.1 = true;
                    continue;
                }
                _ => {}
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if general {
        finish_general(&mut out, &mut wrote);
    }
    if !seen_general {
        let mut head = format!("[General]\n{count}\n{rules}\n");
        if !out.is_empty() {
            head.push('\n');
        }
        out.insert_str(0, &head);
    }
    out
}

/// Adds the rule when this session is KDE Plasma under Wayland, off the
/// interface thread. Does nothing elsewhere, where the window's own saved
/// position already works, and inside a Flatpak, which cannot write KWin's
/// settings.
pub fn ensure() {
    #[cfg(target_os = "linux")]
    {
        if crate::keep_above::method() != crate::keep_above::Method::KWin
            || std::env::var_os("FLATPAK_ID").is_some()
        {
            return;
        }
        let description = crate::i18n::tr("ZapFast: remember where the main window was");
        let spawned = std::thread::Builder::new()
            .name("kwin-rule".to_owned())
            .spawn(move || {
                if let Err(error) = linux::ensure(description) {
                    log::warn!("could not add the KWin rule for the window position: {error}");
                }
            });
        if let Err(error) = spawned {
            log::warn!("could not start the KWin rule thread: {error}");
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;

    fn config_file() -> Option<PathBuf> {
        let dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| std::env::home_dir().map(|home| home.join(".config")))?;
        Some(dir.join("kwinrulesrc"))
    }

    pub fn ensure(description: &str) -> Result<(), String> {
        let path = config_file().ok_or("no configuration folder")?;
        let config = match std::fs::read_to_string(&path) {
            Ok(config) => config,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        let Some(updated) = super::with_rule(&config, "zapfast", description) else {
            return Ok(());
        };
        // Written whole and renamed, so KWin never reads half a file.
        let staging = path.with_file_name(".kwinrulesrc.zapfast");
        std::fs::write(&staging, updated)
            .and_then(|()| std::fs::rename(&staging, &path))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        reconfigure()
            .map_err(|error| format!("could not ask KWin to reload its rules: {error}"))?;
        log::info!("added a KWin rule that remembers the main window's position");
        Ok(())
    }

    /// KWin's `reconfigure` sends no reply, so the call does not wait for one.
    fn reconfigure() -> zbus::Result<()> {
        let connection = zbus::blocking::Connection::session()?;
        let message = zbus::message::Message::method_call("/KWin", "reconfigure")?
            .destination("org.kde.KWin")?
            .interface("org.kde.KWin")?
            .with_flags(zbus::message::Flags::NoReplyExpected)?
            .build(&())?;
        connection.send(&message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = "[5f0c2d9e-8b1a-4c7e-9a43-2e6d1f7b0a58]\n\
         Description=d\n\
         positionrule=4\n\
         title=ZapFast\n\
         titlematch=1\n\
         types=1\n\
         wmclass=zapfast\n\
         wmclassmatch=1\n";

    #[test]
    fn a_missing_file_gets_general_and_the_rule() {
        let out = with_rule("", "zapfast", "d").unwrap();
        assert_eq!(
            out,
            format!("[General]\ncount=1\nrules={RULE_ID}\n\n{RULE}")
        );
        assert_eq!(with_rule(&out, "zapfast", "d"), None, "added once");
    }

    #[test]
    fn other_rules_are_kept_and_the_list_grows() {
        let config = "[General]\ncount=2\nrules=aaa,bbb\n\n[aaa]\nDescription=A\nwmclass=a\n\n[bbb]\nposition=10,20\npositionrule=4\n";
        let out = with_rule(config, "zapfast", "d").unwrap();
        assert_eq!(
            out,
            format!(
                "[General]\ncount=3\nrules=aaa,bbb,{RULE_ID}\n\n[aaa]\nDescription=A\nwmclass=a\n\n[bbb]\nposition=10,20\npositionrule=4\n\n{RULE}"
            )
        );
    }

    #[test]
    fn a_general_without_rules_gets_them_before_the_next_group() {
        let config = "[General]\nfoo=bar\n\n[aaa]\nwmclass=a\n";
        let out = with_rule(config, "zapfast", "d").unwrap();
        assert!(out.starts_with(&format!(
            "[General]\nfoo=bar\ncount=1\nrules={RULE_ID}\n\n[aaa]\n"
        )));
        assert!(out.ends_with(RULE));
    }

    #[test]
    fn a_remembered_position_is_never_rewritten() {
        let config = format!(
            "[General]\ncount=1\nrules={RULE_ID}\n\n[{RULE_ID}]\nposition=417,19\npositionrule=4\n"
        );
        assert_eq!(with_rule(&config, "zapfast", "d"), None);
    }

    #[test]
    fn a_rule_dropped_from_the_list_is_listed_again_without_a_copy() {
        let config = format!("[General]\ncount=0\nrules=\n\n[{RULE_ID}]\nposition=1,2\n");
        let out = with_rule(&config, "zapfast", "d").unwrap();
        assert!(out.starts_with(&format!("[General]\ncount=1\nrules={RULE_ID}\n")));
        assert_eq!(out.matches(&format!("[{RULE_ID}]")).count(), 1);
        assert!(out.contains("position=1,2"));
    }
}
