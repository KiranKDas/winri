//! The core layout engine — niri-style Ribbon -> Column -> Window Stack model.
//!
//! Windows are grouped into **columns**. Each column sits in a horizontal
//! ribbon that may be wider than the viewport; `scroll_offset_x` determines
//! which slice is visible, auto-adjusted and smoothly animated according to
//! `center_focused_column` mode.
//!
//! Within a column, windows are stacked **vertically** and share the column
//! height equally (with configurable gaps). A column with a single window
//! spans the full viewport height. Fullscreen windows are treated as a column
//! in the ribbon and can be smoothly navigated into and out of.
//!
//! Workspaces operate on the **vertical axis**: switching workspaces moves up
//! and down, preserving each workspace's independent horizontal ribbon and scroll offset.
//!
//! All geometry is applied to real windows atomically via
//! `BeginDeferWindowPos` / `DeferWindowPos` / `EndDeferWindowPos` with
//! `SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED` to eliminate visual stutter.
//!
//! In accordance with Niri rules:
//! 1. Window Stability: Windows are NEVER resized automatically during scrolling or navigation.
//!    Width is strictly constant (`Column.width`), and height is strictly
//!    determined by the vertical stack count.
//! 2. Anchor Invariance: Windows opening, closing, or resizing to the left of the
//!    active column never cause the focused window to visually jump or jitter on-screen.
//! 3. Padding between the screen and apps (`screen_padding`) and padding between
//!    the apps (`app_gap`) are distinct, configurable options in `winri.yaml`.
//! 4. Translucency is optional and controlled via `winri.yaml`.
//! 5. Smooth ribbon animations translate windows without reflowing their UI.

use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::Context;
use joy_error::log::ResultLogExt;
use log::warn;
use windows::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos,
    HWND_BOTTOM, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
};

use crate::{
    app::subscription::global::MouseDragButton,
    Config, config::{CenterFocusedColumn, ColumnWidthRule, ScreenPadding}, system,
    utils::math::{Bounds, Position, Size}, window::Window, wincall_result,
};

/// Global flag set while Winri is executing an atomic layout pass.
/// Used by WinEventHook to discard self-generated location/size events,
/// eliminating recursive event feedback loops.
pub static IS_APPLYING_LAYOUT: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// 1. Data Model
// ---------------------------------------------------------------------------

/// A single managed window item.
#[derive(Clone, PartialEq, Eq)]
pub struct WindowItem {
    pub hwnd: isize,
    pub title: String,
    pub natural_width: i32,
    pub inner: Window,
    pub current_opacity: u8,
    pub is_floating: bool,
    pub floating_rect: Option<Rect>,
}

impl WindowItem {
    pub fn new(inner: Window) -> Self {
        let title = inner
            .title()
            .ok()
            .flatten()
            .unwrap_or_default();
        let natural_width = inner
            .desktop_manager_bounds()
            .ok()
            .map(|b| b.size().width() as i32)
            .unwrap_or(0);
        let hwnd = inner.handle().0 as isize;
        Self {
            hwnd,
            title,
            natural_width,
            inner,
            current_opacity: 255,
            is_floating: false,
            floating_rect: None,
        }
    }
}

/// A vertical column of windows.
#[derive(Clone, PartialEq, Eq)]
pub struct Column {
    pub windows: Vec<WindowItem>,
    pub focused_window_idx: usize,
    pub width: i32, // Width of this column
    pub cycle_step: usize,
    pub is_fullscreen: bool,
    pub saved_width: Option<i32>,
}

impl Column {
    pub fn new(window: WindowItem, width: i32) -> Self {
        Self {
            windows: vec![window],
            focused_window_idx: 0,
            width,
            cycle_step: 0,
            is_fullscreen: false,
            saved_width: None,
        }
    }

    pub fn focused_window(&self) -> Option<&WindowItem> {
        self.windows.get(self.focused_window_idx)
    }

    pub fn clamp_focus(&mut self) {
        if self.windows.is_empty() {
            self.focused_window_idx = 0;
        } else {
            self.focused_window_idx = self.focused_window_idx.min(self.windows.len() - 1);
        }
    }
}

/// A bounding rectangle for window layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The horizontal ribbon holding columns within a workspace.
#[derive(Clone)]
pub struct Ribbon {
    pub columns: Vec<Column>,
    pub focused_col_idx: usize,
    pub viewport_width: i32,
    pub viewport_height: i32,
    pub screen_width: i32,
    pub screen_height: i32,
    pub app_gap: i32,
    pub screen_padding: ScreenPadding,
    pub scroll_offset_x: f32,
    pub target_scroll_offset_x: f32,
    pub scroll_velocity_x: f32,
}

impl Default for Ribbon {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            focused_col_idx: 0,
            viewport_width: 1920,
            viewport_height: 1080,
            screen_width: 1920,
            screen_height: 1080,
            app_gap: 10,
            screen_padding: ScreenPadding::default(),
            scroll_offset_x: 0.0,
            target_scroll_offset_x: 0.0,
            scroll_velocity_x: 0.0,
        }
    }
}

impl Ribbon {
    pub fn clamp_col_focus(&mut self) {
        if self.columns.is_empty() {
            self.focused_col_idx = 0;
        } else {
            self.focused_col_idx = self.focused_col_idx.min(self.columns.len() - 1);
        }
    }

    pub fn focused_window(&self) -> Option<&WindowItem> {
        self.columns
            .get(self.focused_col_idx)
            .and_then(Column::focused_window)
    }

    /// Logical X coordinate of column `idx` along the ribbon.
    /// Starts at `screen_padding.left` and separates columns by `app_gap`.
    pub fn column_logical_x(&self, idx: usize) -> i32 {
        let mut x = self.screen_padding.left as i32;
        for (i, col) in self.columns.iter().enumerate() {
            if i == idx {
                break;
            }
            x += col.width + self.app_gap;
        }
        x
    }

    /// Insert a column with Anchor Invariance:
    /// If inserting to the left of (or at) the focused column, adjust the scroll offset
    /// so the focused column stays at the exact same physical on-screen pixel position.
    pub fn insert_column_anchor_invariant(&mut self, idx: usize, col: Column) {
        let col_width = col.width;
        let shift = (col_width + self.app_gap) as f32;

        if !self.columns.is_empty() && idx <= self.focused_col_idx {
            self.columns.insert(idx, col);
            self.focused_col_idx += 1;
            self.scroll_offset_x += shift;
            self.target_scroll_offset_x += shift;
        } else {
            self.columns.insert(idx, col);
        }
    }

    /// Remove a column with Anchor Invariance:
    /// If removing a column strictly to the left of the focused column, shift the scroll
    /// offset left so the focused column doesn't jump.
    pub fn remove_column_anchor_invariant(&mut self, idx: usize) -> Option<Column> {
        if idx >= self.columns.len() {
            return None;
        }
        let col_width = self.columns[idx].width;
        let shift = (col_width + self.app_gap) as f32;
        let col = self.columns.remove(idx);

        if idx < self.focused_col_idx {
            self.focused_col_idx = self.focused_col_idx.saturating_sub(1);
            self.scroll_offset_x = (self.scroll_offset_x - shift).max(0.0);
            self.target_scroll_offset_x = (self.target_scroll_offset_x - shift).max(0.0);
        } else {
            self.clamp_col_focus();
        }

        Some(col)
    }

    /// Finds the index of the column that spans across physical screen X coordinate.
    pub fn column_at_screen_x(&self, screen_x: i32) -> Option<usize> {
        let logical_x = screen_x as f32 + self.scroll_offset_x;
        for (i, col) in self.columns.iter().enumerate() {
            let col_x = self.column_logical_x(i) as f32;
            let col_w = col.width as f32;
            if logical_x >= col_x && logical_x <= col_x + col_w {
                return Some(i);
            }
        }
        None
    }

    /// Keep active column visible by adjusting horizontal scroll offset (Niri model).
    pub fn adjust_scroll(&mut self, center_mode: CenterFocusedColumn) {
        if self.columns.is_empty() {
            self.target_scroll_offset_x = 0.0;
            return;
        }
        self.clamp_col_focus();
        let col_x_logical = self.column_logical_x(self.focused_col_idx);
        let col = &self.columns[self.focused_col_idx];
        let col_width = col.width;
        let pad_l = self.screen_padding.left as i32;
        let pad_r = self.screen_padding.right as i32;
        let usable_w = (self.viewport_width - pad_l - pad_r).max(200);

        if col.is_fullscreen || col_width >= usable_w {
            self.target_scroll_offset_x = (col_x_logical - pad_l).max(0) as f32;
            return;
        }

        match center_mode {
            CenterFocusedColumn::Always => {
                let center_x = col_x_logical + col_width / 2;
                let screen_center = pad_l + usable_w / 2;
                self.target_scroll_offset_x = (center_x - screen_center).max(0) as f32;
            }
            CenterFocusedColumn::OnOverflow => {
                let visible_left = self.scroll_offset_x as i32 + pad_l;
                let visible_right = self.scroll_offset_x as i32 + pad_l + usable_w;
                let col_left = col_x_logical;
                let col_right = col_x_logical + col_width;

                if col_left < visible_left {
                    // Off-screen to the left: scroll just enough to reveal it
                    self.target_scroll_offset_x = (col_left - pad_l).max(0) as f32;
                } else if col_right > visible_right {
                    // Off-screen to the right: scroll just enough to reveal it
                    self.target_scroll_offset_x = (col_right - pad_l - usable_w).max(0) as f32;
                }
            }
            CenterFocusedColumn::Never => {}
        }
    }

