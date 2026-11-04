use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

use crate::app::DatView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ActionId {
    Open,
    OpenWithFormat,
    AddFiles,
    Reload,
    SaveSession,
    SaveSessionAs,
    OpenSession,
    ExportFigure,
    ExportData,
    ExportScript,
    CopyFigure,
    Settings,
    Quit,
    ViewChart,
    ViewTable,
    ViewInfo,
    ModeSignals,
    ModeDistribution,
    ModeSpectrum,
    ModeNormalized,
    ModeCompare,
    ModePolar,
    ToggleStats,
    ToggleSidebar,
    ToggleFollow,
    Search,
    Palette,
    Shortcuts,
    About,
    Fit,
    NewChannel,
    Processing,
    CurveFit,
    Damping,
    Fatigue,
    StatsReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Menu {
    File,
    Edit,
    View,
    Data,
    Analyze,
    Help,
}

impl Menu {
    pub(crate) const ALL: [Self; 6] = [
        Self::File,
        Self::Edit,
        Self::View,
        Self::Data,
        Self::Analyze,
        Self::Help,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Data => "Data",
            Self::Analyze => "Analyze",
            Self::Help => "Help",
        }
    }
}

/// Availability captured by the shell for the current frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ActionContext {
    pub has_sources: bool,
    pub has_series: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShortcutScope {
    Global,
    Chart,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Shortcut {
    pub keyboard: KeyboardShortcut,
    pub scope: ShortcutScope,
}

impl Shortcut {
    pub(crate) fn text(self, ctx: &egui::Context) -> String {
        ctx.format_shortcut(&self.keyboard)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Action {
    pub id: ActionId,
    pub label: &'static str,
    pub menu: Menu,
    /// Consecutive actions with the same group share a menu section.
    pub group: u8,
    pub shortcut: Option<Shortcut>,
    pub alternate_shortcut: Option<Shortcut>,
    pub enabled: fn(&ActionContext) -> bool,
    pub run: fn(&mut DatView, &egui::Context),
}

impl Action {
    pub(crate) fn is_enabled(&self, state: &ActionContext) -> bool {
        (self.enabled)(state)
    }

    /// Use the same availability check for menus, the palette, and shortcuts.
    pub(crate) fn execute(
        &self,
        app: &mut DatView,
        ctx: &egui::Context,
        state: &ActionContext,
    ) -> bool {
        if !self.is_enabled(state) {
            return false;
        }
        (self.run)(app, ctx);
        true
    }
}

const fn global(modifiers: Modifiers, key: Key) -> Option<Shortcut> {
    Some(Shortcut {
        keyboard: KeyboardShortcut::new(modifiers, key),
        scope: ShortcutScope::Global,
    })
}

const fn chart_key(key: Key) -> Option<Shortcut> {
    Some(Shortcut {
        keyboard: KeyboardShortcut::new(Modifiers::NONE, key),
        scope: ShortcutScope::Chart,
    })
}

fn always(_: &ActionContext) -> bool {
    true
}

fn has_sources(state: &ActionContext) -> bool {
    state.has_sources
}

fn has_series(state: &ActionContext) -> bool {
    state.has_series
}

macro_rules! action {
    ($id:ident, $label:literal, $menu:ident / $group:literal, $shortcut:expr, $alternate:expr, $enabled:ident) => {
        Action {
            id: ActionId::$id,
            label: $label,
            menu: Menu::$menu,
            group: $group,
            shortcut: $shortcut,
            alternate_shortcut: $alternate,
            enabled: $enabled,
            run: |app, ctx| app.run_action(ActionId::$id, ctx),
        }
    };
}

const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

/// The single source of truth for available commands and their presentation.
pub(crate) static ACTIONS: &[Action] = &[
    action!(
        Open,
        "Open file…",
        File / 0,
        global(Modifiers::COMMAND, Key::O),
        None,
        always
    ),
    action!(
        AddFiles,
        "Add files…",
        File / 0,
        global(CMD_SHIFT, Key::O),
        None,
        always
    ),
    action!(
        OpenWithFormat,
        "Open with format…",
        File / 0,
        None,
        None,
        always
    ),
    action!(
        Reload,
        "Reload",
        File / 0,
        global(Modifiers::NONE, Key::F5),
        None,
        has_sources
    ),
    action!(OpenSession, "Open session…", File / 1, None, None, always),
    action!(
        SaveSession,
        "Save session",
        File / 1,
        global(Modifiers::COMMAND, Key::S),
        None,
        has_sources
    ),
    action!(
        SaveSessionAs,
        "Save session as…",
        File / 1,
        global(CMD_SHIFT, Key::S),
        None,
        has_sources
    ),
    action!(
        ExportFigure,
        "Export figure…",
        File / 2,
        global(Modifiers::COMMAND, Key::E),
        None,
        has_series
    ),
    action!(
        CopyFigure,
        "Copy figure",
        File / 2,
        global(CMD_SHIFT, Key::C),
        None,
        has_series
    ),
    action!(
        ExportData,
        "Export data…",
        File / 2,
        global(CMD_SHIFT, Key::E),
        None,
        has_sources
    ),
    action!(
        ExportScript,
        "Export Python script…",
        File / 2,
        None,
        None,
        has_series
    ),
    action!(
        Quit,
        "Quit",
        File / 3,
        global(Modifiers::COMMAND, Key::Q),
        None,
        always
    ),
    action!(
        Search,
        "Find channel",
        Edit / 0,
        global(Modifiers::COMMAND, Key::F),
        global(Modifiers::NONE, Key::Slash),
        has_sources
    ),
    action!(
        Settings,
        "Settings…",
        Edit / 1,
        global(Modifiers::COMMAND, Key::Comma),
        None,
        always
    ),
    action!(
        ViewChart,
        "Chart",
        View / 0,
        global(Modifiers::COMMAND, Key::Num1),
        None,
        always
    ),
    action!(
        ViewTable,
        "Table",
        View / 0,
        global(Modifiers::COMMAND, Key::Num2),
        None,
        has_sources
    ),
    action!(
        ViewInfo,
        "Info",
        View / 0,
        global(Modifiers::COMMAND, Key::Num3),
        None,
        has_sources
    ),
    action!(
        ModeSignals,
        "Signals",
        View / 1,
        global(Modifiers::ALT, Key::Num1),
        None,
        always
    ),
    action!(
        ModeDistribution,
        "Distribution",
        View / 1,
        global(Modifiers::ALT, Key::Num2),
        None,
        always
    ),
    action!(
        ModeSpectrum,
        "Spectrum",
        View / 1,
        global(Modifiers::ALT, Key::Num3),
        None,
        always
    ),
    action!(
        ModeNormalized,
        "Normalized",
        View / 1,
        global(Modifiers::ALT, Key::Num4),
        None,
        always
    ),
    action!(
        ModeCompare,
        "Compare",
        View / 1,
        global(Modifiers::ALT, Key::Num5),
        None,
        always
    ),
    action!(
        ModePolar,
        "Polar",
        View / 1,
        global(Modifiers::ALT, Key::Num6),
        None,
        always
    ),
    action!(
        ToggleSidebar,
        "Show or hide sidebar",
        View / 2,
        global(Modifiers::COMMAND, Key::B),
        None,
        always
    ),
    action!(
        ToggleStats,
        "Show or hide statistics",
        View / 2,
        global(Modifiers::COMMAND, Key::J),
        None,
        always
    ),
    action!(
        ToggleFollow,
        "Follow file changes",
        View / 2,
        None,
        None,
        always
    ),
    action!(
        Fit,
        "Fit all",
        View / 3,
        chart_key(Key::Home),
        chart_key(Key::Num0),
        has_series
    ),
    action!(
        Palette,
        "Command palette…",
        View / 4,
        global(Modifiers::COMMAND, Key::K),
        global(CMD_SHIFT, Key::P),
        always
    ),
    action!(
        NewChannel,
        "New channel…",
        Data / 0,
        global(Modifiers::COMMAND, Key::N),
        None,
        has_sources
    ),
    action!(
        Processing,
        "Processing pipeline…",
        Data / 0,
        None,
        None,
        has_sources
    ),
    action!(CurveFit, "Curve fit…", Analyze / 0, None, None, has_series),
    action!(
        Damping,
        "Damping (logarithmic decrement)…",
        Analyze / 0,
        None,
        None,
        has_series
    ),
    action!(
        Fatigue,
        "Fatigue equivalent load…",
        Analyze / 0,
        None,
        None,
        has_series
    ),
    action!(
        StatsReport,
        "Statistics report…",
        Analyze / 1,
        None,
        None,
        has_series
    ),
    action!(
        Shortcuts,
        "Keyboard shortcuts",
        Help / 0,
        global(Modifiers::NONE, Key::F1),
        global(Modifiers::NONE, Key::Questionmark),
        always
    ),
    action!(About, "About rustDatView", Help / 1, None, None, always),
];

pub(crate) fn find(id: ActionId) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.id == id)
}

/// Consume at most one global action shortcut, preferring more specific modifiers.
/// Bare and Alt shortcuts never interfere with a focused text edit. Chart-scoped
/// shortcuts are consumed by the chart itself only when it has focus or is hovered.
pub(crate) fn handle_shortcuts(
    app: &mut DatView,
    ctx: &egui::Context,
    state: &ActionContext,
) -> bool {
    let text_focused = ctx.text_edit_focused();
    // egui accepts additional Shift/Alt modifiers, so Ctrl+Shift+S must be
    // considered before Ctrl+S, independently of menu presentation order.
    for specificity in (0..=3).rev() {
        for action in ACTIONS {
            if !action.is_enabled(state) {
                continue;
            }
            for shortcut in [action.shortcut, action.alternate_shortcut]
                .into_iter()
                .flatten()
            {
                if shortcut.scope == ShortcutScope::Chart {
                    continue;
                }
                let keyboard = shortcut.keyboard;
                if text_focused
                    && (!keyboard.modifiers.command || keyboard.conflicts_with_text_editing())
                {
                    continue;
                }
                let modifiers = keyboard.modifiers;
                let count = u8::from(modifiers.command || modifiers.ctrl || modifiers.mac_cmd)
                    + u8::from(modifiers.alt)
                    + u8::from(modifiers.shift);
                if count == specificity && ctx.input_mut(|input| input.consume_shortcut(&keyboard))
                {
                    return action.execute(app, ctx, state);
                }
            }
        }
    }
    false
}
