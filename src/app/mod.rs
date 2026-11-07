//! The root app module: it ties together every winri subsystem behind the
//! iced daemon. The flow is `subscription` (global keyboard + window hooks)
//! -> [`Message`] -> [`State::handle_app_message`] -> [`Task`]s, with the
//! current [`Mode`] deciding how input is interpreted and what is rendered.
pub mod action;
mod service;
pub mod subscription;
pub mod task;
mod view;

use std::{path::PathBuf, sync::Arc};

use anyhow::Context;
use arc_swap::ArcSwap;
use iced::{
    Color, Task,
    theme::palette::Seed,
    window::{Settings, settings::PlatformSpecific},
};
use joy_error::ResultUtilityExt;

use crate::{
    Config,
    app::{
        service::{
            overview::{self},
            tiler::{self},
        },
        subscription::global::GlobalMessage,
    },
    assert_log_fail, config,
    scroll_tiler::ScrollTiler,
    system::{self, message_box_query},
    utils::math::Size,
};

/// The whole application state. A single instance lives for the program's
/// lifetime and is mutated in place by [`State::handle_app_message`].
pub struct State {
    pub tiler: ScrollTiler,
    /// What winri is currently doing; gates input handling and rendering.
    pub mode: Mode,
    pub config: Config,
    pub config_source: Option<PathBuf>,
    /// The always-on-top, click-through window we draw the overlay onto.
    overlay_window_id: iced::window::Id,
}

/// The mutually exclusive states winri can be in. The active mode decides which
/// key bindings are live (see [`State::resolve_action`]) and what is rendered.
pub enum Mode {
    Tiler(tiler::State),
    Overview(overview::State),
    /// Transient state requesting a clean shutdown on the next message pump.
    Exit,
}

impl Default for Mode {
    fn default() -> Self {
        Self::Tiler(tiler::State::default())
    }
}

/// Every event the iced runtime can deliver to [`State::handle_app_message`].
#[derive(Debug, Clone)]
pub enum Message {
    /// A resolved, mode-aware user action (keybinding already mapped).
    Action(action::Action),

    Overview(overview::Message),

    Global(subscription::global::GlobalMessage),

    /// Restore managed windows and quit. Sent after [`Mode::Exit`] is set.
    CleanupAndExit,
}

/// Creates the full-screen, transparent, click-through overlay window used to
/// draw winri's UI (currently just the focused-window border) on top of every
/// other window without intercepting any input.
fn create_overlay_window(screen_size: Size) -> (iced::window::Id, Task<Message>) {
    let (id, task) = iced::window::open(Settings {
        decorations: false,
        transparent: true,
        resizable: false,
        closeable: false,
        level: iced::window::Level::AlwaysOnTop,
        position: iced::window::Position::Specific(iced::Point::ORIGIN),
        size: screen_size.into(),
        platform_specific: PlatformSpecific {
            skip_taskbar: true,
            ..Default::default()
        },
        ..Default::default()
    });

    (
        id,
        task.then(move |id| {
            Task::batch([
                iced::window::enable_mouse_passthrough(id),
                task::ensure_overlay_not_focused(id),
            ])
        }),
    )
}

impl State {
    /// Builds the initial state and the task that opens the overlay window.
    /// This is the `new` callback handed to `iced::daemon`.
    pub fn new() -> (Self, Task<Message>) {
        const MESSAGE: &str = r"
The config file could not be loaded.
Do you want to continue with default values ?
";
        let mut init_tasks = Vec::new();
        let (config, config_source) = match config::load() {
            Ok(config) => config,
            Err(err) => {
                log::error!("could not load config: {err:?}");
                let should_continue = message_box_query("configuration loading error", MESSAGE);
                if !should_continue {
                    log::info!(
                        "user chose to close Winri instead of continuing with default config"
                    );
                    init_tasks.push(Task::done(Message::CleanupAndExit));
                }
                (config::Root::default(), None)
            }
        };

        let config = Arc::new(ArcSwap::from_pointee(config));

        let screen_size = system::screen_size().expect("Screen size retrieval");
        let work_area = system::work_area().expect("Work area retrieval");
        let tiler = ScrollTiler::new(config.clone(), work_area, screen_size);
        let (overlay_window_id, overlay_window_creation_task) = create_overlay_window(screen_size);
        init_tasks.push(overlay_window_creation_task);

        system::ensure_taskbar_on_top();

        (
            Self {
                tiler,
                mode: Mode::default(),
                config,
                config_source,
                overlay_window_id,
            },
            Task::batch(init_tasks),
        )
    }

    /// Title for winri's overlay window. Kept empty so no caption bar text appears.
    pub fn title(_: &Self, _window_id: iced::window::Id) -> String {
        String::new()
    }

