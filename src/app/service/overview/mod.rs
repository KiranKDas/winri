//! Overview mode: shows every tiled window scaled down and packed side by side
//! so they can all be seen at once. Each preview is a borderless, click-through window
//! onto which the real window is mirrored via a DWM thumbnail (see
//! [`thumbnail`]). The real windows are moved off-screen while overview is open
//! and restored when it closes. Opening is asynchronous because each preview
//! window must be created by the iced runtime before it can host a thumbnail.

mod thumbnail;

use anyhow::Context;
use iced::Task;
use itertools::Itertools;

use crate::{
    app::{
        self, Mode,
        service::overview::{self, thumbnail::ThumbnailId},
    },
    window::Window,
};

/// An item represented in overview mode, with an assigned number (1..=9).
#[derive(Debug, Clone)]
pub struct OverviewItem {
    pub window: Window,
    pub number: usize,
    pub pos: crate::utils::math::Position,
    pub size: crate::utils::math::Size,
}

/// State specific to overview mode: the preview windows currently open, paired
/// with their DWM thumbnail registration so both can be torn down on close.
pub struct State {
    pub opened_thumbnails: Vec<(ThumbnailId, iced::window::Id)>,
    pub items: Vec<OverviewItem>,
}

/// Payload reported once the iced runtime has finished creating a preview
/// window, carrying everything needed to bind the source window's thumbnail
/// onto it.
#[derive(Debug, Clone)]
pub struct ThumbnailWindowCreated {
    /// The real, tiled window being previewed.
    pub src: Window,
    /// The preview window's iced id (used to close it later).
    pub dest_id: iced::window::Id,
    /// The preview window's native `HWND`, needed for the DWM thumbnail API.
    pub dest_raw_handle: u64,
    pub size: crate::utils::math::Size,
}

#[derive(Debug, Clone)]
pub enum Message {
    ThumbnailWindowCreated(ThumbnailWindowCreated),
}

impl app::State {
    /// Computes the preview layout and spawns one preview window per tiled
    /// window. Each window's creation resolves into a
    /// [`Message::ThumbnailWindowCreated`], which [`Self::finalize_open_overview`]
    /// then turns into a live thumbnail. No-op if already in overview mode.
    pub fn prepare_open_overview(&mut self) -> Task<app::Message> {
        if matches!(self.mode, Mode::Overview(_)) {
            log::warn!(
                "Overview operation requested in {} while already in Overview mode",
                crate::function!()
            );
            return Task::none();
        }

        // Collect flat window list from the column/ribbon model.
        // Each column contributes a `WindowData` per window-item; we use the
        // column's width for the thumbnail scale calculation.
        let windows_data: Vec<thumbnail::WindowData> = self
            .tiler
            .active_ribbon()
            .columns
            .iter()
            .flat_map(|col| {
                col.windows.iter().map(move |item| thumbnail::WindowData {
                    inner: item.inner,
                    width: col.width as f32,
                })
            })
            .collect_vec();

        let thumbnails_data = thumbnail::compute_thumbnails_bounds_from_tiler_windows(
            &windows_data,
            self.tiler.work_area().size(),
            10.0,
        );

        let items: Vec<OverviewItem> = windows_data
            .iter()
            .zip(&thumbnails_data)
            .enumerate()
            .map(|(idx, (w, thumb))| OverviewItem {
                window: w.inner,
                number: idx + 1,
                pos: thumb.pos,
                size: thumb.size,
            })
            .collect();

        self.mode = Mode::Overview(State {
            opened_thumbnails: Vec::new(),
            items,
        });

        Task::batch(thumbnails_data.into_iter().zip(windows_data).map(
            |(thumbnail_data, window)| {
                thumbnail::thumbnail_window_creation_task(
                    window.inner,
                    thumbnail_data.pos,
                    thumbnail_data.size,
                )
                .then(|overview_message| Task::done(app::Message::Overview(overview_message)))
            },
        ))
    }

    pub fn handle_overview_message(&mut self, message: overview::Message) -> anyhow::Result<()> {
        match message {
            overview::Message::ThumbnailWindowCreated(thumbnail_window_created) => {
                self.finalize_open_overview(thumbnail_window_created)?;
            }
        }

        Ok(())
    }

    /// Completes opening for a single preview: moves the real windows
    /// off-screen, binds the source window's DWM thumbnail onto the (now
    /// created) preview window, and shows it on top. Switches into overview
    /// mode on the first preview.
    fn finalize_open_overview(
        &mut self,
        ThumbnailWindowCreated {
            src,
            dest_id,
            dest_raw_handle,
            size,
        }: ThumbnailWindowCreated,
    ) -> anyhow::Result<()> {
        let Mode::Overview(State {
            opened_thumbnails: thumbnails,
            ..
        }) = &mut self.mode
        else {
            return Ok(());
        };

        // Move all tiled windows offscreen while previews stand in for them.
        for item in self.tiler.windows() {
            item.inner.move_offscreen()?;
        }

        let dest_window = Window::from_safe_hwnd(dest_raw_handle)
            .context(dest_raw_handle)
            .context("invalid hwnd from thumbnail window")?;

        dest_window.set_no_activate()?;

        let thumbnail_id =
            thumbnail::bind_thumbnail(src, dest_window, size).context("thumbnail binding")?;

        dest_window.show()?;
        dest_window.set_max_zindex()?;

        thumbnails.push((thumbnail_id, dest_id));

        Ok(())
    }

    /// Tears down overview: unregisters every thumbnail, closes the preview
    /// windows, and returns to tiler mode (which moves the real windows back
    /// into place). No-op if not in overview mode.
    pub fn close_overview(&mut self) -> anyhow::Result<Task<app::Message>> {
        let Mode::Overview(State {
            opened_thumbnails: thumbnails,
            ..
        }) = &self.mode
        else {
            log::warn!(
                "Close overview requested in {} while not in Overview mode",
                crate::function!()
            );
            return Ok(Task::none());
        };

        let tasks = thumbnails
            .iter()
            .map(|(thumbnail_id, window_id)| {
                thumbnail::unbind_thumbnail(*thumbnail_id).map(|()| window_id)
            })
            .map(|window_id| window_id.map(|id| iced::window::close(*id)))
            .collect::<anyhow::Result<Vec<_>>>()?;

        self.switch_to_tiler_mode()?;

        Ok(Task::batch(tasks))
    }

    /// Tears down overview, focuses the window assigned to `number`, and returns to tiler mode.
    pub fn close_overview_and_focus(&mut self, number: usize) -> anyhow::Result<Task<app::Message>> {
        let Mode::Overview(State {
            opened_thumbnails: thumbnails,
            items,
        }) = &self.mode
        else {
            log::warn!(
                "Close overview and focus requested in {} while not in Overview mode",
                crate::function!()
            );
            return Ok(Task::none());
        };

        let target_window = items.iter().find(|it| it.number == number).map(|it| it.window);

        let tasks = thumbnails
            .iter()
            .map(|(thumbnail_id, window_id)| {
                thumbnail::unbind_thumbnail(*thumbnail_id).map(|()| window_id)
            })
            .map(|window_id| window_id.map(|id| iced::window::close(*id)))
            .collect::<anyhow::Result<Vec<_>>>()?;

        self.switch_to_tiler_mode()?;

        if let Some(target) = target_window {
            self.tiler.focus_window_by_handle(target);
            self.update_tiler()?;
        }

        Ok(Task::batch(tasks))
    }
}
