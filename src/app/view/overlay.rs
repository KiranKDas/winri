use iced::{
    Color, Renderer, Theme, mouse,
    widget::{
        self,
        canvas::{self, Path, Stroke},
    },
};

use crate::{
    adapter::iced::CssColorExt,
    app::{self, service::tiler::State, view},
    utils::math::Bounds,
};

/// Renders the overlay's contents for the current mode.
pub fn view(app: &app::State) -> iced::Element<'_, app::Message> {
    match &app.mode {
        app::Mode::Tiler(tiler_state) => tiler_view(app, tiler_state),
        app::Mode::Overview(overview_state) => overview_view(app, overview_state),
        _ => view::empty(),
    }
}

fn tiler_view<'a>(app: &'a app::State, tiler_state: &'a State) -> iced::Element<'a, app::Message> {
    // If the active window is fullscreen, hide all overlay elements cleanly
    if app.tiler.is_current_column_fullscreen() {
        return view::empty();
    }

    let config = app.config.load();
    let ribbon = app.tiler.active_ribbon();

    // Stack depth indicator info (total windows in active column, active index)
    let stack_info = if let Some(col) = ribbon.columns.get(ribbon.focused_col_idx) {
        if col.windows.len() > 1 {
            Some((col.windows.len(), col.focused_window_idx))
        } else {
            None
        }
    } else {
        None
    };

    // Ribbon horizontal scroll mini-map / progress indicator
    let scroll_info = if !ribbon.columns.is_empty() {
        let last_idx = ribbon.columns.len() - 1;
        let total_logical_w = (ribbon.column_logical_x(last_idx) + ribbon.columns[last_idx].width + ribbon.screen_padding.right as i32) as f32;
        let vp_w = ribbon.viewport_width as f32;
        if total_logical_w > vp_w + 10.0 {
            let max_scroll = (total_logical_w - vp_w).max(1.0);
            let progress = (ribbon.scroll_offset_x / max_scroll).clamp(0.0, 1.0);
            let visible_ratio = (vp_w / total_logical_w).clamp(0.12, 1.0);
            Some((progress, visible_ratio))
        } else {
            None
        }
    } else {
        None
    };

    if tiler_state.current_border_bounds.is_none() && scroll_info.is_none() {
        return view::empty();
    }

    widget::canvas(TilerOverlayCanvas {
        border_bounds: tiler_state.current_border_bounds,
        border_thickness: config.default_window.border_thickness,
        border_color: config.default_window.border_color.to_iced(),
        border_radius: config.default_window.border_radius,
        stack_info,
        scroll_info,
        viewport_width: ribbon.viewport_width as f32,
        viewport_height: ribbon.viewport_height as f32,
    })
    .width(iced::Length::Fill)
    .height(iced::Length::Fill)
    .into()
}

struct TilerOverlayCanvas {
    border_bounds: Option<Bounds>,
    border_thickness: f32,
    border_color: iced::Color,
    border_radius: f32,
    stack_info: Option<(usize, usize)>,
    scroll_info: Option<(f32, f32)>,
    viewport_width: f32,
    viewport_height: f32,
}

impl canvas::Program<app::Message> for TilerOverlayCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // 1. Draw Active Window Highlight Border
        if let Some(border_bounds) = self.border_bounds {
            let half_t = self.border_thickness / 2.0;
            let pos = border_bounds.position();
            let size = border_bounds.size();

            let inset_pos = iced::Point::new(pos.x() + half_t, pos.y() + half_t);
            let inset_size = iced::Size::new(
                (size.width() - self.border_thickness).max(0.0),
                (size.height() - self.border_thickness).max(0.0),
            );
            let radius = (self.border_radius - half_t).max(0.0);

            let path = Path::rounded_rectangle(inset_pos, inset_size, radius.into());
            frame.stroke(
                &path,
                Stroke::default()
                    .with_color(self.border_color)
                    .with_width(self.border_thickness),
            );

            // 2. Draw Column Stack Depth Indicator (dots alongside the window)
            if let Some((total_windows, active_idx)) = self.stack_info {
                let dot_radius = 3.5;
                let dot_spacing = 14.0;
                let stack_height = (total_windows as f32) * dot_spacing;
                let start_y = pos.y() + (size.height() - stack_height) / 2.0 + dot_radius;
                let dot_x = pos.x() + size.width() - 12.0;

                for i in 0..total_windows {
                    let dot_y = start_y + (i as f32) * dot_spacing;
                    let dot_color = if i == active_idx {
                        self.border_color
                    } else {
                        Color::from_rgba(1.0, 1.0, 1.0, 0.35)
                    };
                    let circle = Path::circle(iced::Point::new(dot_x, dot_y), dot_radius);
                    frame.fill(&circle, dot_color);
                }
            }
        }

        // 3. Draw Ribbon Scroll Position Indicator (Mini-Map Pill at bottom center)
        if let Some((progress, visible_ratio)) = self.scroll_info {
            let track_w = 160.0;
            let track_h = 4.0;
            let track_x = (self.viewport_width - track_w) / 2.0;
            let track_y = self.viewport_height - 10.0;

            let track_path = Path::rounded_rectangle(
                iced::Point::new(track_x, track_y),
                iced::Size::new(track_w, track_h),
                (track_h / 2.0).into(),
            );
            frame.fill(&track_path, Color::from_rgba(0.2, 0.2, 0.2, 0.45));

            let thumb_w = (track_w * visible_ratio).clamp(24.0, track_w);
            let thumb_x = track_x + progress * (track_w - thumb_w);
            let thumb_path = Path::rounded_rectangle(
                iced::Point::new(thumb_x, track_y),
                iced::Size::new(thumb_w, track_h),
                (track_h / 2.0).into(),
            );
            frame.fill(&thumb_path, Color::from_rgba(1.0, 1.0, 1.0, 0.65));
        }

        vec![frame.into_geometry()]
    }
}

fn overview_view<'a>(
    app: &'a app::State,
    overview_state: &'a app::service::overview::State,
) -> iced::Element<'a, app::Message> {
    if overview_state.items.is_empty() {
        return view::empty();
    }

    let config = app.config.load();
    let accent_color = config.default_window.border_color.to_iced();

    widget::canvas(OverviewOverlayCanvas {
        items: overview_state.items.clone(),
        accent_color,
    })
    .width(iced::Length::Fill)
    .height(iced::Length::Fill)
    .into()
}

struct OverviewOverlayCanvas {
    items: Vec<app::service::overview::OverviewItem>,
    accent_color: iced::Color,
}

impl canvas::Program<app::Message> for OverviewOverlayCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        for item in &self.items {
            let x = item.pos.x();
            let y = item.pos.y();
            let w = item.size.width();

            let badge_radius = 16.0;
            let center_x = x + w / 2.0;
            let center_y = if y >= 40.0 {
                y - 22.0
            } else {
                y + badge_radius + 8.0
            };
            let center = iced::Point::new(center_x, center_y);

            // Badge circular pill with dark background and accent ring
            let circle = Path::circle(center, badge_radius);
            frame.fill(&circle, Color::from_rgba(0.08, 0.09, 0.12, 0.92));
            frame.stroke(
                &circle,
                Stroke::default()
                    .with_color(self.accent_color)
                    .with_width(2.0),
            );

            // Centered white number text
            frame.fill_text(canvas::Text {
                content: format!("{}", item.number),
                position: iced::Point::new(center_x - 4.5, center_y - 7.5),
                color: Color::WHITE,
                size: 15.0.into(),
                ..Default::default()
            });
        }

        vec![frame.into_geometry()]
    }
}
