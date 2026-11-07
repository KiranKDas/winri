# Winri Architecture & Development Rules (Niri Parity)

These rules are strictly enforced across the codebase to maintain fidelity to the [niri](https://github.com/niri-wm/niri) architectural model and ensure stability on Windows 11.

---

### Rule 1: Windows Are Never Resized Automatically During Navigation or Scrolling
* **Fixed Dimensions**: A window's width is strictly determined by its `Column.width`.
* **No Clipping Resizes**: Windows must **never** be resized when crossing screen boundaries (e.g., left taskbar or monitor edges). Resizing windows during ribbon scrolling causes massive UI reflows, stuttering, and GPU swapchain churn.
* **Manual Resize Only**: Windows only resize when the user explicitly requests it:
  - Width cycling (`Win + C`)
  - Incremental resizing (`Win + Shift + Left` / `Win + Shift + Right`)
  - Fullscreen toggle (`Win + F`)
  - Adding/removing windows in a vertical column stack (`windows.len()`)
* **Movement Optimization**: During scroll animations, layout passes must preserve window width and height, updating only the `x` and `y` coordinates with `SWP_NOSIZE`.

---

### Rule 2: Ribbon -> Column -> Window Stack Architecture & Spacing
* **The Ribbon**: An endless horizontal strip of columns separated by `app_gap`.
* **Distinct Padding Options (in px)**:
  - `screen_padding`: The margin/padding between the screen edges and the windows in px (supports uniform float or `{ top, bottom, left, right }`).
  - `app_gap`: The padding/gap between adjacent windows and columns in px.
* **In-Column Vertical Stacking**: Windows within a column share the column height equally:
  `window_height = (usable_height - (windows.len() - 1) * app_gap) / windows.len()`
  where `usable_height = viewport_height - screen_padding.top - screen_padding.bottom`.
* **Fullscreen is a Column**: A fullscreen window is simply a column with `width = viewport_width` and zero gaps. It sits in the ribbon at its natural position and scrolls horizontally into and out of view like any other column. Windows must **never** be teleported off-screen unless completely scrolled out of the viewport.

---

### Rule 3: Config-Driven & Optional Features
* **Padding & Gaps (`winri.yaml`)**:
  ```yaml
  default_window:
    # Padding between the screen and the app (px)
    screen_padding: 16.0    # or: { top: 10, bottom: 10, left: 16, right: 16 }
    # Padding between the apps (px)
    app_gap: 12.0
  ```
* **Translucency (Opacity)**:
  - Configured via `default_window.translucency.enabled` in `winri.yaml` (default: `false`).
  - When disabled, Winri must **never** touch `WS_EX_LAYERED` or `SetLayeredWindowAttributes`.
  - When enabled, opacity is updated **only on focus transitions**, never on animation ticks.
* **Animations**:
  - Configured via `animations.enabled` in `winri.yaml` (default: `true`).
  - When disabled, scrolling snaps instantly to target positions.
  - When enabled, scrolling uses smooth, non-resizing translations.

---

### Rule 4: Zero GPU/CPU Thrashing & No Event Feedback Loops
* **Atomic Suppressions**: `IS_APPLYING_LAYOUT` must always wrap `DeferWindowPos` batches to prevent `WinEventHook` (`EVENT_OBJECT_LOCATIONCHANGE`) from triggering recursive layout updates.
* **Property Caching**: Never call Win32 style modification APIs (`SetWindowLongPtrW`, `SetLayeredWindowAttributes`) repeatedly if the target state has not changed.

---

### Rule 5: Rigorous Verification with Unit Tests
* Every new feature or layout math modification must include unit tests.
* Run `cargo test` and verify that all tests pass cleanly with 0 compiler warnings before committing or reporting changes.
