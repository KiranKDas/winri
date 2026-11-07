use iced::Task;
use keyboard_types::Modifiers;
use rdev::Key;

use crate::{
    config::Keybindings,
    app::{
        self, Mode,
        action::{Action, OverviewAction, TilerAction},
    },
};

pub mod overview;
pub mod tiler;

impl app::State {
    /// Maps a key combination to an [`Action`], given the current [`Mode`] and
    /// the user's keybinding config. Returns `None` when the combination is
    /// unbound, letting the keystroke pass through.
    pub fn resolve_action(&self, modifiers: Modifiers, key: Key) -> Option<Action> {
        let kb = &self.config.load().keybindings;
        let m = |b: &crate::config::Binding| Keybindings::matches(b, modifiers, key);

        match &self.mode {
            Mode::Tiler { .. } => {
                // Touchpad 3-finger swipe / Alt+Tab app switching
                if modifiers.contains(Modifiers::ALT) && key == Key::Tab {
                    if modifiers.contains(Modifiers::SHIFT) {
                        return Some(Action::Tiler(TilerAction::MoveFocusPrevious));
                    }
                    return Some(Action::Tiler(TilerAction::MoveFocusNext));
                }

                if m(&kb.focus_left)           { return Some(Action::Tiler(TilerAction::MoveFocusPrevious)); }
                if m(&kb.focus_right)          { return Some(Action::Tiler(TilerAction::MoveFocusNext)); }
                if m(&kb.focus_up)             { return Some(Action::Tiler(TilerAction::MoveFocusUp)); }
                if m(&kb.focus_down)           { return Some(Action::Tiler(TilerAction::MoveFocusDown)); }
                if m(&kb.focus_workspace_up)   { return Some(Action::Tiler(TilerAction::FocusWorkspaceUp)); }
                if m(&kb.focus_workspace_down) { return Some(Action::Tiler(TilerAction::FocusWorkspaceDown)); }
                if m(&kb.move_to_workspace_up) { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspaceUp)); }
                if m(&kb.move_to_workspace_down) { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspaceDown)); }
                if m(&kb.focus_workspace_1)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(0))); }
                if m(&kb.focus_workspace_2)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(1))); }
                if m(&kb.focus_workspace_3)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(2))); }
                if m(&kb.focus_workspace_4)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(3))); }
                if m(&kb.focus_workspace_5)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(4))); }
                if m(&kb.focus_workspace_6)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(5))); }
                if m(&kb.focus_workspace_7)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(6))); }
                if m(&kb.focus_workspace_8)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(7))); }
                if m(&kb.focus_workspace_9)    { return Some(Action::Tiler(TilerAction::FocusWorkspace(8))); }
                if m(&kb.move_to_workspace_1)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(0))); }
                if m(&kb.move_to_workspace_2)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(1))); }
                if m(&kb.move_to_workspace_3)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(2))); }
                if m(&kb.move_to_workspace_4)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(3))); }
                if m(&kb.move_to_workspace_5)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(4))); }
                if m(&kb.move_to_workspace_6)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(5))); }
                if m(&kb.move_to_workspace_7)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(6))); }
                if m(&kb.move_to_workspace_8)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(7))); }
                if m(&kb.move_to_workspace_9)  { return Some(Action::Tiler(TilerAction::MoveWindowToWorkspace(8))); }
                if m(&kb.move_col_to_workspace_up) { return Some(Action::Tiler(TilerAction::MoveColumnToWorkspaceUp)); }
                if m(&kb.move_col_to_workspace_down) { return Some(Action::Tiler(TilerAction::MoveColumnToWorkspaceDown)); }
                if m(&kb.move_into_col_above)  { return Some(Action::Tiler(TilerAction::MoveWindowIntoColumnAbove)); }
                if m(&kb.move_into_col_below)  { return Some(Action::Tiler(TilerAction::MoveWindowIntoColumnBelow)); }
                if m(&kb.consume_window)       { return Some(Action::Tiler(TilerAction::ConsumeWindow)); }
                if m(&kb.expel_window)         { return Some(Action::Tiler(TilerAction::ExpelWindow)); }
                if m(&kb.swap_col_left)        { return Some(Action::Tiler(TilerAction::SwapWithPrevious)); }
                if m(&kb.swap_col_right)       { return Some(Action::Tiler(TilerAction::SwapWithNext)); }
                if m(&kb.cycle_width)          { return Some(Action::Tiler(TilerAction::CycleWidth)); }
                if m(&kb.maximize_column)      { return Some(Action::Tiler(TilerAction::MaximizeColumn)); }
                if m(&kb.reset_window_height)  { return Some(Action::Tiler(TilerAction::ResetWindowHeight)); }
                if m(&kb.increment_width)      { return Some(Action::Tiler(TilerAction::IncrementWidth)); }
                if m(&kb.decrement_width)      { return Some(Action::Tiler(TilerAction::DecrementWidth)); }
                if m(&kb.close_current)        { return Some(Action::Tiler(TilerAction::CloseCurrent)); }
                if m(&kb.resize_fullscreen)    { return Some(Action::Tiler(TilerAction::ResizeToFullscreen)); }
                if m(&kb.resize_halfscreen)    { return Some(Action::Tiler(TilerAction::ResizeToHalfScreen)); }
                if m(&kb.force_refresh)        { return Some(Action::Tiler(TilerAction::ForceRefresh)); }
                if m(&kb.open_overview)        { return Some(Action::Tiler(TilerAction::OpenOverview)); }
                if m(&kb.toggle_window_floating) { return Some(Action::Tiler(TilerAction::ToggleWindowFloating)); }
                if m(&kb.switch_focus_floating) { return Some(Action::Tiler(TilerAction::SwitchFocusBetweenFloatingAndTiling)); }
                if m(&kb.focus_column_first)   { return Some(Action::Tiler(TilerAction::FocusColumnFirst)); }
                if m(&kb.focus_column_last)    { return Some(Action::Tiler(TilerAction::FocusColumnLast)); }
                if m(&kb.move_column_to_first) { return Some(Action::Tiler(TilerAction::MoveColumnToFirst)); }
                if m(&kb.move_column_to_last)  { return Some(Action::Tiler(TilerAction::MoveColumnToLast)); }
                if m(&kb.focus_window_top)     { return Some(Action::Tiler(TilerAction::FocusWindowTop)); }
                if m(&kb.focus_window_bottom)  { return Some(Action::Tiler(TilerAction::FocusWindowBottom)); }
                if m(&kb.move_window_up_in_column) { return Some(Action::Tiler(TilerAction::MoveWindowUpInColumn)); }
                if m(&kb.move_window_down_in_column) { return Some(Action::Tiler(TilerAction::MoveWindowDownInColumn)); }
                if m(&kb.expand_column_to_available_width) { return Some(Action::Tiler(TilerAction::ExpandColumnToAvailableWidth)); }
            }
            Mode::Overview { .. } => {
                if m(&kb.close_overview_down) || m(&kb.close_overview_escape) {
                    return Some(Action::Overview(OverviewAction::CloseOverview));
                }

                // Direct number selection in overview mode
                let num_pressed = match key {
                    Key::Num1 => Some(1),
                    Key::Num2 => Some(2),
                    Key::Num3 => Some(3),
                    Key::Num4 => Some(4),
                    Key::Num5 => Some(5),
                    Key::Num6 => Some(6),
                    Key::Num7 => Some(7),
                    Key::Num8 => Some(8),
                    Key::Num9 => Some(9),
                    _ => None,
                };
                if let Some(num) = num_pressed {
                    return Some(Action::Overview(OverviewAction::FocusWindow(num)));
                }
            }
            Mode::Exit => {}
        }

        // Custom spawn commands
        for spawn_binding in &kb.custom {
            if m(&spawn_binding.binding) {
                return Some(Action::Spawn(spawn_binding.spawn.clone()));
            }
        }

        // Global: Exit is available in any mode.
        if m(&kb.exit) {
            return Some(Action::Exit);
        }

        None
    }

    /// Executes a resolved [`Action`], mutating state and returning any
    /// follow-up task (e.g. opening the overview). Most tiler actions re-run
    /// [`Self::update_tiler`] so the on-screen layout matches the new model.
    pub fn handle_action(&mut self, action: Action) -> anyhow::Result<Task<app::Message>> {
        log::info!("Executing action: {action:?}");
        match action {
            Action::Tiler(tiler_action) => match tiler_action {
                TilerAction::CloseCurrent => {
                    if let Some(window) = self.tiler.focused_window() {
                        window.close()?;
                        self.update_tiler()?;
                    }
                }
                TilerAction::MoveFocusNext => {
                    self.tiler.focus_right();
                    self.update_tiler()?;
                }
                TilerAction::MoveFocusPrevious => {
                    self.tiler.focus_left();
                    self.update_tiler()?;
                }
                TilerAction::MoveFocusUp => {
                    self.tiler.focus_up();
                    self.update_tiler()?;
                }
                TilerAction::MoveFocusDown => {
                    self.tiler.focus_down();
                    self.update_tiler()?;
                }
                TilerAction::FocusWorkspaceUp => {
                    self.tiler.focus_workspace_up();
                    self.update_tiler()?;
                }
                TilerAction::FocusWorkspaceDown => {
                    self.tiler.focus_workspace_down();
                    self.update_tiler()?;
                }
                TilerAction::MoveWindowToWorkspaceUp => {
                    self.tiler.move_window_to_workspace_up();
                    self.update_tiler()?;
                }
                TilerAction::MoveWindowToWorkspaceDown => {
                    self.tiler.move_window_to_workspace_down();
                    self.update_tiler()?;
                }
                TilerAction::FocusWorkspace(idx) => {
                    self.tiler.focus_workspace_index(idx);
                    self.update_tiler()?;
                }
                TilerAction::MoveWindowToWorkspace(idx) => {
                    self.tiler.move_window_to_workspace_index(idx);
                    self.update_tiler()?;
                }
                TilerAction::MoveColumnToWorkspace(idx) => {
                    self.tiler.move_column_to_workspace_index(idx);
                    self.update_tiler()?;
                }
                TilerAction::MoveColumnToWorkspaceUp => {
                    self.tiler.move_column_to_workspace_up();
                    self.update_tiler()?;
                }
                TilerAction::MoveColumnToWorkspaceDown => {
                    self.tiler.move_column_to_workspace_down();
                    self.update_tiler()?;
                }
                TilerAction::MoveWindowIntoColumnAbove => {
                    self.tiler.move_window_into_column_above();
                    self.update_tiler()?;
                }
                TilerAction::MoveWindowIntoColumnBelow => {
                    self.tiler.move_window_into_column_below();
                    self.update_tiler()?;
                }
                TilerAction::ConsumeWindow => {
                    self.tiler.consume_window();
                    self.update_tiler()?;
                }
                TilerAction::ExpelWindow => {
                    self.tiler.expel_window();
                    self.update_tiler()?;
                }
                TilerAction::SwapWithNext => {
                    self.tiler.swap_current_right();
                    self.update_tiler()?;
                }
                TilerAction::SwapWithPrevious => {
                    self.tiler.swap_current_left();
                    self.update_tiler()?;
                }
                TilerAction::ResizeToFullscreen => {
                    self.tiler.set_current_window_fullscreen();
                    self.update_tiler()?;
                }
                TilerAction::ResizeToHalfScreen => {
                    self.tiler.set_current_window_halfscreen();
                    self.update_tiler()?;
                }
                TilerAction::MaximizeColumn => {
                    self.tiler.maximize_column();
                    self.update_tiler()?;
                }
                TilerAction::ResetWindowHeight => {
                    self.tiler.reset_window_height();
                    self.update_tiler()?;
                }
                TilerAction::CycleWidth => {
                    self.tiler.cycle_width();
                    self.update_tiler()?;
                }
                TilerAction::IncrementWidth => {
                    self.tiler.increment_current_window_width();
                    self.update_tiler()?;
                }
                TilerAction::DecrementWidth => {
                    self.tiler.decrement_current_window_width();
                    self.update_tiler()?;
                }
                TilerAction::OpenOverview => return Ok(self.prepare_open_overview()),
                TilerAction::ForceRefresh => self.update_tiler()?,
                TilerAction::ToggleWindowFloating => {
                    self.tiler.toggle_window_floating()?;
                    self.update_tiler()?;
                }
                TilerAction::SwitchFocusBetweenFloatingAndTiling => {
                    self.tiler.switch_focus_between_floating_and_tiling();
                    self.update_tiler()?;
                }
                TilerAction::FocusColumnFirst => {
                    self.tiler.focus_column_first();
                    self.update_tiler()?;
                }
                TilerAction::FocusColumnLast => {
                    self.tiler.focus_column_last();
                    self.update_tiler()?;
                }
                TilerAction::MoveColumnToFirst => {
                    self.tiler.move_column_to_first();
                    self.update_tiler()?;
                }
                TilerAction::MoveColumnToLast => {
                    self.tiler.move_column_to_last();
                    self.update_tiler()?;
                }
                TilerAction::FocusWindowTop => {
                    self.tiler.focus_window_top();
                    self.update_tiler()?;
                }
                TilerAction::FocusWindowBottom => {
                    self.tiler.focus_window_bottom();
                    self.update_tiler()?;
                }
                TilerAction::MoveWindowUpInColumn => {
                    self.tiler.move_window_up_in_column();
                    self.update_tiler()?;
                }
                TilerAction::MoveWindowDownInColumn => {
                    self.tiler.move_window_down_in_column();
                    self.update_tiler()?;
                }
                TilerAction::ExpandColumnToAvailableWidth => {
                    self.tiler.expand_column_to_available_width();
                    self.update_tiler()?;
                }
            },
            Action::Overview(overview_action) => match overview_action {
                OverviewAction::CloseOverview => return self.close_overview(),
                OverviewAction::FocusWindow(num) => return self.close_overview_and_focus(num),
            },
            Action::Spawn(cmd) => {
                log::info!("Spawning custom command: {cmd}");
                let _ = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-Command", &cmd])
                    .spawn();
            }
            Action::Exit => self.mode = Mode::Exit,
        }

        Ok(Task::none())
    }
}
