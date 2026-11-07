//! Decides which windows winri manages. Tiling everything would be chaos
//! (system UI, popups, tooltips, winri's own overlay), so the rules here keep
//! the set down to real, user-facing application windows.
use std::collections::HashSet;

use crate::window::Window;

/// Window class an app can set to opt out of tiling.
pub const WINRI_IGNORED_CLASS_NAME: &str = "Winri_IgnoreWindowClass";
/// Title substring that opts a window out of tiling. winri's own windows use it
/// (see [`crate::app::State::title`]) so it never manages itself.
pub const WINRI_IGNORED_WINDOW_TITLE_SUBSTRING: &str = "[Winri Ignore Window]";

const IGNORED_CLASSES: &[&str] = &[
    "Progman",
    "TopLevelWindowForOverflowXamlIsland",
    "XamlExplorerHostIslandWindow",
    "Xaml_WindowedPopupClass",
    "Shell_TrayWnd",
    "FindMyMouse",
    WINRI_IGNORED_CLASS_NAME,
];

const IGNORED_PROCESS_NAMES: &[&str] = &[
    "Microsoft.CmdPal.UI.exe",
    "PowerToys.MeasureToolUI.exe",
    "ShareX.exe",
    "SnippingTool.exe",
    "PowerToys.PowerLauncher.exe",
    "Ditto.exe",
    "AQAUserPS.exe",
    "winri.exe",
    "winri-dev.exe",
];

macro_rules! filter_out_if {
    ($bool:expr) => {
        if $bool {
            return Ok(false);
        }
    };
}

/// Whether a single window qualifies for tiling, applying every exclusion rule
/// (visibility, cloaking, dialogs, ignored classes/processes, …).
pub fn should_be_tiled(window: Window) -> anyhow::Result<bool> {
    should_be_tiled_with_rules(window, &[])
}

/// Whether a single window qualifies for tiling, applying exclusion rules
/// and custom user window rules (`open_floating`, `block_out`).
pub fn should_be_tiled_with_rules(
    window: Window,
    rules: &[crate::config::WindowRule],
) -> anyhow::Result<bool> {
    // 1. Never tile Winri's own windows
    if let Ok(pid) = window.process_id() {
        if pid == std::process::id() {
            return Ok(false);
        }
    }

    filter_out_if!(!window.is_visible().unwrap_or(false));
    filter_out_if!(window.is_cloaked().unwrap_or(false));
    filter_out_if!(!window.is_ancestor().unwrap_or(true));
    filter_out_if!(window.is_dialog().unwrap_or(false));
    filter_out_if!(window.is_tool_window().unwrap_or(false));

    let class = window.class().unwrap_or_default();
    filter_out_if!(class == "#32770");
    filter_out_if!(IGNORED_CLASSES.contains(&class.as_str()));
    // Ignore JUCE-based audio overlays / OSDs (e.g. AQAUserPS.exe)
    if class.starts_with("JUCE_") {
        return Ok(false);
    }

    let title = window.title().unwrap_or(None);
    filter_out_if!(title.is_none());
    filter_out_if!(title.as_ref().is_some_and(|title| {
        title.contains(WINRI_IGNORED_WINDOW_TITLE_SUBSTRING)
            || title == "QA OSD"
            || title.ends_with(" OSD")
    }));

    let proc_name = window.process_name().unwrap_or_default();
    if !proc_name.is_empty() {
        filter_out_if!(IGNORED_PROCESS_NAMES
            .iter()
            .any(|&p| p.eq_ignore_ascii_case(&proc_name)));
    }
    filter_out_if!(!window.is_valid().unwrap_or(false));

    let title_str = title.as_deref().unwrap_or("");
    for rule in rules {
        if rule.matches.is_match(&proc_name, title_str, &class) {
            if rule.block_out.unwrap_or(false) || rule.open_floating.unwrap_or(false) {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

/// The current set of tileable windows: every top-level window passed through
/// [`should_be_tiled`]. This is the snapshot fed to the tiler.
#[allow(dead_code)]
pub fn opened_windows() -> anyhow::Result<HashSet<Window>> {
    opened_windows_with_rules(&[])
}

/// The current set of tileable windows respecting user window rules.
pub fn opened_windows_with_rules(
    rules: &[crate::config::WindowRule],
) -> anyhow::Result<HashSet<Window>> {
    let windows = Window::enumerate()?
        .into_iter()
        .filter(|window| should_be_tiled_with_rules(*window, rules).unwrap_or(false))
        .collect::<HashSet<_>>();

    Ok(windows)
}