    /// Compute distinct `Rect { x, y, width, height }` for every window inside each column.
    /// Notice: Window widths and heights are strictly constant and are NEVER resized during scrolling.
    pub fn compute_layout(&self, origin_x: i32, origin_y: i32) -> Vec<(WindowItem, Rect)> {
        let mut result = Vec::new();

        for (ci, col) in self.columns.iter().enumerate() {
            let n = col.windows.len();
            if n == 0 {
                continue;
            }

            let is_fs = col.is_fullscreen;
            let logical_x = self.column_logical_x(ci);
            let col_x = (logical_x as f32 - self.scroll_offset_x).round() as i32;

            let _pad_l = self.screen_padding.left as i32;
            let _pad_r = self.screen_padding.right as i32;
            let pad_t = self.screen_padding.top as i32;
            let pad_b = self.screen_padding.bottom as i32;

            let (win_x, win_w) = if is_fs {
                (0, self.screen_width)
            } else {
                (origin_x + col_x, col.width)
            };

            let (base_y, col_h) = if is_fs {
                (0, self.screen_height)
            } else {
                (origin_y + pad_t, (self.viewport_height - pad_t - pad_b).max(100))
            };

            let window_height = if n == 1 || is_fs {
                col_h
            } else {
                (col_h - (n as i32 - 1) * self.app_gap) / n as i32
            };

            for (wi, item) in col.windows.iter().enumerate() {
                let win_y = base_y + wi as i32 * (window_height + self.app_gap);
                result.push((
                    item.clone(),
                    Rect {
                        x: win_x,
                        y: win_y,
                        width: win_w,
                        height: window_height,
                    },
                ));
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// 2. ScrollTiler Engine (Two-Axis: Workspaces & Ribbon)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseDragMode {
    MoveColumn {
        start_cursor_x: i32,
        current_col_idx: usize,
    },
    ResizeColumn {
        start_cursor_x: i32,
        start_width: i32,
        col_idx: usize,
    },
}

pub struct ScrollTiler {
    pub workspaces: Vec<Ribbon>,
    pub active_workspace_idx: usize,
    pub mouse_drag: Option<MouseDragMode>,
    pub floating_windows: Vec<WindowItem>,
    config: Config,
    work_area: Bounds,
    screen_size: Size,
}

impl ScrollTiler {
    pub fn new(config: Config, work_area: Bounds, screen_size: Size) -> Self {
        let app_gap = config.load().default_window.app_gap as i32;
        let screen_padding = config.load().default_window.screen_padding;
        let viewport_width = work_area.size().width() as i32;
        let viewport_height = work_area.size().height() as i32;
        let screen_width = screen_size.width() as i32;
        let screen_height = screen_size.height() as i32;

        let initial_ribbon = Ribbon {
            columns: Vec::new(),
            focused_col_idx: 0,
            viewport_width,
            viewport_height,
            screen_width,
            screen_height,
            app_gap,
            screen_padding,
            scroll_offset_x: 0.0,
            target_scroll_offset_x: 0.0,
            scroll_velocity_x: 0.0,
        };

        Self {
            workspaces: vec![initial_ribbon],
            active_workspace_idx: 0,
            mouse_drag: None,
            floating_windows: Vec::new(),
            config,
            work_area,
            screen_size,
        }
    }

    pub fn active_ribbon(&self) -> &Ribbon {
        &self.workspaces[self.active_workspace_idx]
    }

    pub fn active_ribbon_mut(&mut self) -> &mut Ribbon {
        &mut self.workspaces[self.active_workspace_idx]
    }

    fn usable_width(&self) -> i32 {
        let ribbon = self.active_ribbon();
        let pad_l = ribbon.screen_padding.left as i32;
        let pad_r = ribbon.screen_padding.right as i32;
        (ribbon.viewport_width - pad_l - pad_r).max(200)
    }

    fn default_column_width(&self) -> i32 {
        let usable = self.usable_width();
        (usable / 2 - self.active_ribbon().app_gap / 2).max(200)
    }

    fn max_column_width(&self) -> i32 {
        self.usable_width()
    }

    // -----------------------------------------------------------------------
    // Focus Navigation
    // -----------------------------------------------------------------------

    pub fn focused_window(&self) -> Option<Window> {
        if let Some(fw) = self.floating_windows.iter().find(|w| w.inner.is_focused().unwrap_or(false)) {
            return Some(fw.inner);
        }
        self.active_ribbon().focused_window().map(|item| item.inner)
    }

    pub fn focus_left(&mut self) {
        if self.active_ribbon().columns.is_empty() { return; }
        if self.active_ribbon().focused_col_idx > 0 {
            self.active_ribbon_mut().focused_col_idx -= 1;
        }
        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);
        self.focus_active_window();
    }

    pub fn focus_right(&mut self) {
        if self.active_ribbon().columns.is_empty() { return; }
        if self.active_ribbon().focused_col_idx + 1 < self.active_ribbon().columns.len() {
            self.active_ribbon_mut().focused_col_idx += 1;
        }
        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);
        self.focus_active_window();
    }

    pub fn focus_up(&mut self) {
        let ribbon = self.active_ribbon_mut();
        let should_refocus = if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            if col.focused_window_idx > 0 {
                col.focused_window_idx -= 1;
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_refocus {
            self.focus_active_window();
        }
    }

    pub fn focus_down(&mut self) {
        let ribbon = self.active_ribbon_mut();
        let should_refocus = if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            if col.focused_window_idx + 1 < col.windows.len() {
                col.focused_window_idx += 1;
                true
            } else {
                false
            }
        } else {
            false
        };
        if should_refocus {
            self.focus_active_window();
        }
    }

    pub fn focus_column_first(&mut self) {
        if self.active_ribbon().columns.is_empty() { return; }
        self.active_ribbon_mut().focused_col_idx = 0;
        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);
        self.focus_active_window();
    }

