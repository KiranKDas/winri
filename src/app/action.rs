//! Semantic user intents, decoupled from the key combinations that trigger them.

#[derive(Debug, Clone)]
pub enum Action {
    Tiler(TilerAction),
    Overview(OverviewAction),
    Spawn(String),
    Exit,
}

#[derive(Debug, Clone)]
pub enum TilerAction {
    // Window lifecycle
    CloseCurrent,
    // Column navigation
    MoveFocusNext,
    MoveFocusPrevious,
    // In-column navigation
    MoveFocusUp,
    MoveFocusDown,
    // Two-Axis: Workspace navigation (Vertical)
    FocusWorkspaceUp,
    FocusWorkspaceDown,
    MoveWindowToWorkspaceUp,
    MoveWindowToWorkspaceDown,
    FocusWorkspace(usize),
    MoveWindowToWorkspace(usize),
    MoveColumnToWorkspace(usize),
    MoveColumnToWorkspaceUp,
    MoveColumnToWorkspaceDown,
    // In-column stacking operations
    MoveWindowIntoColumnAbove,
    MoveWindowIntoColumnBelow,
    ConsumeWindow,
    ExpelWindow,
    // Column reordering
    SwapWithNext,
    SwapWithPrevious,
    // Width and stack controls
    ResizeToFullscreen,
    ResizeToHalfScreen,
    MaximizeColumn,
    ResetWindowHeight,
    CycleWidth,
    IncrementWidth,
    DecrementWidth,
    // Misc
    OpenOverview,
    ForceRefresh,
    // Floating
    ToggleWindowFloating,
    SwitchFocusBetweenFloatingAndTiling,
    // Phase 1: Navigation & Column controls
    FocusColumnFirst,
    FocusColumnLast,
    MoveColumnToFirst,
    MoveColumnToLast,
    FocusWindowTop,
    FocusWindowBottom,
    MoveWindowUpInColumn,
    MoveWindowDownInColumn,
    ExpandColumnToAvailableWidth,
}

#[derive(Debug, Clone)]
pub enum OverviewAction {
    CloseOverview,
    FocusWindow(usize),
}
