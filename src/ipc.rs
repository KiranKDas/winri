//! Inter-Process Communication (IPC) for winri via Windows Named Pipes (`\\.\pipe\winri-ipc`).
//!
//! Allows external scripts, status bars (Zebar, Yasb), and CLI invocations (`winri msg action ...`)
//! to query state and control the window manager remotely, matching `niri msg`.

use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

use iced::futures::channel::mpsc::Sender;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

use crate::app::{
    action::{Action, TilerAction},
    subscription::global::GlobalMessage,
};

pub const PIPE_NAME: &str = r"\\.\pipe\winri-ipc";
static IS_RUNNING: AtomicBool = AtomicBool::new(true);

unsafe extern "system" {
    fn CreateNamedPipeW(
        lpName: *const u16,
        dwOpenMode: u32,
        dwPipeMode: u32,
        nMaxInstances: u32,
        nOutBufferSize: u32,
        nInBufferSize: u32,
        nDefaultTimeOut: u32,
        lpSecurityAttributes: *const std::ffi::c_void,
    ) -> HANDLE;

    fn ConnectNamedPipe(
        hNamedPipe: HANDLE,
        lpOverlapped: *mut std::ffi::c_void,
    ) -> windows::core::BOOL;

    fn DisconnectNamedPipe(hNamedPipe: HANDLE) -> windows::core::BOOL;
}

const PIPE_ACCESS_DUPLEX: u32 = 0x00000003;
const PIPE_TYPE_BYTE: u32 = 0x00000000;
const PIPE_READMODE_BYTE: u32 = 0x00000000;
const PIPE_WAIT: u32 = 0x00000000;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;

/// Runs client mode for `winri msg <args...>`.
pub fn run_client(args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        println!("winri msg - CLI control for winri\n");
        println!("Usage: winri msg <subcommand> [args...]\n");
        println!("Commands:");
        println!("  action <name>     Execute an action (e.g. focus-left, cycle-width, toggle-floating)");
        println!("  query <type>      Query state (e.g. workspaces, focused, windows)");
        return Ok(());
    }

    let command = args.join(" ");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_NAME)?;

    writeln!(file, "{command}")?;
    file.flush()?;

    let mut reader = BufReader::new(file);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    print!("{response}");
    Ok(())
}

/// Spawns the background named pipe server thread.
pub fn launch_server(mut tx: Sender<GlobalMessage>) {
    let _ = thread::Builder::new()
        .name("winri-ipc-server".into())
        .spawn(move || {
            let pipe_name_wide: Vec<u16> = PIPE_NAME
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            while IS_RUNNING.load(Ordering::Relaxed) {
                let pipe_handle = unsafe {
                    CreateNamedPipeW(
                        pipe_name_wide.as_ptr(),
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        PIPE_UNLIMITED_INSTANCES,
                        4096,
                        4096,
                        0,
                        std::ptr::null(),
                    )
                };

                if pipe_handle == INVALID_HANDLE_VALUE {
                    thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                }

                let connected = unsafe { ConnectNamedPipe(pipe_handle, std::ptr::null_mut()) };
                if connected.as_bool() {
                    // Read incoming command using file descriptor wrapper
                    use std::os::windows::io::FromRawHandle;
                    let file = unsafe { std::fs::File::from_raw_handle(pipe_handle.0) };
                    let mut reader = BufReader::new(&file);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_ok() {
                        let trimmed = line.trim();
                        let response = handle_ipc_command(trimmed, &mut tx);
                        let mut writer = &file;
                        let _ = writeln!(writer, "{response}");
                        let _ = writer.flush();
                    }

                    // Let std::fs::File drop, but avoid closing raw handle twice if DisconnectNamedPipe handles it
                    use std::os::windows::io::IntoRawHandle;
                    let raw = file.into_raw_handle();
                    unsafe {
                        let _ = DisconnectNamedPipe(HANDLE(raw));
                        let _ = CloseHandle(HANDLE(raw));
                    }
                } else {
                    unsafe {
                        let _ = CloseHandle(pipe_handle);
                    }
                }
            }
        });
}

