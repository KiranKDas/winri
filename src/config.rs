use std::{
    fs,
    path::{Path, PathBuf},
};

use keyboard_types::Modifiers;
use serde::Deserialize;

use crate::{root_dir, system};

// ---------------------------------------------------------------------------
// Root config
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct Root {
    pub default_window: Window,
    pub keybindings: Keybindings,
    pub animations: Animations,
    pub layout: LayoutConfig,
    pub window_rules: Vec<WindowRule>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct LayoutConfig {
    pub center_focused_column: CenterFocusedColumn,
    pub preset_column_widths: Vec<f32>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            center_focused_column: CenterFocusedColumn::Always,
            preset_column_widths: vec![0.33333, 0.5, 0.66667, 1.0],
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CenterFocusedColumn {
    Always,
    OnOverflow,
    Never,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct WindowRule {
    pub matches: WindowRuleMatch,
    pub default_column_width: Option<ColumnWidthRule>,
    pub open_floating: Option<bool>,
    pub open_maximized_to_column: Option<bool>,
    pub open_fullscreen: Option<bool>,
    pub open_on_workspace: Option<usize>,
    pub block_out: Option<bool>,
    pub min_width: Option<i32>,
    pub max_width: Option<i32>,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct WindowRuleMatch {
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub class_name: Option<String>,
}

impl WindowRuleMatch {
    pub fn is_match(&self, app_id: &str, title: &str, class_name: &str) -> bool {
        if let Some(target_app) = &self.app_id {
            if !app_id.to_lowercase().contains(&target_app.to_lowercase()) {
                return false;
            }
        }
        if let Some(target_title) = &self.title {
            if !title.to_lowercase().contains(&target_title.to_lowercase()) {
                return false;
            }
        }
        if let Some(target_class) = &self.class_name {
            if !class_name.eq_ignore_ascii_case(target_class) {
                return false;
            }
        }
        true
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(untagged)]
pub enum ColumnWidthRule {
    Proportion { proportion: f32 },
    Fixed { fixed: i32 },
    Natural { natural: bool },
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Animations {
    pub enabled: bool,
    pub slowdown: Option<f32>,
}

impl Default for Animations {
    fn default() -> Self {
        Self {
            enabled: true,
            slowdown: None,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Translucency {
    pub enabled: bool,
    pub inactive_opacity: f32,
}

impl Default for Translucency {
    fn default() -> Self {
        Self {
            enabled: false,
            inactive_opacity: 0.88,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenPadding {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl Default for ScreenPadding {
    fn default() -> Self {
        Self {
            top: 10.0,
            bottom: 10.0,
            left: 10.0,
            right: 10.0,
        }
    }
}

impl<'de> serde::Deserialize<'de> for ScreenPadding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Uniform(f32),
            Explicit {
                #[serde(default = "default_padding_val")]
                top: f32,
                #[serde(default = "default_padding_val")]
                bottom: f32,
                #[serde(default = "default_padding_val")]
                left: f32,
                #[serde(default = "default_padding_val")]
                right: f32,
            },
        }

        fn default_padding_val() -> f32 { 10.0 }

        match Helper::deserialize(deserializer)? {
            Helper::Uniform(val) => Ok(Self {
                top: val,
                bottom: val,
                left: val,
                right: val,
            }),
            Helper::Explicit { top, bottom, left, right } => Ok(Self {
                top,
                bottom,
                left,
                right,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Window appearance
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Window {
    /// Padding / gap between apps in px (defaults to 10.0 px).
    #[serde(alias = "window_gap", alias = "gap", alias = "padding_between_apps", alias = "padding")]
    pub app_gap: f32,

    /// Padding between the screen and apps in px.
    #[serde(alias = "padding_between_screen_and_app", alias = "screen_gap", alias = "margin", alias = "struts")]
    pub screen_padding: ScreenPadding,

    pub resize_increment: f32,
    pub border_thickness: f32,
    pub border_color: csscolorparser::Color,
    pub border_radius: f32,
    pub translucency: Translucency,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            app_gap: default_app_gap(),
            screen_padding: ScreenPadding::default(),
            resize_increment: default_window_resize_increment(),
            border_thickness: default_window_border_thickness(),
            border_color: default_window_border_color(),
            border_radius: default_window_border_radius(),
            translucency: Translucency::default(),
        }
    }
}

const fn default_app_gap() -> f32 { 10.0 }
fn default_window_border_color() -> csscolorparser::Color {
    system::highlight_color().map_or_else(
        |_| csscolorparser::Color::from_linear_rgba(0.1, 0.2, 0.8, 1.0),
        |color| csscolorparser::Color::from_linear_rgba(color.r, color.g, color.b, color.a),
    )
}
const fn default_window_border_thickness() -> f32 { 4.0 }
const fn default_window_border_radius() -> f32 { 4.0 }
const fn default_window_resize_increment() -> f32 { 20.0 }

// ---------------------------------------------------------------------------
// Keybindings
//
// Each binding is a (modifiers, key) pair serialised as a YAML string of the
// form "Meta+Left" / "Meta+Ctrl+Up" / "Meta+Shift+C", matching the modifier
// names used by keyboard-types.  The deserializer is kept simple: modifier
// tokens are stripped from the left; whatever remains is the key name.
// ---------------------------------------------------------------------------

/// A parsed keybinding: a set of modifiers plus an `rdev::Key`.
#[derive(Debug, Clone)]
pub struct Binding {
    pub modifiers: Modifiers,
    pub key: rdev::Key,
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        parse_binding(&s).map_err(serde::de::Error::custom)
    }
}

/// Parses "Meta+Ctrl+Shift+Left" into a `Binding`.
fn parse_binding(s: &str) -> Result<Binding, String> {
    let mut modifiers = Modifiers::empty();
    let mut parts = s.split('+').collect::<Vec<_>>();

    // Drain modifier tokens from the front
    let mut consumed = 0;
    for part in &parts {
        match *part {
            "Meta" | "Win" | "Super" => {
                modifiers.insert(Modifiers::META);
                consumed += 1;
            }
            "Ctrl" | "Control" => {
                modifiers.insert(Modifiers::CONTROL);
                consumed += 1;
            }
            "Shift" => {
                modifiers.insert(Modifiers::SHIFT);
                consumed += 1;
            }
            "Alt" => {
                modifiers.insert(Modifiers::ALT);
                consumed += 1;
            }
            _ => break,
        }
    }
    parts.drain(..consumed);
    let key_str = parts.join("+");
    let key = parse_rdev_key(&key_str)
        .ok_or_else(|| format!("Unknown key '{key_str}' in binding '{s}'"))?;

    Ok(Binding { modifiers, key })
}

fn parse_rdev_key(s: &str) -> Option<rdev::Key> {
    use rdev::Key;
    Some(match s {
        "Left"  | "LeftArrow"  => Key::LeftArrow,
        "Right" | "RightArrow" => Key::RightArrow,
        "Up"    | "UpArrow"    => Key::UpArrow,
        "Down"  | "DownArrow"  => Key::DownArrow,
        "Escape" | "Esc"       => Key::Escape,
        "Space"                => Key::Space,
        "Return" | "Enter"     => Key::Return,
        "Tab"                  => Key::Tab,
        "Backspace"            => Key::Backspace,
        "Delete"               => Key::Delete,
        "Home"                 => Key::Home,
        "End"                  => Key::End,
        "PageUp"               => Key::PageUp,
        "PageDown"             => Key::PageDown,
        "F1"  => Key::F1,  "F2"  => Key::F2,  "F3"  => Key::F3,
        "F4"  => Key::F4,  "F5"  => Key::F5,  "F6"  => Key::F6,
        "F7"  => Key::F7,  "F8"  => Key::F8,  "F9"  => Key::F9,
        "F10" => Key::F10, "F11" => Key::F11, "F12" => Key::F12,
        "0" | "Num0" => Key::Num0,
        "1" | "Num1" => Key::Num1,
        "2" | "Num2" => Key::Num2,
        "3" | "Num3" => Key::Num3,
        "4" | "Num4" => Key::Num4,
        "5" | "Num5" => Key::Num5,
        "6" | "Num6" => Key::Num6,
        "7" | "Num7" => Key::Num7,
        "8" | "Num8" => Key::Num8,
        "9" | "Num9" => Key::Num9,
        "A" => Key::KeyA, "B" => Key::KeyB, "C" => Key::KeyC,
        "D" => Key::KeyD, "E" => Key::KeyE, "F" => Key::KeyF,
        "G" => Key::KeyG, "H" => Key::KeyH, "I" => Key::KeyI,
        "J" => Key::KeyJ, "K" => Key::KeyK, "L" => Key::KeyL,
        "M" => Key::KeyM, "N" => Key::KeyN, "O" => Key::KeyO,
        "P" => Key::KeyP, "Q" => Key::KeyQ, "R" => Key::KeyR,
        "S" => Key::KeyS, "T" => Key::KeyT, "U" => Key::KeyU,
        "V" => Key::KeyV, "W" => Key::KeyW, "X" => Key::KeyX,
        "Y" => Key::KeyY, "Z" => Key::KeyZ,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Default keybindings (niri-style)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Keybindings {
    // Column navigation
    pub focus_left:             Binding,
    pub focus_right:            Binding,
    // In-column navigation
    pub focus_up:               Binding,
    pub focus_down:             Binding,
    // Two-Axis: Workspace navigation (Vertical)
    pub focus_workspace_up:     Binding,
    pub focus_workspace_down:   Binding,
    pub move_to_workspace_up:   Binding,
    pub move_to_workspace_down: Binding,
    // Move window into adjacent column (stack vertically)
    pub move_into_col_above:    Binding,
    pub move_into_col_below:    Binding,
    // Workspaces by index & column moving
    pub focus_workspace_1:      Binding,
    pub focus_workspace_2:      Binding,
    pub focus_workspace_3:      Binding,
    pub focus_workspace_4:      Binding,
    pub focus_workspace_5:      Binding,
    pub focus_workspace_6:      Binding,
    pub focus_workspace_7:      Binding,
    pub focus_workspace_8:      Binding,
    pub focus_workspace_9:      Binding,
    pub move_to_workspace_1:    Binding,
    pub move_to_workspace_2:    Binding,
    pub move_to_workspace_3:    Binding,
    pub move_to_workspace_4:    Binding,
    pub move_to_workspace_5:    Binding,
    pub move_to_workspace_6:    Binding,
    pub move_to_workspace_7:    Binding,
    pub move_to_workspace_8:    Binding,
    pub move_to_workspace_9:    Binding,
    pub move_col_to_workspace_up: Binding,
    pub move_col_to_workspace_down: Binding,
    pub consume_window:         Binding,
    pub expel_window:           Binding,
    // Column reordering
    pub swap_col_left:          Binding,
    pub swap_col_right:         Binding,
    // Width and stack controls
    pub cycle_width:            Binding,
    pub maximize_column:        Binding,
    pub reset_window_height:    Binding,
    pub increment_width:        Binding,
    pub decrement_width:        Binding,
    // Misc
    pub close_current:          Binding,
    pub resize_fullscreen:      Binding,
    pub resize_halfscreen:      Binding,
    pub force_refresh:          Binding,
    pub open_overview:          Binding,
    pub close_overview_down:    Binding,
    pub close_overview_escape:  Binding,
    pub toggle_window_floating: Binding,
    pub switch_focus_floating:  Binding,
    pub focus_column_first:     Binding,
    pub focus_column_last:      Binding,
    pub move_column_to_first:   Binding,
    pub move_column_to_last:    Binding,
    pub focus_window_top:       Binding,
    pub focus_window_bottom:    Binding,
    pub move_window_up_in_column:   Binding,
    pub move_window_down_in_column: Binding,
    pub expand_column_to_available_width: Binding,
    pub exit:                   Binding,
    pub custom:                 Vec<SpawnBinding>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SpawnBinding {
    pub binding: Binding,
    pub spawn: String,
}

impl Default for Keybindings {
    fn default() -> Self {
        fn b(s: &str) -> Binding { parse_binding(s).expect("default binding is valid") }
        Self {
            focus_left:            b("Meta+Left"),
            focus_right:           b("Meta+Right"),
            focus_up:              b("Meta+Up"),
            focus_down:            b("Meta+Down"),
            focus_workspace_up:    b("Meta+PageUp"),
            focus_workspace_down:  b("Meta+PageDown"),
            move_to_workspace_up:  b("Meta+Shift+PageUp"),
            move_to_workspace_down: b("Meta+Shift+PageDown"),
            move_into_col_above:   b("Meta+Shift+Up"),
            move_into_col_below:   b("Meta+Shift+Down"),
            focus_workspace_1:     b("Meta+1"),
            focus_workspace_2:     b("Meta+2"),
            focus_workspace_3:     b("Meta+3"),
            focus_workspace_4:     b("Meta+4"),
            focus_workspace_5:     b("Meta+5"),
            focus_workspace_6:     b("Meta+6"),
            focus_workspace_7:     b("Meta+7"),
            focus_workspace_8:     b("Meta+8"),
            focus_workspace_9:     b("Meta+9"),
            move_to_workspace_1:   b("Meta+Shift+1"),
            move_to_workspace_2:   b("Meta+Shift+2"),
            move_to_workspace_3:   b("Meta+Shift+3"),
            move_to_workspace_4:   b("Meta+Shift+4"),
            move_to_workspace_5:   b("Meta+Shift+5"),
            move_to_workspace_6:   b("Meta+Shift+6"),
            move_to_workspace_7:   b("Meta+Shift+7"),
            move_to_workspace_8:   b("Meta+Shift+8"),
            move_to_workspace_9:   b("Meta+Shift+9"),
            move_col_to_workspace_up: b("Meta+Ctrl+PageUp"),
            move_col_to_workspace_down: b("Meta+Ctrl+PageDown"),
            consume_window:        b("Meta+Shift+C"),
            expel_window:          b("Meta+Shift+E"),
            swap_col_left:         b("Meta+Ctrl+Left"),
            swap_col_right:        b("Meta+Ctrl+Right"),
            cycle_width:           b("Meta+C"),
            maximize_column:       b("Meta+M"),
            reset_window_height:   b("Meta+Shift+R"),
            increment_width:       b("Meta+Shift+Right"),
            decrement_width:       b("Meta+Shift+Left"),
            close_current:         b("Meta+Q"),
            resize_fullscreen:     b("Meta+F"),
            resize_halfscreen:     b("Meta+H"),
            force_refresh:         b("Meta+R"),
            open_overview:         b("Meta+O"),
            close_overview_down:   b("Meta+Down"),
            close_overview_escape: b("Meta+Escape"),
            toggle_window_floating: b("Meta+Shift+Space"),
            switch_focus_floating:  b("Meta+Space"),
            focus_column_first:     b("Meta+Home"),
            focus_column_last:      b("Meta+End"),
            move_column_to_first:   b("Meta+Ctrl+Home"),
            move_column_to_last:    b("Meta+Ctrl+End"),
            focus_window_top:       b("Meta+Alt+Up"),
            focus_window_bottom:    b("Meta+Alt+Down"),
            move_window_up_in_column:   b("Meta+Alt+K"),
            move_window_down_in_column: b("Meta+Alt+J"),
            expand_column_to_available_width: b("Meta+E"),
            custom:                Vec::new(),
            exit:                  b("Meta+Escape"),
        }
    }
}

impl Keybindings {
    /// Returns `true` when `(modifiers, key)` matches this binding.
    pub fn matches(binding: &Binding, modifiers: Modifiers, key: rdev::Key) -> bool {
        binding.modifiers == modifiers && binding.key == key
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

fn config_files() -> anyhow::Result<Vec<PathBuf>> {
    let mut pathes = Vec::new();
    let root = root_dir()?;
    pathes.push(root.join("winri.yaml"));
    pathes.push(root.join("winri.yml"));
    pathes.push(root.join("config").join("winri.yaml"));
    pathes.push(root.join("config").join("winri.yml"));

    if let Some(base_config_dir) = dirs::config_dir() {
        let winri_dir = base_config_dir.join("winri");
        pathes.push(winri_dir.join("winri.yaml"));
        pathes.push(winri_dir.join("winri.yml"));
        pathes.push(winri_dir.join("config").join("winri.yaml"));
        pathes.push(winri_dir.join("config").join("winri.yml"));
    }
    Ok(pathes)
}

fn elect_config_file(pathes: Vec<PathBuf>) -> Option<PathBuf> {
    for path in pathes {
        log::info!("Looking for conf at {}", path.display());
        if fs::exists(&path).unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

pub fn load() -> anyhow::Result<(Root, Option<PathBuf>)> {
    let config_files = config_files()?;
    if let Some(config_file_path) = elect_config_file(config_files) {
        return Ok((load_from(&config_file_path)?, Some(config_file_path)));
    }

    // Automatically create a default winri.yaml if none was found
    if let Ok(root) = root_dir() {
        let default_target = root.join("winri.yaml");
        let _ = fs::create_dir_all(&root);
        const DEFAULT_CONFIG: &str = include_str!("../winri.example.yaml");
        if fs::write(&default_target, DEFAULT_CONFIG).is_ok() {
            log::info!("Generated default config file at {}", default_target.display());
            if let Ok(config) = load_from(&default_target) {
                return Ok((config, Some(default_target)));
            }
        }
    }

    let config = Root::default();
    log::info!("No config file found, loading defaults:\n {config:#?}");
    Ok((config, None))
}

pub fn load_from<P: AsRef<Path>>(path: P) -> anyhow::Result<Root> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    log::info!("conf content loaded:\n {content}");
    let config = yaml_serde::from_str::<Root>(content)?;
    log::info!("Config successfully parsed:\n {config:#?}");
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_binding_modifiers_and_key() {
        let b = parse_binding("Meta+Left").unwrap();
        assert_eq!(b.modifiers, Modifiers::META);
        assert_eq!(b.key, rdev::Key::LeftArrow);

        let b2 = parse_binding("Meta+Shift+Down").unwrap();
        assert_eq!(b2.modifiers, Modifiers::META | Modifiers::SHIFT);
        assert_eq!(b2.key, rdev::Key::DownArrow);

        let b3 = parse_binding("Meta+Ctrl+Right").unwrap();
        assert_eq!(b3.modifiers, Modifiers::META | Modifiers::CONTROL);
        assert_eq!(b3.key, rdev::Key::RightArrow);

        let b4 = parse_binding("Meta+C").unwrap();
        assert_eq!(b4.modifiers, Modifiers::META);
        assert_eq!(b4.key, rdev::Key::KeyC);

        let b5 = parse_binding("Meta+F").unwrap();
        assert_eq!(b5.modifiers, Modifiers::META);
        assert_eq!(b5.key, rdev::Key::KeyF);
    }

    #[test]
    fn test_keybindings_matches() {
        let binding = parse_binding("Meta+Left").unwrap();
        assert!(Keybindings::matches(&binding, Modifiers::META, rdev::Key::LeftArrow));
        assert!(!Keybindings::matches(&binding, Modifiers::CONTROL, rdev::Key::LeftArrow));
        assert!(!Keybindings::matches(&binding, Modifiers::META, rdev::Key::RightArrow));
    }

    #[test]
    fn test_yaml_config_with_utf8_bom() {
        let yaml = "\u{feff}
default_window:
  app_gap: 12.0
";
        let content = yaml.strip_prefix('\u{feff}').unwrap_or(yaml);
        let root: Root = yaml_serde::from_str(content).unwrap();
        assert_eq!(root.default_window.app_gap, 12.0);
    }

    #[test]
    fn test_default_config_translucency_and_animations() {
        let root = Root::default();
        // Translucency is optional and disabled by default
        assert!(!root.default_window.translucency.enabled);
        assert_eq!(root.default_window.translucency.inactive_opacity, 0.88);
        // Animations enabled by default
        assert!(root.animations.enabled);
    }

    #[test]
    fn test_yaml_config_translucency_toggle() {
        let yaml = r#"
default_window:
  translucency:
    enabled: true
    inactive_opacity: 0.75
animations:
  enabled: false
"#;
        let root: Root = yaml_serde::from_str(yaml).unwrap();
        assert!(root.default_window.translucency.enabled);
        assert_eq!(root.default_window.translucency.inactive_opacity, 0.75);
        assert!(!root.animations.enabled);
    }

    #[test]
    fn test_yaml_config_padding_and_gap_options() {
        let yaml = r#"
default_window:
  app_gap: 15.0
  screen_padding: 20.0
"#;
        let root: Root = yaml_serde::from_str(yaml).unwrap();
        assert_eq!(root.default_window.app_gap, 15.0);
        assert_eq!(root.default_window.screen_padding.left, 20.0);
        assert_eq!(root.default_window.screen_padding.right, 20.0);
        assert_eq!(root.default_window.screen_padding.top, 20.0);
        assert_eq!(root.default_window.screen_padding.bottom, 20.0);

        let yaml_detailed = r#"
default_window:
  padding_between_apps: 12.0
  screen_padding:
    top: 8.0
    bottom: 8.0
    left: 16.0
    right: 16.0
"#;
        let root2: Root = yaml_serde::from_str(yaml_detailed).unwrap();
        assert_eq!(root2.default_window.app_gap, 12.0);
        assert_eq!(root2.default_window.screen_padding.top, 8.0);
        assert_eq!(root2.default_window.screen_padding.bottom, 8.0);
        assert_eq!(root2.default_window.screen_padding.left, 16.0);
        assert_eq!(root2.default_window.screen_padding.right, 16.0);
    }
}
