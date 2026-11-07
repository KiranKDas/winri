mod config_watcher;
mod input;
mod window;

use std::{path::PathBuf, thread, time::Duration};

use iced::{
    futures::{SinkExt, Stream, StreamExt, channel::mpsc::channel},
    stream,
};
use keyboard_types::Modifiers;

use crate::app::{Message, subscription::STREAM_CHANNEL_BUFFER_SIZE};

/// Button used for interactive mouse drag operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseDragButton {
    Left,
    Right,
}

/// A raw, mode-agnostic event from the OS-level hooks.
#[derive(Debug, Clone)]
pub enum GlobalMessage {
    Key(Modifiers, rdev::Key),
    /// Some window was created or moved; the tiler should re-sync.
    Window,
    ConfigChanged(PathBuf),
    /// 60fps animation tick for smooth ribbon scrolling
    Tick,
    /// Mouse wheel scroll with modifiers (e.g. Meta + Wheel)
    MouseScroll {
        delta_x: f32,
        delta_y: f32,
        modifiers: Modifiers,
    },
    /// Interactive mouse drag began (Meta + LMB or Meta + RMB)
    MouseDragStart {
        button: MouseDragButton,
        x: i32,
        y: i32,
    },
    /// Interactive mouse drag moved
    MouseDragUpdate {
        x: i32,
        y: i32,
    },
    /// Interactive mouse drag ended
    MouseDragEnd,
    /// Remote IPC command dispatch
    IpcAction(crate::app::action::Action),
}

/// The global subscription stream: launches the keyboard and window hooks and
/// merges their events into a single [`Message`] stream for the app.
#[allow(clippy::ref_option)]
pub fn subscription(config_source: &Option<PathBuf>) -> impl Stream<Item = Message> + use<> {
    let config_source = config_source.clone();
    stream::channel(STREAM_CHANNEL_BUFFER_SIZE, async |mut output| {
        let (intermediate_message_tx, mut intermediate_message_rx) = channel(100);

        let global_input_tx = intermediate_message_tx.clone();
        let window_event_tx = intermediate_message_tx.clone();
        let mut animation_tick_tx = intermediate_message_tx.clone();

        input::launch(global_input_tx);
        window::launch(window_event_tx);
        crate::ipc::launch_server(intermediate_message_tx.clone());

        let _ = thread::Builder::new()
            .name("winri-animation-ticker".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_millis(16));
                    let _ = animation_tick_tx.try_send(GlobalMessage::Tick);
                }
            });

        if let Some(config_source) = config_source {
            let config_watcher_event_tx = intermediate_message_tx.clone();
            config_watcher::launch(config_watcher_event_tx, config_source);
        }

        while let Some(event) = intermediate_message_rx.next().await {
            output.send(Message::Global(event)).await.unwrap();
        }
    })
}
