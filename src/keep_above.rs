//! Keeping the call window above other windows, and in the top-right corner of the main window's
//! screen.
//!
//! On Windows, macOS and X11 the window does both itself (`ViewportCommand::WindowLevel` and its
//! position). Wayland has no protocol for either, so winit ignores both requests there; on KDE
//! Plasma the compositor still takes them through a KWin script, loaded over D-Bus, that moves the
//! window and sets its own "Keep above others". Other Wayland compositors cannot be asked, and the
//! call window offers no pin there.

/// How the window can be kept above the others on this desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// The window level, which winit sets.
    WindowLevel,
    /// A KWin script (KDE Plasma on Wayland).
    KWin,
    /// Nothing this desktop offers.
    Unsupported,
}

/// How this session keeps a window above the others.
pub fn method() -> Method {
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            && std::env::var("WINIT_UNIX_BACKEND").map_or(true, |backend| backend != "x11");
        if wayland {
            return if is_kde() {
                Method::KWin
            } else {
                Method::Unsupported
            };
        }
    }
    Method::WindowLevel
}

#[cfg(target_os = "linux")]
fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|desktop| {
            desktop
                .split(':')
                .any(|name| name.eq_ignore_ascii_case("KDE"))
        })
        .unwrap_or(false)
}

/// How far the call window sits from the top and right edges of the screen, in points.
pub const EDGE: f32 = 16.0;

/// The KWin script for this process's window with that title: with `place`, it moves the window to
/// the top-right corner of the usable area (panels left out) of the screen the main window is on;
/// then it sets its "Keep above others".
///
/// KWin 6 lists windows with `windowList()`, KWin 5 with `clientList()`.
pub fn kwin_script(pid: u32, title: &str, place: bool, above: bool) -> String {
    let title = serde_json::to_string(title).unwrap_or_else(|_| "\"\"".to_owned());
    let edge = EDGE;
    format!(
        "(function () {{\n\
         const windows = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         let call = null;\n\
         let main = null;\n\
         for (const window of windows) {{\n\
         if (window.pid !== {pid}) continue;\n\
         if (window.caption === {title}) call = window;\n\
         else if (window.normalWindow) main = window;\n\
         }}\n\
         if (!call) return;\n\
         if ({place}) {{\n\
         const area = workspace.clientArea(KWin.MaximizeArea, main || call);\n\
         const frame = call.frameGeometry;\n\
         call.frameGeometry = {{\n\
         x: area.x + area.width - frame.width - {edge},\n\
         y: area.y + {edge},\n\
         width: frame.width,\n\
         height: frame.height,\n\
         }};\n\
         }}\n\
         call.keepAbove = {above};\n\
         }})();\n"
    )
}

/// Asks KWin to place the window with that title (with `place`) and to keep it above the others or
/// to stop, off this thread: it talks to the session bus and waits a moment for a new window to be
/// mapped.
#[cfg(target_os = "linux")]
pub fn kwin_arrange(title: String, place: bool, above: bool, delay: std::time::Duration) {
    let spawned = std::thread::Builder::new()
        .name("keep-above".to_owned())
        .spawn(move || {
            std::thread::sleep(delay);
            if let Err(error) = kwin::run(&title, place, above) {
                log::warn!("could not ask KWin to arrange the call window: {error}");
            }
        });
    if let Err(error) = spawned {
        log::warn!("could not start the keep-above thread: {error}");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn kwin_arrange(_title: String, _place: bool, _above: bool, _delay: std::time::Duration) {}

#[cfg(target_os = "linux")]
mod kwin {
    use zbus::{Connection, Proxy};

    const SERVICE: &str = "org.kde.KWin";
    const PLUGIN: &str = "zapfast-keep-above";

    pub fn run(title: &str, place: bool, above: bool) -> Result<(), String> {
        let path =
            std::env::temp_dir().join(format!("zapfast-keep-above-{}.js", std::process::id()));
        std::fs::write(
            &path,
            super::kwin_script(std::process::id(), title, place, above),
        )
        .map_err(|error| format!("could not write the script: {error}"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let result = runtime.block_on(load_and_run(&path.to_string_lossy()));
        let _ = std::fs::remove_file(&path);
        result.map_err(|error| error.to_string())
    }

    async fn load_and_run(path: &str) -> zbus::Result<()> {
        let connection = Connection::session().await?;
        let scripting =
            Proxy::new(&connection, SERVICE, "/Scripting", "org.kde.kwin.Scripting").await?;
        // A script left loaded by an earlier call would make loading this one fail.
        let _: zbus::Result<bool> = scripting.call("unloadScript", &(PLUGIN,)).await;
        let id: i32 = scripting.call("loadScript", &(path, PLUGIN)).await?;
        // KWin 6 puts the script under /Scripting/Script<id>, KWin 5 under /<id>.
        let mut ran = Err(zbus::Error::Failure("the script was not found".to_owned()));
        for object in [format!("/Scripting/Script{id}"), format!("/{id}")] {
            let script =
                Proxy::new(&connection, SERVICE, object.as_str(), "org.kde.kwin.Script").await?;
            ran = script.call::<_, _, ()>("run", &()).await;
            if ran.is_ok() {
                break;
            }
        }
        // KWin reads the file and evaluates it after `run` returns, so both stay a moment.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _: zbus::Result<bool> = scripting.call("unloadScript", &(PLUGIN,)).await;
        ran
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kwin_script_names_the_window_by_process_and_quoted_title() {
        let script = kwin_script(42, "Ana \"Bia\" </script>", false, true);
        assert!(script.contains("window.pid !== 42"));
        assert!(script.contains(r#"window.caption === "Ana \"Bia\" </script>""#));
        assert!(script.contains("call.keepAbove = true;"));
        assert!(script.contains("if (false)"), "no move unless asked");
        assert!(kwin_script(1, "x", false, false).contains("call.keepAbove = false;"));
    }

    #[test]
    fn a_placed_window_goes_to_the_top_right_of_the_main_windows_screen() {
        let script = kwin_script(1, "x", true, true);
        assert!(script.contains("if (true)"));
        assert!(script.contains("clientArea(KWin.MaximizeArea, main || call)"));
        assert!(script.contains("area.x + area.width - frame.width - 16"));
        assert!(script.contains("y: area.y + 16"));
    }
}
