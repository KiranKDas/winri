//! Global keyboard and touchpad hook.
//! Uses native Win32 low-level hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) to:
//! 1. Swallow the Win (Meta) key so winri uses it without triggering the Start Menu or OS shortcuts.
//! 2. Differentiate between physical Alt+Tab (which passes through to Windows' default window switcher)
//!    and Precision Touchpad 3-finger horizontal swipes (which inject Alt+Tab with LLKHF_INJECTED,
//!    which Winri intercepts and swallows to slide through apps).
//! 3. Support two-finger horizontal scroll when holding a modifier.

use std::{
    sync::Mutex,
    thread,
    time::Duration,
};

use iced::futures::channel::mpsc::Sender;
use joy_error::ResultUtilityExt;
use keyboard_types::Modifiers;
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT,
        LLKHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL,
        WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL,
        WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    },
};

use crate::{
    app::subscription::global::{GlobalMessage, MouseDragButton},
    system,
};

struct HookState {
    tx: Option<Sender<GlobalMessage>>,
    modifiers: Modifiers,
    alt_nav_active: bool,
    drag_active: bool,
}

static HOOK_STATE: Mutex<HookState> = Mutex::new(HookState {
    tx: None,
    modifiers: Modifiers::empty(),
    alt_nav_active: false,
    drag_active: false,
});