    pub fn focus_column_last(&mut self) {
        if self.active_ribbon().columns.is_empty() { return; }
        self.active_ribbon_mut().focused_col_idx = self.active_ribbon().columns.len().saturating_sub(1);
        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);
        self.focus_active_window();
    }

    pub fn focus_window_top(&mut self) {
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            col.focused_window_idx = 0;
            self.focus_active_window();
        }
    }

    pub fn focus_window_bottom(&mut self) {
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            col.focused_window_idx = col.windows.len().saturating_sub(1);
            self.focus_active_window();
        }
    }

    fn focus_active_window(&self) {
        if let Some(window) = self.focused_window() {
            let _ = window.focus()
                .context(window.get_formatted_extensive_info())
                .context("Changing tiler focused window")
                .error().log_err();
        }
    }

    // -----------------------------------------------------------------------
    // Two-Axis Workspace Model (Workspaces Up / Down)
    // -----------------------------------------------------------------------

    pub fn focus_workspace_up(&mut self) {
        if self.active_workspace_idx > 0 {
            self.active_workspace_idx -= 1;
            let center_mode = self.config.load().layout.center_focused_column;
            self.active_ribbon_mut().adjust_scroll(center_mode);
            self.apply_layout();
            self.focus_active_window();
        }
    }

    pub fn focus_workspace_down(&mut self) {
        if self.active_workspace_idx + 1 >= self.workspaces.len() {
            let app_gap = self.config.load().default_window.app_gap as i32;
            let screen_padding = self.config.load().default_window.screen_padding;
            let viewport_width = self.work_area.size().width() as i32;
            let viewport_height = self.work_area.size().height() as i32;
            let screen_width = self.screen_size.width() as i32;
            let screen_height = self.screen_size.height() as i32;

            self.workspaces.push(Ribbon {
                columns: Vec::new(),
                focused_col_idx: 0,
                viewport_width,
                viewport_height,
                screen_width,
                screen_height,
                app_gap,
                screen_padding,
                scroll_offset_x: 0.0,
                target_scroll_offset_x: 0.0,
                scroll_velocity_x: 0.0,
            });
        }
        self.active_workspace_idx += 1;
        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);
        self.apply_layout();
        self.focus_active_window();
    }

    pub fn move_window_to_workspace_up(&mut self) {
        if self.active_workspace_idx == 0 { return; }
        let target_ws = self.active_workspace_idx - 1;
        self.transfer_active_window_to_workspace(target_ws);
    }

    pub fn move_window_to_workspace_down(&mut self) {
        let target_ws = self.active_workspace_idx + 1;
        if target_ws >= self.workspaces.len() {
            let app_gap = self.config.load().default_window.app_gap as i32;
            let screen_padding = self.config.load().default_window.screen_padding;
            let viewport_width = self.work_area.size().width() as i32;
            let viewport_height = self.work_area.size().height() as i32;
            let screen_width = self.screen_size.width() as i32;
            let screen_height = self.screen_size.height() as i32;

            self.workspaces.push(Ribbon {
                columns: Vec::new(),
                focused_col_idx: 0,
                viewport_width,
                viewport_height,
                screen_width,
                screen_height,
                app_gap,
                screen_padding,
                scroll_offset_x: 0.0,
                target_scroll_offset_x: 0.0,
                scroll_velocity_x: 0.0,
            });
        }
        self.transfer_active_window_to_workspace(target_ws);
    }

    pub fn focus_workspace_index(&mut self, idx: usize) {
        self.ensure_workspace_exists(idx);
        if self.active_workspace_idx != idx {
            self.active_workspace_idx = idx;
            let center_mode = self.config.load().layout.center_focused_column;
            self.active_ribbon_mut().adjust_scroll(center_mode);
            self.apply_layout();
            self.focus_active_window();
        }
    }

    pub fn move_window_to_workspace_index(&mut self, idx: usize) {
        self.ensure_workspace_exists(idx);
        if self.active_workspace_idx != idx {
            self.transfer_active_window_to_workspace(idx);
        }
    }

    pub fn move_column_to_workspace_index(&mut self, idx: usize) {
        self.ensure_workspace_exists(idx);
        if self.active_workspace_idx != idx {
            let curr_ws = self.active_workspace_idx;
            let ribbon = &mut self.workspaces[curr_ws];
            if ribbon.columns.is_empty() { return; }
            let col_idx = ribbon.focused_col_idx;
            if let Some(col) = ribbon.remove_column_anchor_invariant(col_idx) {
                let target_ribbon = &mut self.workspaces[idx];
                target_ribbon.columns.push(col);
                target_ribbon.focused_col_idx = target_ribbon.columns.len().saturating_sub(1);
                let center_mode = self.config.load().layout.center_focused_column;
                self.active_ribbon_mut().adjust_scroll(center_mode);
                self.apply_layout();
                self.focus_active_window();
            }
        }
    }

    pub fn move_column_to_workspace_up(&mut self) {
        if self.active_workspace_idx > 0 {
            self.move_column_to_workspace_index(self.active_workspace_idx - 1);
        }
    }

    pub fn move_column_to_workspace_down(&mut self) {
        self.move_column_to_workspace_index(self.active_workspace_idx + 1);
    }

    fn transfer_active_window_to_workspace(&mut self, target_ws: usize) {
        let curr_ws = self.active_workspace_idx;
        let ribbon = &mut self.workspaces[curr_ws];
        if ribbon.columns.is_empty() { return; }

        let ci = ribbon.focused_col_idx;
        let col = &mut ribbon.columns[ci];
        if col.windows.is_empty() { return; }

        let wi = col.focused_window_idx;
        let item = col.windows.remove(wi);
        col.clamp_focus();

        let width = col.width;
        let is_fs = col.is_fullscreen;
        if col.windows.is_empty() {
            ribbon.remove_column_anchor_invariant(ci);
        }

        // Place into target workspace as a new column
        let mut new_col = Column::new(item, width);
        new_col.is_fullscreen = is_fs;
        let target_ribbon = &mut self.workspaces[target_ws];
        target_ribbon.columns.push(new_col);
        target_ribbon.focused_col_idx = target_ribbon.columns.len() - 1;

        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);
        self.apply_layout();
        self.focus_active_window();
    }

    // -----------------------------------------------------------------------
    // Column Reordering (Win + Ctrl + Left / Right)
    // -----------------------------------------------------------------------

    pub fn swap_current_left(&mut self) {
        let center_mode = self.config.load().layout.center_focused_column;
        let ribbon = self.active_ribbon_mut();
        if ribbon.focused_col_idx > 0 {
            ribbon.columns.swap(ribbon.focused_col_idx, ribbon.focused_col_idx - 1);
            ribbon.focused_col_idx -= 1;
            ribbon.adjust_scroll(center_mode);
        }
    }

    pub fn swap_current_right(&mut self) {
        let center_mode = self.config.load().layout.center_focused_column;
        let ribbon = self.active_ribbon_mut();
        if ribbon.focused_col_idx + 1 < ribbon.columns.len() {
            ribbon.columns.swap(ribbon.focused_col_idx, ribbon.focused_col_idx + 1);
            ribbon.focused_col_idx += 1;
            ribbon.adjust_scroll(center_mode);
        }
    }

    pub fn move_column_to_first(&mut self) {
        let center_mode = self.config.load().layout.center_focused_column;
        let ribbon = self.active_ribbon_mut();
        if ribbon.columns.len() <= 1 || ribbon.focused_col_idx == 0 { return; }
        let col = ribbon.columns.remove(ribbon.focused_col_idx);
        ribbon.columns.insert(0, col);
        ribbon.focused_col_idx = 0;
        ribbon.adjust_scroll(center_mode);
    }

    pub fn move_column_to_last(&mut self) {
        let center_mode = self.config.load().layout.center_focused_column;
        let ribbon = self.active_ribbon_mut();
        let last_idx = ribbon.columns.len().saturating_sub(1);
        if ribbon.columns.len() <= 1 || ribbon.focused_col_idx == last_idx { return; }
        let col = ribbon.columns.remove(ribbon.focused_col_idx);
        ribbon.columns.push(col);
        ribbon.focused_col_idx = last_idx;
        ribbon.adjust_scroll(center_mode);
    }

    pub fn move_window_up_in_column(&mut self) {
        let mut moved = false;
        {
            let ribbon = self.active_ribbon_mut();
            if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
                if col.focused_window_idx > 0 {
                    col.windows.swap(col.focused_window_idx, col.focused_window_idx - 1);
                    col.focused_window_idx -= 1;
                    moved = true;
                }
            }
        }
        if moved {
            self.focus_active_window();
        }
    }

    pub fn move_window_down_in_column(&mut self) {
        let mut moved = false;
        {
            let ribbon = self.active_ribbon_mut();
            if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
                if col.focused_window_idx + 1 < col.windows.len() {
                    col.windows.swap(col.focused_window_idx, col.focused_window_idx + 1);
                    col.focused_window_idx += 1;
                    moved = true;
                }
            }
        }
        if moved {
            self.focus_active_window();
        }
    }

    pub fn expand_column_to_available_width(&mut self) {
        let usable = self.usable_width();
        let center_mode = self.config.load().layout.center_focused_column;
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            col.is_fullscreen = false;
            col.width = usable;
        }
        ribbon.adjust_scroll(center_mode);
    }

    // -----------------------------------------------------------------------
    // Vertical Stacking & In-Column Operations
    // -----------------------------------------------------------------------

    pub fn move_window_into_column_above(&mut self) {
        let ribbon = self.active_ribbon_mut();
        if ribbon.columns.is_empty() { return; }
        let ci = ribbon.focused_col_idx;
        let col = &mut ribbon.columns[ci];
        if col.windows.is_empty() { return; }

        let wi = col.focused_window_idx;
        if col.windows.len() > 1 && wi > 0 {
            col.windows.swap(wi, wi - 1);
            col.focused_window_idx = wi - 1;
        } else if ci > 0 {
            let item = col.windows.remove(wi);
            col.clamp_focus();
            if col.windows.is_empty() {
                ribbon.remove_column_anchor_invariant(ci);
            }
            let target_ci = ci - 1;
            let target = &mut ribbon.columns[target_ci];
            target.windows.push(item);
            target.focused_window_idx = target.windows.len() - 1;
            ribbon.focused_col_idx = target_ci;
        }
    }

    pub fn move_window_into_column_below(&mut self) {
        let default_w = self.default_column_width();
        let ribbon = self.active_ribbon_mut();
        if ribbon.columns.is_empty() { return; }
        let ci = ribbon.focused_col_idx;
        let col = &mut ribbon.columns[ci];
        if col.windows.is_empty() { return; }

        let wi = col.focused_window_idx;
        if col.windows.len() > 1 && wi + 1 < col.windows.len() {
            col.windows.swap(wi, wi + 1);
            col.focused_window_idx = wi + 1;
        } else {
            let item = col.windows.remove(wi);
            col.clamp_focus();
            let had_one_window = col.windows.is_empty();
            if had_one_window {
                ribbon.remove_column_anchor_invariant(ci);
            }
            let target_ci = if had_one_window { ci } else { ci + 1 };

            if target_ci >= ribbon.columns.len() {
                let width = if item.natural_width >= 200 && item.natural_width <= ribbon.viewport_width {
                    item.natural_width
                } else {
                    default_w
                };
                ribbon.columns.push(Column::new(item, width));
                ribbon.focused_col_idx = ribbon.columns.len() - 1;
            } else {
                let target = &mut ribbon.columns[target_ci];
                target.windows.insert(0, item);
                target.focused_window_idx = 0;
                ribbon.focused_col_idx = target_ci;
            }
        }
    }

    /// Consume a window from an adjacent column into the current column stack (Niri model).
    pub fn consume_window(&mut self) {
        let ribbon = self.active_ribbon_mut();
        if ribbon.columns.is_empty() { return; }
        let ci = ribbon.focused_col_idx;

        if ci + 1 < ribbon.columns.len() {
            // Consume from right column
            let source_col = &mut ribbon.columns[ci + 1];
            if !source_col.windows.is_empty() {
                let wi = source_col.focused_window_idx;
                let item = source_col.windows.remove(wi);
                source_col.clamp_focus();
                let is_empty = source_col.windows.is_empty();
                if is_empty {
                    ribbon.remove_column_anchor_invariant(ci + 1);
                }
                let curr_col = &mut ribbon.columns[ci];
                curr_col.windows.push(item);
                curr_col.focused_window_idx = curr_col.windows.len() - 1;
            }
        } else if ci > 0 {
            // Consume from left column
            let source_col = &mut ribbon.columns[ci - 1];
            if !source_col.windows.is_empty() {
                let wi = source_col.focused_window_idx;
                let item = source_col.windows.remove(wi);
                source_col.clamp_focus();
                let is_empty = source_col.windows.is_empty();
                if is_empty {
                    ribbon.remove_column_anchor_invariant(ci - 1);
                }
                let curr_col = &mut ribbon.columns[ribbon.focused_col_idx];
                curr_col.windows.insert(0, item);
                curr_col.focused_window_idx = 0;
            }
        }
    }

    /// Expel the active window from the current column into its own new column beside it (Niri model).
    pub fn expel_window(&mut self) {
        let ribbon = self.active_ribbon_mut();
        if ribbon.columns.is_empty() { return; }
        let ci = ribbon.focused_col_idx;
        let col = &mut ribbon.columns[ci];

        if col.windows.len() > 1 {
            let wi = col.focused_window_idx;
            let item = col.windows.remove(wi);
            col.clamp_focus();
            let width = col.width;
            let new_col = Column::new(item, width);
            ribbon.insert_column_anchor_invariant(ci + 1, new_col);
            ribbon.focused_col_idx = ci + 1;
            self.focus_active_window();
        }
    }

    // -----------------------------------------------------------------------
    // Sizing & Width Presets
    // -----------------------------------------------------------------------

    pub fn toggle_current_column_fullscreen(&mut self) {
        let default_w = self.default_column_width();
        let sw = self.active_ribbon().screen_width;
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            if !col.is_fullscreen {
                col.saved_width = Some(col.width);
                col.is_fullscreen = true;
                col.width = sw;
            } else {
                col.is_fullscreen = false;
                col.width = col.saved_width.take().unwrap_or(default_w);
            }
        }
    }

    pub fn set_current_window_fullscreen(&mut self) {
        self.toggle_current_column_fullscreen();
    }

    pub fn is_current_column_fullscreen(&self) -> bool {
        self.active_ribbon()
            .columns
            .get(self.active_ribbon().focused_col_idx)
            .map(|col| col.is_fullscreen)
            .unwrap_or(false)
    }

    pub fn focus_window_by_handle(&mut self, target: Window) {
        let center_mode = self.config.load().layout.center_focused_column;
        let mut target_indices = None;
        {
            let ribbon = self.active_ribbon();
            for (ci, col) in ribbon.columns.iter().enumerate() {
                for (wi, item) in col.windows.iter().enumerate() {
                    if item.inner == target {
                        target_indices = Some((ci, wi));
                        break;
                    }
                }
                if target_indices.is_some() {
                    break;
                }
            }
        }

        if let Some((ci, wi)) = target_indices {
            let ribbon = self.active_ribbon_mut();
            ribbon.focused_col_idx = ci;
            if let Some(col) = ribbon.columns.get_mut(ci) {
                col.focused_window_idx = wi;
            }
            ribbon.adjust_scroll(center_mode);
            self.focus_active_window();
            return;
        }

        if let Some(fw) = self.floating_windows.iter().find(|w| w.inner == target) {
            let _ = fw.inner.focus();
        }
    }

    pub fn set_current_window_halfscreen(&mut self) {
        let width = self.default_column_width();
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            col.is_fullscreen = false;
            col.width = width;
        }
    }

    pub fn maximize_column(&mut self) {
        let usable = self.usable_width();
        let default_w = self.default_column_width();
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            col.is_fullscreen = false;
            if (col.width - usable).abs() <= 5 {
                col.width = col.saved_width.take().unwrap_or(default_w);
            } else {
                col.saved_width = Some(col.width);
                col.width = usable;
            }
        }
    }

    pub fn reset_window_height(&mut self) {
        self.apply_layout();
    }

    pub fn cycle_width(&mut self) {
        let usable = self.usable_width();
        let gap = self.active_ribbon().app_gap;
        let max = self.max_column_width();
        let presets = self.config.load().layout.preset_column_widths.clone();
        let presets = if presets.is_empty() { vec![0.33333, 0.5, 0.66667, 1.0] } else { presets };

        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            let total_steps = presets.len() + 1; // user presets + natural width
            let step = col.cycle_step % total_steps;
            col.cycle_step += 1;
            col.is_fullscreen = false;

            if step < presets.len() {
                let prop = presets[step];
                if (prop - 1.0).abs() < 0.001 {
                    col.width = usable;
                } else {
                    col.width = ((usable as f32 * prop) as i32 - gap).clamp(200, max);
                }
            } else {
                let natural = col.focused_window()
                    .map(|w| w.natural_width)
                    .unwrap_or(usable / 2);
                col.width = natural.clamp(200, max);
            }
        }
    }

    pub fn increment_current_window_width(&mut self) {
        let increment = self.config.load().default_window.resize_increment as i32;
        let max = self.max_column_width();
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            col.is_fullscreen = false;
            col.width = (col.width + increment).min(max);
        }
    }

    pub fn decrement_current_window_width(&mut self) {
        let increment = self.config.load().default_window.resize_increment as i32;
        let ribbon = self.active_ribbon_mut();
        if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
            col.is_fullscreen = false;
            col.width = (col.width - increment).max(200);
        }
    }

    // -----------------------------------------------------------------------
    // Interactive Mouse Drag & Resize
    // -----------------------------------------------------------------------

    pub fn handle_mouse_drag_start(&mut self, button: MouseDragButton, x: i32, _y: i32) {
        let ribbon = self.active_ribbon_mut();
        if ribbon.columns.is_empty() {
            return;
        }
        let col_idx = ribbon.column_at_screen_x(x).unwrap_or(ribbon.focused_col_idx);
        ribbon.focused_col_idx = col_idx;

        match button {
            MouseDragButton::Left => {
                self.mouse_drag = Some(MouseDragMode::MoveColumn {
                    start_cursor_x: x,
                    current_col_idx: col_idx,
                });
            }
            MouseDragButton::Right => {
                let start_width = ribbon.columns[col_idx].width;
                self.mouse_drag = Some(MouseDragMode::ResizeColumn {
                    start_cursor_x: x,
                    start_width,
                    col_idx,
                });
            }
        }
        self.focus_active_window();
    }

    pub fn handle_mouse_drag_update(&mut self, x: i32, _y: i32) {
        let Some(drag) = self.mouse_drag else { return; };
        match drag {
            MouseDragMode::ResizeColumn { start_cursor_x, start_width, col_idx } => {
                let ribbon = self.active_ribbon_mut();
                if let Some(col) = ribbon.columns.get_mut(col_idx) {
                    let dx = x - start_cursor_x;
                    let min_w = 200;
                    let max_w = (ribbon.viewport_width
                        - ribbon.screen_padding.left as i32
                        - ribbon.screen_padding.right as i32)
                        .max(min_w);
                    col.is_fullscreen = false;
                    col.width = (start_width + dx).clamp(min_w, max_w);
                }
            }
            MouseDragMode::MoveColumn { start_cursor_x, current_col_idx } => {
                let ribbon = self.active_ribbon_mut();
                let dx = x - start_cursor_x;

                if dx > 0 && current_col_idx + 1 < ribbon.columns.len() {
                    let neighbor_w = ribbon.columns[current_col_idx + 1].width;
                    if dx > neighbor_w / 2 {
                        ribbon.columns.swap(current_col_idx, current_col_idx + 1);
                        ribbon.focused_col_idx = current_col_idx + 1;
                        self.mouse_drag = Some(MouseDragMode::MoveColumn {
                            start_cursor_x: x,
                            current_col_idx: current_col_idx + 1,
                        });
                    }
                } else if dx < 0 && current_col_idx > 0 {
                    let neighbor_w = ribbon.columns[current_col_idx - 1].width;
                    if dx < -(neighbor_w / 2) {
                        ribbon.columns.swap(current_col_idx, current_col_idx - 1);
                        ribbon.focused_col_idx = current_col_idx - 1;
                        self.mouse_drag = Some(MouseDragMode::MoveColumn {
                            start_cursor_x: x,
                            current_col_idx: current_col_idx - 1,
                        });
                    }
                }
            }
        }
    }

    pub fn handle_mouse_drag_end(&mut self) {
        self.mouse_drag = None;
        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);
        self.apply_layout();
        self.focus_active_window();
    }

    // -----------------------------------------------------------------------
    // Floating Window Management (Phase 3)
    // -----------------------------------------------------------------------

    pub fn toggle_window_floating(&mut self) -> anyhow::Result<()> {
        // 1. If currently focusing a floating window, unfloat it back into the ribbon
        if let Some(pos) = self.floating_windows.iter().position(|w| {
            w.inner.is_focused().unwrap_or(false)
        }) {
            let mut item = self.floating_windows.remove(pos);
            item.is_floating = false;
            let max_w = self.max_column_width();
            let default_w = self.default_column_width();
            let center_mode = self.config.load().layout.center_focused_column;

            let width = if item.natural_width >= 200 {
                item.natural_width.min(max_w)
            } else {
                default_w
            };
            let col = Column::new(item, width);
            let ribbon = self.active_ribbon_mut();
            let insert_at = (ribbon.focused_col_idx + 1).min(ribbon.columns.len());
            ribbon.insert_column_anchor_invariant(insert_at, col);
            ribbon.focused_col_idx = insert_at;
            ribbon.adjust_scroll(center_mode);
            self.apply_layout();
            self.focus_active_window();
            return Ok(());
        }

        // 2. Otherwise, float the currently focused tiled window
        let usable_w = self.usable_width();
        let wa_pos = self.work_area.position();
        let (vp_w, vp_h) = {
            let r = self.active_ribbon();
            (r.viewport_width, r.viewport_height)
        };
        let fw = (usable_w * 2 / 3).max(500).min(vp_w);
        let fh = (vp_h * 2 / 3).max(350).min(vp_h);
        let fx = wa_pos.x() as i32 + (vp_w - fw) / 2;
        let fy = wa_pos.y() as i32 + (vp_h - fh) / 2;

        let mut removed_item = None;
        {
            let ribbon = self.active_ribbon_mut();
            if let Some(col) = ribbon.columns.get_mut(ribbon.focused_col_idx) {
                if !col.windows.is_empty() {
                    let item = col.windows.remove(col.focused_window_idx);
                    col.clamp_focus();
                    if col.windows.is_empty() {
                        let col_idx = ribbon.focused_col_idx;
                        ribbon.remove_column_anchor_invariant(col_idx);
                    }
                    removed_item = Some(item);
                }
            }
        }

        if let Some(mut item) = removed_item {
            item.is_floating = true;
            item.floating_rect = Some(Rect { x: fx, y: fy, width: fw, height: fh });

            let _ = item.inner.move_to(
                Position([fx as f32, fy as f32]),
                Size([fw as f32, fh as f32]),
            );
            let _ = item.inner.focus();

            self.floating_windows.push(item);
            self.apply_layout();
        }

        Ok(())
    }

    pub fn switch_focus_between_floating_and_tiling(&mut self) {
        let is_floating_focused = self.floating_windows.iter().any(|w| w.inner.is_focused().unwrap_or(false));
        if is_floating_focused {
            self.focus_active_window();
        } else if let Some(first_floating) = self.floating_windows.first() {
            let _ = first_floating.inner.focus();
        }
    }

    // -----------------------------------------------------------------------
    // Iterators & Animation State
    // -----------------------------------------------------------------------

    pub fn windows(&self) -> impl Iterator<Item = &WindowItem> {
        self.workspaces
            .iter()
            .flat_map(|r| r.columns.iter().flat_map(|c| c.windows.iter()))
            .chain(self.floating_windows.iter())
    }

    pub const fn work_area(&self) -> Bounds { self.work_area }

    /// Returns true if the camera ribbon is currently translating.
    /// Used to hide the highlight border during animations to eliminate jitter.
    pub fn is_animating(&self) -> bool {
        let ribbon = self.active_ribbon();
        (ribbon.scroll_offset_x - ribbon.target_scroll_offset_x).abs() > 0.5
    }

    /// Step the smooth scroll animation towards target_scroll_offset_x.
    /// Returns true if an animation frame was rendered.
    pub fn tick_animation(&mut self) -> bool {
        let anim_config = self.config.load().animations.clone();
        let ribbon = self.active_ribbon_mut();

        if !anim_config.enabled {
            if (ribbon.scroll_offset_x - ribbon.target_scroll_offset_x).abs() > 0.001 {
                ribbon.scroll_offset_x = ribbon.target_scroll_offset_x;
                ribbon.scroll_velocity_x = 0.0;
                self.apply_layout();
            }
            return false;
        }

        let diff = ribbon.target_scroll_offset_x - ribbon.scroll_offset_x;
        if diff.abs() > 0.5 {
            // Subtle, crisp ease-out with strictly zero overshoot or springiness
            let slowdown = anim_config.slowdown.unwrap_or(1.0).max(0.1);
            let factor = (0.38 / slowdown).clamp(0.05, 0.85);
            let step = diff * factor;
            ribbon.scroll_offset_x += step;
            ribbon.scroll_velocity_x = 0.0;

            if (ribbon.target_scroll_offset_x - ribbon.scroll_offset_x).abs() <= 0.5 {
                ribbon.scroll_offset_x = ribbon.target_scroll_offset_x;
            }
            self.apply_layout();
            true
        } else if (ribbon.scroll_offset_x - ribbon.target_scroll_offset_x).abs() > 0.001 {
            ribbon.scroll_offset_x = ribbon.target_scroll_offset_x;
            ribbon.scroll_velocity_x = 0.0;
            self.apply_layout();
            false
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Layout Reconciliation Pass
    // -----------------------------------------------------------------------

    pub fn ensure_workspace_exists(&mut self, target_idx: usize) {
        while self.workspaces.len() <= target_idx {
            let initial = Ribbon {
                viewport_width: self.work_area.size().width() as i32,
                viewport_height: self.work_area.size().height() as i32,
                screen_width: self.screen_size.width() as i32,
                screen_height: self.screen_size.height() as i32,
                app_gap: self.config.load().default_window.app_gap as i32,
                screen_padding: self.config.load().default_window.screen_padding,
                ..Default::default()
            };
            self.workspaces.push(initial);
        }
    }

    pub fn handle_window_snapshot(&mut self, snapshot: &HashSet<Window>) {
        if snapshot.is_empty() {
            for ribbon in &mut self.workspaces {
                ribbon.columns.clear();
                ribbon.focused_col_idx = 0;
                ribbon.scroll_offset_x = 0.0;
                ribbon.target_scroll_offset_x = 0.0;
            }
            return;
        }

        // Keep work area & viewport geometry up to date with the OS
        if let Ok(wa) = system::work_area() {
            self.work_area = wa;
            for ribbon in &mut self.workspaces {
                ribbon.viewport_width = wa.size().width() as i32;
                ribbon.viewport_height = wa.size().height() as i32;
            }
        }
        if let Ok(ss) = system::screen_size() {
            self.screen_size = ss;
            for ribbon in &mut self.workspaces {
                ribbon.screen_width = ss.width() as i32;
                ribbon.screen_height = ss.height() as i32;
            }
        }

        let app_gap = self.config.load().default_window.app_gap as i32;
        let screen_padding = self.config.load().default_window.screen_padding;
        for ribbon in &mut self.workspaces {
            ribbon.app_gap = app_gap;
            ribbon.screen_padding = screen_padding;
        }

        // 1. Remove closed windows across all workspaces and floating windows
        self.floating_windows.retain(|item| snapshot.contains(&item.inner));
        for ribbon in &mut self.workspaces {
            ribbon.columns.retain_mut(|col| {
                col.windows.retain(|item| snapshot.contains(&item.inner));
                col.clamp_focus();

                for item in &col.windows {
                    let _ = item.inner.restore_if_maximized_or_minimized();
                }

                !col.windows.is_empty()
            });
            ribbon.clamp_col_focus();
        }

        // 2. Identify new windows not in any workspace
        let present: HashSet<Window> = self.windows().map(|w| w.inner).collect();
        let new_windows: Vec<Window> = snapshot
            .iter()
            .filter(|w| !present.contains(w))
            .copied()
            .collect();

        let window_rules = self.config.load().window_rules.clone();
        let usable_w = self.usable_width();
        let max_w = self.max_column_width();
        let default_w = self.default_column_width();

        for window in new_windows {
            let _ = window.restore_if_maximized_or_minimized();
            let item = WindowItem::new(window);

            let proc = window.process_name().unwrap_or_default();
            let title = window.title().unwrap_or_default().unwrap_or_default();
            let class = window.class().unwrap_or_default();

            // Match against WindowRules
            let matched_rule = window_rules.iter().find(|r| r.matches.is_match(&proc, &title, &class));

            let mut is_fullscreen = false;
            let mut width = default_w;

            if let Some(rule) = matched_rule {
                if rule.open_fullscreen.unwrap_or(false) {
                    is_fullscreen = true;
                    width = self.active_ribbon().screen_width;
                } else if rule.open_maximized_to_column.unwrap_or(false) {
                    is_fullscreen = false;
                    width = usable_w;
                } else if let Some(width_rule) = rule.default_column_width {
                    match width_rule {
                        ColumnWidthRule::Proportion { proportion } => {
                            if (proportion - 1.0).abs() < 0.001 {
                                width = usable_w.clamp(200, max_w);
                            } else {
                                width = ((usable_w as f32 * proportion) as i32 - app_gap).clamp(200, max_w);
                            }
                        }
                        ColumnWidthRule::Fixed { fixed } => {
                            width = fixed.clamp(200, max_w);
                        }
                        ColumnWidthRule::Natural { natural } => {
                            if natural && item.natural_width >= 200 {
                                width = item.natural_width.clamp(200, max_w);
                            }
                        }
                    }
                }
                if let Some(min) = rule.min_width {
                    width = width.max(min);
                }
                if let Some(max) = rule.max_width {
                    width = width.min(max);
                }
            } else if item.natural_width >= 200 {
                width = item.natural_width.min(max_w);
            }

            let mut col = Column::new(item, width);
            col.is_fullscreen = is_fullscreen;

            // Route to configured workspace if rule specifies open_on_workspace
            let target_ws = matched_rule
                .and_then(|r| r.open_on_workspace)
                .unwrap_or(self.active_workspace_idx);
            self.ensure_workspace_exists(target_ws);

            let ribbon = &mut self.workspaces[target_ws];
            let insert_at = if ribbon.columns.is_empty() {
                0
            } else {
                (ribbon.focused_col_idx + 1).min(ribbon.columns.len())
            };

            ribbon.insert_column_anchor_invariant(insert_at, col);
            ribbon.focused_col_idx = insert_at;
        }

        self.active_ribbon_mut().clamp_col_focus();

        // 3. Sync focus from Win32 foreground window
        self.sync_focus_from_os();

        // 4. Adjust ribbon scroll according to configured centering mode
        let center_mode = self.config.load().layout.center_focused_column;
        self.active_ribbon_mut().adjust_scroll(center_mode);

        // 5. Apply layout atomically via DeferWindowPos
        self.apply_layout();
    }

    fn sync_focus_from_os(&mut self) {
        if let Ok(fw) = Window::focused() {
            for (ws_idx, ribbon) in self.workspaces.iter_mut().enumerate() {
                for (ci, col) in ribbon.columns.iter_mut().enumerate() {
                    for (wi, item) in col.windows.iter().enumerate() {
                        if item.inner == fw {
                            self.active_workspace_idx = ws_idx;
                            ribbon.focused_col_idx = ci;
                            col.focused_window_idx = wi;
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Layout calculation & atomic DeferWindowPos update across visible & off-screen windows
    pub fn apply_layout(&mut self) {
        let origin = self.work_area.position();
        let origin_x = origin.x() as i32;
        let origin_y = origin.y() as i32;

        let active_ws = self.active_workspace_idx;
        let mut active_rects = self.workspaces[active_ws].compute_layout(origin_x, origin_y);

        // Collect all inactive workspace windows (must be parked off-screen)
        let mut inactive_windows = Vec::new();
        for (i, ws) in self.workspaces.iter().enumerate() {
            if i != active_ws {
                for col in &ws.columns {
                    for item in &col.windows {
                        inactive_windows.push(item.clone());
                    }
                }
            }
        }

        let total = (active_rects.len() + inactive_windows.len()) as i32;
        if total == 0 { return; }

        let Ok(mut hdwp) = (unsafe { BeginDeferWindowPos(total) }) else {
            warn!("BeginDeferWindowPos failed, using fallback");
            self.apply_layout_fallback(&mut active_rects, &inactive_windows);
            return;
        };

        if hdwp.is_invalid() {
            warn!("BeginDeferWindowPos returned invalid handle, using fallback");
            self.apply_layout_fallback(&mut active_rects, &inactive_windows);
            return;
        }

        let config = self.config.load();
        let translucency_enabled = config.default_window.translucency.enabled;
        let inactive_opacity = config.default_window.translucency.inactive_opacity;

        IS_APPLYING_LAYOUT.store(true, Ordering::SeqCst);

        // Translucency (optional, controlled via config)
        if translucency_enabled {
            let focused_hwnd = self.focused_window().map(|w| w.handle());
            let inactive_alpha = (inactive_opacity.clamp(0.1, 1.0) * 255.0) as u8;
            for (item, _) in &mut active_rects {
                let is_active = focused_hwnd == Some(item.inner.handle());
                let target_alpha = if is_active { 255 } else { inactive_alpha };
                if item.current_opacity != target_alpha {
                    let _ = item.inner.set_opacity(if is_active { 1.0 } else { inactive_opacity });
                    item.current_opacity = target_alpha;
                }
            }
        }

        // Park inactive workspace windows off-screen without altering their size
        for item in &inactive_windows {
            let defer_res = unsafe {
                DeferWindowPos(
                    hdwp,
                    item.inner.handle(),
                    Some(HWND_BOTTOM),
                    -10000, -10000, 0, 0,
                    SWP_NOACTIVATE | SWP_NOSIZE,
                )
            };
            if let Ok(new_hdwp) = defer_res {
                if !new_hdwp.is_invalid() {
                    hdwp = new_hdwp;
                }
            }
        }

        let focused_hwnd = self.focused_window().map(|w| w.handle());
        let taskbar = system::taskbar_windows().first().copied();

        // Apply active workspace layout continuously along the ribbon.
        // Windows maintain their exact position and are never resized off-screen.
        // Side/inactive windows are placed behind the taskbar so they never overlap it.
        for (item, rect) in &mut active_rects {
            let _ = item.inner.restore_if_maximized_or_minimized();

            let is_focused = focused_hwnd == Some(item.inner.handle());
            let is_fs = self.is_current_column_fullscreen() && is_focused;

            let [pad_l, pad_t, pad_r, pad_b] = if is_fs {
                [0.0, 0.0, 0.0, 0.0]
            } else {
                item.inner.padding().unwrap_or([0.0; 4])
            };

            let x = rect.x - pad_l as i32;
            let y = rect.y - pad_t as i32;
            let w = rect.width + (pad_l + pad_r) as i32;
            let h = rect.height + (pad_t + pad_b) as i32;

            let (insert_after, mut flags) = if is_fs {
                (Some(HWND_TOPMOST), SWP_NOACTIVATE)
            } else if is_focused {
                (None, SWP_NOZORDER | SWP_NOACTIVATE)
            } else if let Some(tb) = taskbar {
                (Some(tb), SWP_NOACTIVATE)
            } else {
                (Some(HWND_BOTTOM), SWP_NOACTIVATE)
            };

            if let Ok(b) = item.inner.outer_bounds() {
                if is_focused
                    && (b.left as i32 - x).abs() <= 1
                    && (b.top as i32 - y).abs() <= 1
                    && (b.size().width() as i32 - w).abs() <= 1
                    && (b.size().height() as i32 - h).abs() <= 1
                {
                    continue;
                }
                if (b.size().width() as i32 - w).abs() <= 1 && (b.size().height() as i32 - h).abs() <= 1 {
                    flags |= SWP_NOSIZE;
                }
                if (b.left as i32 - x).abs() <= 1 && (b.top as i32 - y).abs() <= 1 {
                    flags |= SWP_NOMOVE;
                }
            }

            let defer_res = unsafe {
                DeferWindowPos(
                    hdwp,
                    item.inner.handle(),
                    insert_after,
                    x, y, w, h,
                    flags,
                )
            };
            if let Ok(new_hdwp) = defer_res {
                if !new_hdwp.is_invalid() {
                    hdwp = new_hdwp;
                }
            }
        }

        if let Err(e) = wincall_result!(EndDeferWindowPos(hdwp)).context("EndDeferWindowPos") {
            warn!("EndDeferWindowPos failed: {e:?}");
        }

        if !self.is_current_column_fullscreen() {
            system::ensure_taskbar_on_top();
        }

        IS_APPLYING_LAYOUT.store(false, Ordering::SeqCst);
    }

    fn apply_layout_fallback(&mut self, active_rects: &mut [(WindowItem, Rect)], inactive_windows: &[WindowItem]) {
        IS_APPLYING_LAYOUT.store(true, Ordering::SeqCst);

        for item in inactive_windows {
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    item.inner.handle(),
                    Some(HWND_BOTTOM),
                    -10000, -10000, 0, 0,
                    SWP_NOACTIVATE | SWP_NOSIZE,
                )
            };
        }

        let focused_hwnd = self.focused_window().map(|w| w.handle());
        let taskbar = system::taskbar_windows().first().copied();

        for (item, rect) in active_rects {
            let is_focused = focused_hwnd == Some(item.inner.handle());
            let is_fs = self.is_current_column_fullscreen() && is_focused;

            let [pad_l, pad_t, pad_r, pad_b] = if is_fs {
                [0.0, 0.0, 0.0, 0.0]
            } else {
                item.inner.padding().unwrap_or([0.0; 4])
            };

            let x = rect.x - pad_l as i32;
            let y = rect.y - pad_t as i32;
            let w = rect.width + (pad_l + pad_r) as i32;
            let h = rect.height + (pad_t + pad_b) as i32;

            let (insert_after, mut flags) = if is_fs {
                (Some(HWND_TOPMOST), SWP_NOACTIVATE)
            } else if is_focused {
                (None, SWP_NOZORDER | SWP_NOACTIVATE)
            } else if let Some(tb) = taskbar {
                (Some(tb), SWP_NOACTIVATE)
            } else {
                (Some(HWND_BOTTOM), SWP_NOACTIVATE)
            };

            if let Ok(b) = item.inner.outer_bounds() {
                if (b.size().width() as i32 - w).abs() <= 1 && (b.size().height() as i32 - h).abs() <= 1 {
                    flags |= SWP_NOSIZE;
                }
                if (b.left as i32 - x).abs() <= 1 && (b.top as i32 - y).abs() <= 1 {
                    flags |= SWP_NOMOVE;
                }
            }

            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    item.inner.handle(),
                    insert_after,
                    x, y, w, h,
                    flags,
                )
            };
        }

        if !self.is_current_column_fullscreen() {
            system::ensure_taskbar_on_top();
        }

        IS_APPLYING_LAYOUT.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_window_item(hwnd: isize, natural_width: i32) -> WindowItem {
        WindowItem {
            hwnd,
            title: format!("Window {hwnd}"),
            natural_width,
            inner: Window { hwnd: hwnd as u64 },
            current_opacity: 255,
            is_floating: false,
            floating_rect: None,
        }
    }

    #[test]
    fn test_column_logical_x_calculation() {
        let ribbon = Ribbon {
            columns: vec![
                Column::new(dummy_window_item(1, 500), 500),
                Column::new(dummy_window_item(2, 600), 600),
                Column::new(dummy_window_item(3, 700), 700),
            ],
            app_gap: 15,
            screen_padding: ScreenPadding {
                left: 20.0,
                right: 20.0,
                top: 10.0,
                bottom: 10.0,
            },
            viewport_width: 1920,
            viewport_height: 1080,
            ..Default::default()
        };

        assert_eq!(ribbon.column_logical_x(0), 20);
        assert_eq!(ribbon.column_logical_x(1), 20 + 500 + 15); // 535
        assert_eq!(ribbon.column_logical_x(2), 535 + 600 + 15); // 1150
    }

    #[test]
    fn test_niri_column_centering_scroll() {
        let mut ribbon = Ribbon {
            columns: vec![
                Column::new(dummy_window_item(1, 900), 900),
                Column::new(dummy_window_item(2, 900), 900),
                Column::new(dummy_window_item(3, 900), 900),
            ],
            focused_col_idx: 1,
            app_gap: 20,
            screen_padding: ScreenPadding {
                left: 30.0,
                right: 30.0,
                top: 20.0,
                bottom: 20.0,
            },
            viewport_width: 1920,
            viewport_height: 1080,
            ..Default::default()
        };

        ribbon.adjust_scroll(CenterFocusedColumn::Always);
        assert_eq!(ribbon.target_scroll_offset_x, 440.0);
    }

    #[test]
    fn test_niri_on_overflow_scroll() {
        let mut ribbon = Ribbon {
            columns: vec![
                Column::new(dummy_window_item(1, 800), 800),
                Column::new(dummy_window_item(2, 800), 800),
                Column::new(dummy_window_item(3, 800), 800),
            ],
            focused_col_idx: 0,
            app_gap: 10,
            screen_padding: ScreenPadding {
                left: 10.0,
                right: 10.0,
                top: 10.0,
                bottom: 10.0,
            },
            viewport_width: 1920,
            viewport_height: 1080,
            scroll_offset_x: 0.0,
            target_scroll_offset_x: 0.0,
            ..Default::default()
        };

        // Col 0 is at logical X: 10..810. Fits inside 1920. Target stays 0.
        ribbon.adjust_scroll(CenterFocusedColumn::OnOverflow);
        assert_eq!(ribbon.target_scroll_offset_x, 0.0);

        // Move focus to Col 2 (logical X = 10 + 800 + 10 + 800 + 10 = 1630, width = 800, right edge = 2430)
        ribbon.focused_col_idx = 2;
        ribbon.adjust_scroll(CenterFocusedColumn::OnOverflow);
        // Usable width = 1920 - 20 = 1900. Right edge 2430 > visible right (1910).
        // Target scrolls right just enough: 2430 - 10 - 1900 = 520.
        assert_eq!(ribbon.target_scroll_offset_x, 520.0);
    }

    #[test]
    fn test_anchor_invariance_on_insert_and_remove() {
        let mut ribbon = Ribbon {
            columns: vec![
                Column::new(dummy_window_item(1, 600), 600),
                Column::new(dummy_window_item(2, 600), 600),
            ],
            focused_col_idx: 1,
            app_gap: 10,
            screen_padding: ScreenPadding { left: 10.0, right: 10.0, top: 10.0, bottom: 10.0 },
            viewport_width: 1920,
            viewport_height: 1080,
            scroll_offset_x: 100.0,
            target_scroll_offset_x: 100.0,
            ..Default::default()
        };

        // Screen position of focused column (col 1):
        // logical X = 10 + 600 + 10 = 620
        // screen X = logical X - scroll_offset_x = 620 - 100 = 520.
        let orig_screen_x = ribbon.column_logical_x(ribbon.focused_col_idx) as f32 - ribbon.scroll_offset_x;
        assert_eq!(orig_screen_x, 520.0);

        // Insert a new column of width 400 at index 0 (to the left of focus)
        ribbon.insert_column_anchor_invariant(0, Column::new(dummy_window_item(99, 400), 400));
        assert_eq!(ribbon.focused_col_idx, 2); // shifted to index 2

        // Check new screen position: MUST be exactly 520.0 (Anchor Invariance!)
        let new_screen_x = ribbon.column_logical_x(ribbon.focused_col_idx) as f32 - ribbon.scroll_offset_x;
        assert_eq!(new_screen_x, 520.0);

        // Remove the column at index 0:
        ribbon.remove_column_anchor_invariant(0);
        assert_eq!(ribbon.focused_col_idx, 1);

        // Screen position MUST STILL be exactly 520.0
        let after_remove_screen_x = ribbon.column_logical_x(ribbon.focused_col_idx) as f32 - ribbon.scroll_offset_x;
        assert_eq!(after_remove_screen_x, 520.0);
    }

    #[test]
    fn test_vertical_stacking_heights_and_padding() {
        let ribbon = Ribbon {
            columns: vec![
                Column {
                    windows: vec![
                        dummy_window_item(1, 800),
                        dummy_window_item(2, 800),
                    ],
                    focused_window_idx: 0,
                    width: 800,
                    cycle_step: 0,
                    is_fullscreen: false,
                    saved_width: None,
                },
            ],
            app_gap: 12,
            screen_padding: ScreenPadding {
                left: 16.0,
                right: 16.0,
                top: 24.0,
                bottom: 24.0,
            },
            viewport_width: 1920,
            viewport_height: 1000,
            ..Default::default()
        };

        let rects = ribbon.compute_layout(0, 0);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].1.height, 470);
        assert_eq!(rects[1].1.height, 470);
        assert_eq!(rects[0].1.y, 24);
        assert_eq!(rects[1].1.y, 24 + 470 + 12);
        assert_eq!(rects[1].1.y + rects[1].1.height, 1000 - 24);
    }

    #[test]
    fn test_window_never_resized_during_scrolling() {
        let mut ribbon = Ribbon {
            columns: vec![
                Column::new(dummy_window_item(1, 600), 600),
                Column::new(dummy_window_item(2, 800), 800),
                Column::new(dummy_window_item(3, 1000), 1000),
            ],
            app_gap: 10,
            screen_padding: ScreenPadding::default(),
            viewport_width: 1920,
            viewport_height: 1080,
            scroll_offset_x: 0.0,
            ..Default::default()
        };

        let rects_0 = ribbon.compute_layout(0, 0);
        assert_eq!(rects_0[0].1.width, 600);
        assert_eq!(rects_0[1].1.width, 800);
        assert_eq!(rects_0[2].1.width, 1000);

        ribbon.scroll_offset_x = 450.0;
        let rects_scrolled = ribbon.compute_layout(0, 0);
        assert_eq!(rects_scrolled[0].1.width, 600);
        assert_eq!(rects_scrolled[1].1.width, 800);
        assert_eq!(rects_scrolled[2].1.width, 1000);
        assert_eq!(rects_scrolled[0].1.height, rects_0[0].1.height);
        assert_eq!(rects_scrolled[1].1.height, rects_0[1].1.height);
        assert_eq!(rects_scrolled[2].1.height, rects_0[2].1.height);
    }

    #[test]
    fn test_two_axis_workspace_navigation_and_window_moving() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );

        // Add 2 columns to Workspace 0
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(10, 800), 800));
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(20, 800), 800));
        tiler.workspaces[0].focused_col_idx = 1;

        assert_eq!(tiler.active_workspace_idx, 0);
        assert_eq!(tiler.active_ribbon().columns.len(), 2);

        // Move focused window (20) to workspace down
        tiler.move_window_to_workspace_down();

        // Workspace 0 should now have only 1 column
        assert_eq!(tiler.workspaces[0].columns.len(), 1);
        assert_eq!(tiler.workspaces[0].columns[0].windows[0].hwnd, 10);

        // Workspace 1 should have been created and contain window 20
        assert_eq!(tiler.workspaces.len(), 2);
        assert_eq!(tiler.workspaces[1].columns.len(), 1);
        assert_eq!(tiler.workspaces[1].columns[0].windows[0].hwnd, 20);

        // Focus workspace down
        tiler.focus_workspace_down();
        assert_eq!(tiler.active_workspace_idx, 1);
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 20);

        // Focus workspace up
        tiler.focus_workspace_up();
        assert_eq!(tiler.active_workspace_idx, 0);
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 10);
    }

    #[test]
    fn test_consume_and_expel_window() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );

        // Column 0 has window 1, Column 1 has window 2
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(1, 800), 800));
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(2, 800), 800));
        tiler.workspaces[0].focused_col_idx = 0;

        // Consume window from Column 1 into Column 0
        tiler.consume_window();

        // Column 0 should now have 2 windows, and Column 1 is removed
        assert_eq!(tiler.active_ribbon().columns.len(), 1);
        assert_eq!(tiler.active_ribbon().columns[0].windows.len(), 2);
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 1);
        assert_eq!(tiler.active_ribbon().columns[0].windows[1].hwnd, 2);

        // Expel window from Column 0
        tiler.expel_window();

        // Should now have 2 columns again
        assert_eq!(tiler.active_ribbon().columns.len(), 2);
        assert_eq!(tiler.active_ribbon().columns[0].windows.len(), 1);
        assert_eq!(tiler.active_ribbon().columns[1].windows.len(), 1);
    }

    #[test]
    fn test_fullscreen_preserves_original_column_width() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );

        // Column with custom width of 750px
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(1, 750), 750));
        tiler.workspaces[0].focused_col_idx = 0;

        assert_eq!(tiler.active_ribbon().columns[0].width, 750);
        assert!(!tiler.active_ribbon().columns[0].is_fullscreen);

        // Toggle fullscreen on
        tiler.toggle_current_column_fullscreen();
        assert!(tiler.active_ribbon().columns[0].is_fullscreen);
        assert_eq!(tiler.active_ribbon().columns[0].width, 1920);

        // Toggle fullscreen off -> MUST restore exact previous width 750px, not halved!
        tiler.toggle_current_column_fullscreen();
        assert!(!tiler.active_ribbon().columns[0].is_fullscreen);
        assert_eq!(tiler.active_ribbon().columns[0].width, 750);
    }

    #[test]
    fn test_fullscreen_different_from_ratio_1() {
        let ribbon = Ribbon {
            columns: vec![
                Column::new(dummy_window_item(1, 1880), 1880), // 1.0 ratio
            ],
            app_gap: 10,
            screen_padding: ScreenPadding {
                left: 20.0,
                right: 20.0,
                top: 15.0,
                bottom: 15.0,
            },
            viewport_width: 1920,
            viewport_height: 1040,
            screen_width: 1920,
            screen_height: 1080,
            ..Default::default()
        };

        // 1.0 ratio layout: stays within work area above taskbar, has screen padding
        let rects_ratio1 = ribbon.compute_layout(0, 0);
        assert_eq!(rects_ratio1[0].1.x, 20);
        assert_eq!(rects_ratio1[0].1.y, 15);
        assert_eq!(rects_ratio1[0].1.width, 1880);
        assert_eq!(rects_ratio1[0].1.height, 1040 - 30); // 1010

        // Fullscreen layout: fills full monitor screen, extends behind taskbar, no padding
        let mut fs_ribbon = ribbon.clone();
        fs_ribbon.columns[0].is_fullscreen = true;
        fs_ribbon.columns[0].width = 1920;
        let rects_fs = fs_ribbon.compute_layout(0, 0);
        assert_eq!(rects_fs[0].1.x, 0);
        assert_eq!(rects_fs[0].1.y, 0);
        assert_eq!(rects_fs[0].1.width, 1920);
        assert_eq!(rects_fs[0].1.height, 1080);

        assert_ne!(rects_ratio1[0].1, rects_fs[0].1);
    }

    #[test]
    fn test_cycle_width_has_ratio_1_and_not_fullscreen() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(1, 600), 600));

        let usable = tiler.usable_width();

        // Step 0: ~0.33
        tiler.cycle_width();
        assert!(!tiler.active_ribbon().columns[0].is_fullscreen);

        // Step 1: ~0.50
        tiler.cycle_width();
        assert!(!tiler.active_ribbon().columns[0].is_fullscreen);

        // Step 2: ~0.66
        tiler.cycle_width();
        assert!(!tiler.active_ribbon().columns[0].is_fullscreen);

        // Step 3: 1.0 ratio (MUST BE usable width and NOT fullscreen)
        tiler.cycle_width();
        assert!(!tiler.active_ribbon().columns[0].is_fullscreen);
        assert_eq!(tiler.active_ribbon().columns[0].width, usable);

        // Step 4: natural
        tiler.cycle_width();
        assert!(!tiler.active_ribbon().columns[0].is_fullscreen);
    }

    #[test]
    fn test_mouse_drag_move_and_resize() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(1, 600), 600));
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(2, 600), 600));

        // Test column detection at screen X
        assert_eq!(tiler.active_ribbon().column_at_screen_x(100), Some(0));
        assert_eq!(tiler.active_ribbon().column_at_screen_x(700), Some(1));

        // Test interactive column resize
        tiler.handle_mouse_drag_start(MouseDragButton::Right, 100, 100);
        tiler.handle_mouse_drag_update(250, 100); // +150px
        assert_eq!(tiler.active_ribbon().columns[0].width, 750);
        tiler.handle_mouse_drag_end();
        assert!(tiler.mouse_drag.is_none());

        // Test interactive column move/swap
        tiler.handle_mouse_drag_start(MouseDragButton::Left, 100, 100);
        tiler.handle_mouse_drag_update(500, 100); // drag past col 1 center
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 2);
        assert_eq!(tiler.active_ribbon().columns[1].windows[0].hwnd, 1);
        tiler.handle_mouse_drag_end();
        assert!(tiler.mouse_drag.is_none());
    }

    #[test]
    fn test_floating_window_toggle() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(1, 600), 600));
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(2, 600), 600));

        assert_eq!(tiler.active_ribbon().columns.len(), 2);
        assert_eq!(tiler.floating_windows.len(), 0);

        // Toggle window 1 to floating
        tiler.toggle_window_floating().unwrap();
        assert_eq!(tiler.floating_windows.len(), 1);
        assert!(tiler.floating_windows[0].is_floating);
        assert!(tiler.floating_windows[0].floating_rect.is_some());
        assert_eq!(tiler.active_ribbon().columns.len(), 1);
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 2);
    }

    #[test]
    fn test_phase1_navigation_and_column_controls() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(1, 600), 600));
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(2, 600), 600));
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(3, 600), 600));

        // Focus first & last column
        tiler.focus_column_last();
        assert_eq!(tiler.active_ribbon().focused_col_idx, 2);
        tiler.focus_column_first();
        assert_eq!(tiler.active_ribbon().focused_col_idx, 0);

        // Move column 0 to last
        tiler.move_column_to_last();
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 2);
        assert_eq!(tiler.active_ribbon().columns[2].windows[0].hwnd, 1);
        assert_eq!(tiler.active_ribbon().focused_col_idx, 2);

        // Move column 2 to first
        tiler.move_column_to_first();
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 1);
        assert_eq!(tiler.active_ribbon().focused_col_idx, 0);

        // Expand column to available width
        let usable = tiler.usable_width();
        tiler.expand_column_to_available_width();
        assert_eq!(tiler.active_ribbon().columns[0].width, usable);

        // Test in-column window reordering
        tiler.workspaces[0].columns[0].windows.push(dummy_window_item(99, 600));
        assert_eq!(tiler.active_ribbon().columns[0].windows.len(), 2);
        tiler.move_window_down_in_column();
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 99);
        assert_eq!(tiler.active_ribbon().columns[0].windows[1].hwnd, 1);
        tiler.move_window_up_in_column();
        assert_eq!(tiler.active_ribbon().columns[0].windows[0].hwnd, 1);
        assert_eq!(tiler.active_ribbon().columns[0].windows[1].hwnd, 99);
    }

    #[test]
    fn test_phase2_workspace_indexing_and_column_transfer() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(1, 600), 600));
        tiler.workspaces[0].columns.push(Column::new(dummy_window_item(2, 600), 600));

        // Jump to workspace 3 (index 2) directly
        tiler.focus_workspace_index(2);
        assert_eq!(tiler.active_workspace_idx, 2);
        assert_eq!(tiler.workspaces.len(), 3);

        // Jump back to workspace 1 (index 0)
        tiler.focus_workspace_index(0);
        assert_eq!(tiler.active_workspace_idx, 0);

        // Move active column (column 0 with window 1) to workspace 3 (index 2)
        tiler.move_column_to_workspace_index(2);
        assert_eq!(tiler.workspaces[0].columns.len(), 1);
        assert_eq!(tiler.workspaces[0].columns[0].windows[0].hwnd, 2);
        assert_eq!(tiler.workspaces[2].columns.len(), 1);
        assert_eq!(tiler.workspaces[2].columns[0].windows[0].hwnd, 1);
    }

    #[test]
    fn test_subtle_animation_no_overshoot() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );

        tiler.active_ribbon_mut().scroll_offset_x = 0.0;
        tiler.active_ribbon_mut().target_scroll_offset_x = 200.0;

        let mut prev_offset = 0.0;
        while tiler.tick_animation() {
            let curr = tiler.active_ribbon().scroll_offset_x;
            // Must strictly advance toward target without overshooting 200.0
            assert!(curr >= prev_offset);
            assert!(curr <= 200.0);
            prev_offset = curr;
        }

        assert_eq!(tiler.active_ribbon().scroll_offset_x, 200.0);
    }

    #[test]
    fn test_focus_window_by_handle() {
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let config = Arc::new(ArcSwap::from_pointee(crate::config::Root::default()));
        let mut tiler = ScrollTiler::new(
            config,
            Bounds::new(0.0, 0.0, 1920.0, 1040.0),
            Size([1920.0, 1080.0]),
        );

        let win1 = dummy_window_item(100, 600);
        let win2 = dummy_window_item(200, 600);
        let win3 = dummy_window_item(300, 600);

        tiler.workspaces[0].columns.push(Column::new(win1.clone(), 600));
        tiler.workspaces[0].columns.push(Column::new(win2.clone(), 600));
        tiler.workspaces[0].columns[0].windows.push(win3.clone());

        // Focus window 200 (in column 1)
        tiler.focus_window_by_handle(win2.inner);
        assert_eq!(tiler.active_ribbon().focused_col_idx, 1);

        // Focus window 300 (second window in column 0)
        tiler.focus_window_by_handle(win3.inner);
        assert_eq!(tiler.active_ribbon().focused_col_idx, 0);
        assert_eq!(tiler.active_ribbon().columns[0].focused_window_idx, 1);
    }
}