    /// Central message dispatch (the daemon's `update` callback): routes each
    /// [`Message`] to its handler and chains the resulting tasks. Also re-asserts
    /// that the overlay never holds focus, and triggers shutdown once
    /// [`Mode::Exit`] has been set.
    pub fn handle_app_message(&mut self, message: Message) -> Task<Message> {
        let mut task = Task::none();

        // HACK: By default the overlay window steals focus when created, but should not be able to be focused.
        // It causes weird behavior like keystroke not recorded until another window is focused.
        // So we refocus the desktop window after creation.
        task = task.chain(task::ensure_overlay_not_focused(self.overlay_window_id));

        match message {
            Message::Global(global_message) => {
                task = task.chain(self.handle_global_message(global_message));
            }
            Message::CleanupAndExit => {
                system::restore_windows();
                return iced::exit();
            }
            Message::Overview(message) => self
                .handle_overview_message(message)
                .handle_faillible_process()
                .discard(),
            Message::Action(action) => {
                if let Ok(action_task) = self
                    .handle_action(action)
                    .context("action handling")
                    .handle_faillible_process()
                {
                    task = task.chain(action_task);
                }
            }
        }
        if matches!(self.mode, Mode::Exit) {
            task = task.chain(Task::done(Message::CleanupAndExit));
        }
        task
    }

    /// Handles a raw hook event: maps a keystroke to an [`action::Action`] for
    /// the current mode, or refreshes the tiler when the window set changes.
    pub fn handle_global_message(&mut self, message: GlobalMessage) -> Task<Message> {
        match message {
            GlobalMessage::Key(modifiers, key) => {
                if let Some(action) = self.resolve_action(modifiers, key) {
                    return Task::done(Message::Action(action));
                }
            }
            GlobalMessage::IpcAction(action) => {
                return Task::done(Message::Action(action));
            }
            GlobalMessage::Window => {
                system::ensure_taskbar_on_top();
                self.update_tiler()
                    .context("global window event")
                    .handle_faillible_process()
                    .discard();
            }
            GlobalMessage::ConfigChanged(path) => {
                let config = config::load_from(path).unwrap_or_default();
                self.config.store(Arc::new(config));
            }
            GlobalMessage::Tick => {
                if matches!(self.mode, Mode::Tiler(_)) {
                    let _ = self.tiler.tick_animation();
                    let _ = self.update_tiler_border();
                    system::ensure_taskbar_on_top();
                }
            }
            GlobalMessage::MouseScroll { delta_x, delta_y, modifiers } => {
                if matches!(self.mode, Mode::Tiler(_)) {
                    if modifiers.contains(keyboard_types::Modifiers::SHIFT) {
                        if delta_y > 0.0 || delta_x < 0.0 {
                            self.tiler.swap_current_left();
                        } else {
                            self.tiler.swap_current_right();
                        }
                    } else if delta_y > 0.0 || delta_x < 0.0 {
                        self.tiler.focus_left();
                    } else {
                        self.tiler.focus_right();
                    }
                    let _ = self.update_tiler();
                }
            }
            GlobalMessage::MouseDragStart { button, x, y } => {
                if matches!(self.mode, Mode::Tiler(_)) {
                    self.tiler.handle_mouse_drag_start(button, x, y);
                    let _ = self.update_tiler_border();
                }
            }
            GlobalMessage::MouseDragUpdate { x, y } => {
                if matches!(self.mode, Mode::Tiler(_)) {
                    self.tiler.handle_mouse_drag_update(x, y);
                    self.tiler.apply_layout();
                    let _ = self.update_tiler_border();
                }
            }
            GlobalMessage::MouseDragEnd => {
                if matches!(self.mode, Mode::Tiler(_)) {
                    self.tiler.handle_mouse_drag_end();
                    let _ = self.update_tiler_border();
                }
            }
        }
        Task::none()
    }

    /// The daemon's `view` callback. Only the overlay window has content; every
    /// other window (e.g. thumbnail windows) renders empty.
    pub fn view(&self, window_id: iced::window::Id) -> iced::Element<'_, Message> {
        if window_id == self.overlay_window_id {
            view::overlay::view(self)
        } else {
            view::empty()
        }
    }

    /// The daemon's `theme` callback. The overlay uses a fully transparent
    /// background so only what we draw is visible over the desktop.
    pub fn theme(&self, window_id: iced::window::Id) -> iced::Theme {
        if window_id == self.overlay_window_id {
            iced::Theme::custom(
                "Overlay transparent theme",
                Seed {
                    background: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
                    ..Seed::DARK
                },
            )
        } else {
            iced::Theme::Dark // TODO: Adapt to system theme
        }
    }

    /// The daemon's `subscription` callback: starts the global keyboard and
    /// window-event hooks and streams their events back as [`Message`]s.
    pub fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::run_with(
            self.config_source.clone(),
            subscription::global::subscription,
        )
    }
}

/// Extension that logs (and in debug, panics on) an `Err` while passing the
/// `Result` through unchanged, so fallible side-tasks can be fire-and-forget
/// without silently swallowing failures.
#[easy_ext::ext(HandleFaillibleProcessResultExt)]
impl<T, E: std::fmt::Debug> Result<T, E> {
    fn handle_faillible_process(self) -> Self {
        match &self {
            Ok(_) => {}
            Err(e) => {
                assert_log_fail!("{:?}", e);
            }
        }
        self
    }
}