fn vk_to_rdev(vk: u32) -> Option<rdev::Key> {
    use rdev::Key;
    match vk {
        0x25 => Some(Key::LeftArrow),
        0x26 => Some(Key::UpArrow),
        0x27 => Some(Key::RightArrow),
        0x28 => Some(Key::DownArrow),
        0x21 => Some(Key::PageUp),
        0x22 => Some(Key::PageDown),
        0x23 => Some(Key::End),
        0x24 => Some(Key::Home),
        0x1B => Some(Key::Escape),
        0x20 => Some(Key::Space),
        0x0D => Some(Key::Return),
        0x09 => Some(Key::Tab),
        0x08 => Some(Key::Backspace),
        0x2E => Some(Key::Delete),
        0x70 => Some(Key::F1),
        0x71 => Some(Key::F2),
        0x72 => Some(Key::F3),
        0x73 => Some(Key::F4),
        0x74 => Some(Key::F5),
        0x75 => Some(Key::F6),
        0x76 => Some(Key::F7),
        0x77 => Some(Key::F8),
        0x78 => Some(Key::F9),
        0x79 => Some(Key::F10),
        0x7A => Some(Key::F11),
        0x7B => Some(Key::F12),
        0x30 => Some(Key::Num0),
        0x31 => Some(Key::Num1),
        0x32 => Some(Key::Num2),
        0x33 => Some(Key::Num3),
        0x34 => Some(Key::Num4),
        0x35 => Some(Key::Num5),
        0x36 => Some(Key::Num6),
        0x37 => Some(Key::Num7),
        0x38 => Some(Key::Num8),
        0x39 => Some(Key::Num9),
        0x41 => Some(Key::KeyA),
        0x42 => Some(Key::KeyB),
        0x43 => Some(Key::KeyC),
        0x44 => Some(Key::KeyD),
        0x45 => Some(Key::KeyE),
        0x46 => Some(Key::KeyF),
        0x47 => Some(Key::KeyG),
        0x48 => Some(Key::KeyH),
        0x49 => Some(Key::KeyI),
        0x4A => Some(Key::KeyJ),
        0x4B => Some(Key::KeyK),
        0x4C => Some(Key::KeyL),
        0x4D => Some(Key::KeyM),
        0x4E => Some(Key::KeyN),
        0x4F => Some(Key::KeyO),
        0x50 => Some(Key::KeyP),
        0x51 => Some(Key::KeyQ),
        0x52 => Some(Key::KeyR),
        0x53 => Some(Key::KeyS),
        0x54 => Some(Key::KeyT),
        0x55 => Some(Key::KeyU),
        0x56 => Some(Key::KeyV),
        0x57 => Some(Key::KeyW),
        0x58 => Some(Key::KeyX),
        0x59 => Some(Key::KeyY),
        0x5A => Some(Key::KeyZ),
        _ => None,
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let kbd = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let msg = wparam.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        let is_injected = (kbd.flags.0 & LLKHF_INJECTED.0) != 0;
        let vk = kbd.vkCode;

        let mut state = HOOK_STATE.lock().unwrap();

        // 1. Track modifier keys
        match vk {
            0xA0 | 0xA1 | 0x10 => { // VK_LSHIFT, VK_RSHIFT, VK_SHIFT
                state.modifiers.set(Modifiers::SHIFT, is_down);
                return CallNextHookEx(None, code, wparam, lparam);
            }
            0xA2 | 0xA3 | 0x11 => { // VK_LCONTROL, VK_RCONTROL, VK_CONTROL
                state.modifiers.set(Modifiers::CONTROL, is_down);
                return CallNextHookEx(None, code, wparam, lparam);
            }
            0xA4 | 0xA5 | 0x12 => { // VK_LMENU, VK_RMENU, VK_MENU (Alt)
                state.modifiers.set(Modifiers::ALT, is_down);
                if is_up && state.alt_nav_active {
                    state.alt_nav_active = false;
                    // Prevent Windows from activating the active window's menu bar (SC_KEYMENU)
                    rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::ShiftLeft)).discard();
                }
                return CallNextHookEx(None, code, wparam, lparam);
            }
            0x5B | 0x5C => { // VK_LWIN, VK_RWIN (Meta)
                state.modifiers.set(Modifiers::META, is_down);
                if is_up && state.drag_active {
                    state.drag_active = false;
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::MouseDragEnd);
                    }
                }
                // Swallow Win key presses so Start Menu and OS shortcuts do not fire
                return LRESULT(1);
            }
            _ => {}
        }

        let modifiers = state.modifiers;

        // 2. Alt + Tab handling:
        // If it's a physical keyboard press (is_injected == false):
        // Pass it directly through to Windows so the default window switching behaviour works!
        // If it's an injected event from the touchpad 3-finger horizontal swipe (is_injected == true):
        // Intercept it, swallow it from Windows, and send it to Winri to move through apps!
        if vk == 0x09 && modifiers.contains(Modifiers::ALT) { // 0x09 = VK_TAB
            if is_injected {
                if is_down {
                    state.alt_nav_active = true;
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::Key(modifiers, rdev::Key::Tab));
                    }
                }
                return LRESULT(1); // Swallow injected Tab
            } else {
                // Physical keyboard Alt+Tab: pass through to Windows default switcher
                return CallNextHookEx(None, code, wparam, lparam);
            }
        }

        // 3. Winri hotkeys (when Win / Meta is held):
        if is_down && modifiers.contains(Modifiers::META) {
            if let Some(rdev_key) = vk_to_rdev(vk) {
                if let Some(tx) = &mut state.tx {
                    let _ = tx.try_send(GlobalMessage::Key(modifiers, rdev_key));
                }
                if (modifiers, rdev_key) == (Modifiers::META, rdev::Key::KeyL) {
                    thread::spawn(|| {
                        thread::sleep(Duration::from_millis(300));
                        rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::MetaLeft)).discard();
                        rdev::simulate(&rdev::EventType::KeyRelease(rdev::Key::MetaRight)).discard();
                    });
                }
                return LRESULT(1); // Swallow Win hotkeys
            }
        }

        CallNextHookEx(None, code, wparam, lparam)
    }
}

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if code < 0 {
            return CallNextHookEx(None, code, wparam, lparam);
        }

        let msg = wparam.0 as u32;
        let mouse = &*(lparam.0 as *const MSLLHOOKSTRUCT);

        let mut state = HOOK_STATE.lock().unwrap();
        let modifiers = state.modifiers;

        match msg {
            WM_MOUSEWHEEL => {
                #[allow(clippy::cast_possible_truncation)]
                let delta = (mouse.mouseData >> 16) as i16 as f32;
                if delta != 0.0 && modifiers.contains(Modifiers::META) {
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::MouseScroll {
                            delta_x: 0.0,
                            delta_y: delta,
                            modifiers,
                        });
                    }
                    return LRESULT(1);
                }
            }
            WM_MOUSEHWHEEL => {
                #[allow(clippy::cast_possible_truncation)]
                let delta = (mouse.mouseData >> 16) as i16 as f32;
                if delta != 0.0 && (modifiers.contains(Modifiers::META) || modifiers.contains(Modifiers::ALT)) {
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::MouseScroll {
                            delta_x: delta,
                            delta_y: 0.0,
                            modifiers,
                        });
                    }
                    return LRESULT(1);
                }
            }
            WM_LBUTTONDOWN => {
                if modifiers.contains(Modifiers::META) {
                    state.drag_active = true;
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::MouseDragStart {
                            button: MouseDragButton::Left,
                            x: mouse.pt.x,
                            y: mouse.pt.y,
                        });
                    }
                    return LRESULT(1);
                }
            }
            WM_RBUTTONDOWN => {
                if modifiers.contains(Modifiers::META) {
                    state.drag_active = true;
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::MouseDragStart {
                            button: MouseDragButton::Right,
                            x: mouse.pt.x,
                            y: mouse.pt.y,
                        });
                    }
                    return LRESULT(1);
                }
            }
            WM_MOUSEMOVE => {
                if state.drag_active {
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::MouseDragUpdate {
                            x: mouse.pt.x,
                            y: mouse.pt.y,
                        });
                    }
                    return LRESULT(1);
                }
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                if state.drag_active {
                    state.drag_active = false;
                    if let Some(tx) = &mut state.tx {
                        let _ = tx.try_send(GlobalMessage::MouseDragEnd);
                    }
                    return LRESULT(1);
                }
            }
            _ => {}
        }

        CallNextHookEx(None, code, wparam, lparam)
    }
}

pub fn launch(tx: Sender<GlobalMessage>) {
    let _ = thread::Builder::new()
        .name("global-input-hook".into())
        .spawn(move || {
            {
                let mut state = HOOK_STATE.lock().unwrap();
                state.tx = Some(tx);
                state.modifiers = system::current_modifiers();
            }

            unsafe {
                let k_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0);
                let m_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), None, 0);

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                if let Ok(h) = k_hook { let _ = UnhookWindowsHookEx(h); }
                if let Ok(h) = m_hook { let _ = UnhookWindowsHookEx(h); }
            }
        });
}
