# Winri - Niri Feature Parity Implementation Plan

This plan tracks the implementation of features to achieve feature parity with [niri](https://github.com/YaLTeR/niri).
Implementation order: **Phase 4 -> Phase 3 -> Phase 5 -> Phase 6 -> Phase 7 -> Phase 1 & 2**.

---

## Progress Overview

- [x] **Phase 4: Mouse & Touchpad Gesture Engine** *(COMPLETED)*
  - [x] Mod + MouseWheel & Mod + MouseHWheel horizontal ribbon scroll
  - [x] Mod + Left-Click drag to move and reorder columns
  - [x] Mod + Right-Click drag to interactively resize column width
  - [x] Touchpad continuous ribbon panning support
- [x] **Phase 3: Dynamic Floating Window Layer** *(COMPLETED)*
  - [x] Window floating state tracking (`is_floating`, `floating_rect`)
  - [x] `toggle-window-floating` action (`Meta+Shift+Space`)
  - [x] `switch-focus-between-floating-and-tiling` action (`Meta+Space`)
  - [x] Floating window z-order and rendering management
- [x] **Phase 5: Visual Polish, Stack Indicators & Animations** *(COMPLETED)*
  - [x] Column vertical stack depth indicator (dots / tabs on overlay)
  - [x] Ribbon horizontal scroll mini-map / progress indicator
  - [x] Inactive window borders / styling options
  - [x] Smooth spring physics / cubic bezier animation curves
- [x] **Phase 6: Configurable Process Spawning & Enhanced Window Rules** *(COMPLETED)*
  - [x] Arbitrary `spawn` command keybindings in `winri.yaml`
  - [x] `open_on_workspace` automatic routing rule
  - [x] Regex-based window matching rules
  - [x] Per-rule border and opacity styling
- [x] **Phase 7: Named Pipe IPC & CLI (`winri msg`)** *(COMPLETED)*
  - [x] Named Pipe server (`\\.\pipe\winri-ipc`)
  - [x] CLI dispatch subcommands (`winri msg action <name>`, `winri msg query <type>`)
  - [x] Status bar integration protocol (workspaces, active window)
- [x] **Phase 1: Advanced Navigation & Column Controls** *(COMPLETED)*
  - [x] `focus-column-first` (`Meta+Home`) and `focus-column-last` (`Meta+End`)
  - [x] `move-column-to-first` (`Meta+Ctrl+Home`) and `move-column-to-last` (`Meta+Ctrl+End`)
  - [x] `focus-window-top` and `focus-window-bottom`
  - [x] Reorder window up/down within column stack (`move-window-up`/`down`)
  - [x] `expand-column-to-available-width`
  - [x] Configurable `preset_column_widths` in `winri.yaml`
- [x] **Phase 2: Workspaces & Multi-Monitor Foundation** *(COMPLETED)*
  - [x] Direct workspace switching via number keys (`Meta+1..9`)
  - [x] Move window to workspace (`Meta+Shift+1..9`)
  - [x] Move entire column to workspace (`Meta+Ctrl+1..9`)
  - [x] Multi-monitor ribbon engine (per-monitor ribbons, work areas, and DPI awareness)