fn handle_ipc_command(line: &str, tx: &mut Sender<GlobalMessage>) -> String {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return r#"{"error": "empty command"}"#.into();
    }

    match parts[0] {
        "action" => {
            if parts.len() < 2 {
                return r#"{"error": "missing action name"}"#.into();
            }
            if let Some(action) = parse_action_name(parts[1]) {
                let _ = tx.try_send(GlobalMessage::IpcAction(action));
                r#"{"status": "ok"}"#.into()
            } else {
                format!(r#"{{"error": "unknown action '{}'"}}"#, parts[1])
            }
        }
        "query" => {
            if parts.len() < 2 {
                return r#"{"error": "missing query target"}"#.into();
            }
            match parts[1] {
                "ping" => r#"{"status": "pong"}"#.into(),
                _ => format!(r#"{{"error": "unknown query '{}'"}}"#, parts[1]),
            }
        }
        _ => format!(r#"{{"error": "unknown command '{}'"}}"#, parts[0]),
    }
}

pub fn parse_action_name(name: &str) -> Option<Action> {
    match name {
        "focus-left" | "focus-column-left" => Some(Action::Tiler(TilerAction::MoveFocusPrevious)),
        "focus-right" | "focus-column-right" => Some(Action::Tiler(TilerAction::MoveFocusNext)),
        "focus-up" | "focus-window-up" => Some(Action::Tiler(TilerAction::MoveFocusUp)),
        "focus-down" | "focus-window-down" => Some(Action::Tiler(TilerAction::MoveFocusDown)),
        "focus-workspace-up" => Some(Action::Tiler(TilerAction::FocusWorkspaceUp)),
        "focus-workspace-down" => Some(Action::Tiler(TilerAction::FocusWorkspaceDown)),
        "move-to-workspace-up" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspaceUp)),
        "move-to-workspace-down" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspaceDown)),
        "move-col-to-workspace-up" => Some(Action::Tiler(TilerAction::MoveColumnToWorkspaceUp)),
        "move-col-to-workspace-down" => Some(Action::Tiler(TilerAction::MoveColumnToWorkspaceDown)),
        "workspace-1" => Some(Action::Tiler(TilerAction::FocusWorkspace(0))),
        "workspace-2" => Some(Action::Tiler(TilerAction::FocusWorkspace(1))),
        "workspace-3" => Some(Action::Tiler(TilerAction::FocusWorkspace(2))),
        "workspace-4" => Some(Action::Tiler(TilerAction::FocusWorkspace(3))),
        "workspace-5" => Some(Action::Tiler(TilerAction::FocusWorkspace(4))),
        "workspace-6" => Some(Action::Tiler(TilerAction::FocusWorkspace(5))),
        "workspace-7" => Some(Action::Tiler(TilerAction::FocusWorkspace(6))),
        "workspace-8" => Some(Action::Tiler(TilerAction::FocusWorkspace(7))),
        "workspace-9" => Some(Action::Tiler(TilerAction::FocusWorkspace(8))),
        "move-to-workspace-1" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(0))),
        "move-to-workspace-2" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(1))),
        "move-to-workspace-3" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(2))),
        "move-to-workspace-4" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(3))),
        "move-to-workspace-5" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(4))),
        "move-to-workspace-6" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(5))),
        "move-to-workspace-7" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(6))),
        "move-to-workspace-8" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(7))),
        "move-to-workspace-9" => Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(8))),
        "move-into-col-above" => Some(Action::Tiler(TilerAction::MoveWindowIntoColumnAbove)),
        "move-into-col-below" => Some(Action::Tiler(TilerAction::MoveWindowIntoColumnBelow)),
        "swap-left" | "swap-column-left" => Some(Action::Tiler(TilerAction::SwapWithPrevious)),
        "swap-right" | "swap-column-right" => Some(Action::Tiler(TilerAction::SwapWithNext)),
        "consume-window" => Some(Action::Tiler(TilerAction::ConsumeWindow)),
        "expel-window" => Some(Action::Tiler(TilerAction::ExpelWindow)),
        "cycle-width" => Some(Action::Tiler(TilerAction::CycleWidth)),
        "maximize-column" => Some(Action::Tiler(TilerAction::MaximizeColumn)),
        "reset-window-height" => Some(Action::Tiler(TilerAction::ResetWindowHeight)),
        "increment-width" => Some(Action::Tiler(TilerAction::IncrementWidth)),
        "decrement-width" => Some(Action::Tiler(TilerAction::DecrementWidth)),
        "toggle-fullscreen" | "fullscreen" => Some(Action::Tiler(TilerAction::ResizeToFullscreen)),
        "halfscreen" => Some(Action::Tiler(TilerAction::ResizeToHalfScreen)),
        "toggle-floating" => Some(Action::Tiler(TilerAction::ToggleWindowFloating)),
        "switch-focus-floating" => Some(Action::Tiler(TilerAction::SwitchFocusBetweenFloatingAndTiling)),
        "focus-column-first" | "first-column" => Some(Action::Tiler(TilerAction::FocusColumnFirst)),
        "focus-column-last" | "last-column" => Some(Action::Tiler(TilerAction::FocusColumnLast)),
        "move-column-to-first" => Some(Action::Tiler(TilerAction::MoveColumnToFirst)),
        "move-column-to-last" => Some(Action::Tiler(TilerAction::MoveColumnToLast)),
        "focus-window-top" | "window-top" => Some(Action::Tiler(TilerAction::FocusWindowTop)),
        "focus-window-bottom" | "window-bottom" => Some(Action::Tiler(TilerAction::FocusWindowBottom)),
        "move-window-up" => Some(Action::Tiler(TilerAction::MoveWindowUpInColumn)),
        "move-window-down" => Some(Action::Tiler(TilerAction::MoveWindowDownInColumn)),
        "expand-column-to-available-width" => Some(Action::Tiler(TilerAction::ExpandColumnToAvailableWidth)),
        "close-window" | "close" => Some(Action::Tiler(TilerAction::CloseCurrent)),
        "force-refresh" => Some(Action::Tiler(TilerAction::ForceRefresh)),
        "open-overview" | "overview" => Some(Action::Tiler(TilerAction::OpenOverview)),
        "exit" | "quit" => Some(Action::Exit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_action_parsing() {
        assert!(matches!(parse_action_name("focus-left"), Some(Action::Tiler(TilerAction::MoveFocusPrevious))));
        assert!(matches!(parse_action_name("focus-column-right"), Some(Action::Tiler(TilerAction::MoveFocusNext))));
        assert!(matches!(parse_action_name("toggle-floating"), Some(Action::Tiler(TilerAction::ToggleWindowFloating))));
        assert!(matches!(parse_action_name("toggle-fullscreen"), Some(Action::Tiler(TilerAction::ResizeToFullscreen))));
        assert!(matches!(parse_action_name("exit"), Some(Action::Exit)));
        assert!(parse_action_name("non-existent-action").is_none());
    }
}
