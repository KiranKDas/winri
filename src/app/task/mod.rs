//! Asynchronous `Task`s for the iced runtime.
use anyhow::Context;
use iced::Task;
use joy_error::{ResultUtilityExt, log::ResultLogExt};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

use crate::{
    app::{self},
    system,
    window::Window,
};

/// Refocuses the desktop if the overlay currently holds focus.
///
/// The overlay steals focus when first shown, which breaks keystroke capture
/// until another window is clicked. Run after every message as a cheap guard
/// against that state.
pub fn ensure_overlay_not_focused(overlay_window_id: iced::window::Id) -> Task<app::Message> {
    iced::window::raw_id::<app::Message>(overlay_window_id).then(|raw_id| {
        unfocus_window(raw_id)
            .context("unfocusing overlay window")
            .error()
            .log_err()
            .discard();
        prevent_window_blur(raw_id)
            .context("preventing overlay blur")
            .error()
            .log_err()
            .discard();
        Task::none()
    })
}

fn unfocus_window(raw_id: u64) -> anyhow::Result<()> {
    let Ok(focused_window) = Window::focused() else {
        return Ok(());
    };

    let overlay_window = Window::from_safe_hwnd(raw_id).context(format!(
        "given raw id ({raw_id}) is invalid: expected overlay raw id (aka. HWND)"
    ))?;

    let desktop_window = system::get_desktop_window().context("getting desktop window")?;
    if focused_window == overlay_window {
        desktop_window.focus()?;
    }

    Ok(())
}

/// Prevents mods such as Windhawk's "Translucent Windows" from applying blur,
/// acrylic, or mica effects to Winri's transparent overlay window.
///
/// Windhawk checks:
/// `if ((styleEx & WS_EX_NOACTIVATE) || (styleEx & WS_EX_TRANSPARENT)) return FALSE;`
/// By ensuring `WS_EX_NOACTIVATE`, `WS_EX_TRANSPARENT`, and `WS_EX_TOOLWINDOW` are set,
/// Windhawk skips the overlay window entirely.
///
/// Note: We must NOT set `WS_EX_LAYERED` or call `SetWindowCompositionAttribute(ACCENT_DISABLED)`,
/// as both break DX12 swapchain transparency and force DWM to render an opaque grey background.
fn prevent_window_blur(raw_id: u64) -> anyhow::Result<()> {
    let overlay_window = Window::from_safe_hwnd(raw_id).context(format!(
        "given raw id ({raw_id}) is invalid: expected overlay raw id (aka. HWND)"
    ))?;
    let hwnd = overlay_window.handle();

    unsafe {
        let current_ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let target_ex_style = (current_ex_style
            | (WS_EX_TRANSPARENT.0 as isize)
            | (WS_EX_NOACTIVATE.0 as isize)
            | (WS_EX_TOOLWINDOW.0 as isize))
            & !((WS_EX_APPWINDOW.0 as isize) | (WS_EX_LAYERED.0 as isize));
        if current_ex_style != target_ex_style {
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, target_ex_style);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    Ok(())
}
