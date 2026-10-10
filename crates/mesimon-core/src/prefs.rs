//! The names of the machine preferences (T-361).
//!
//! `prefs.json` is the TUI's file and its fields live there; this module
//! holds only the KEYS, so the keymap can name a settings row by the
//! preference it edits and so one list says which of them a board may
//! override. A board's overrides live in a second, sparse `prefs.json` under
//! that repo's state dir — private to the machine, never shared through a
//! team board — and an absent key there means "inherit the machine's".

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefKey {
    Dark,
    Light,
    SnoozeNeedsYou,
    WeekStart,
    MergeTrain,
    MergeTrainNotice,
    StatusTop,
    /// The terminal's own tab or window title names the board (T-492): a
    /// named value — `off`, `project` (the board's name alone) or
    /// `mesimon` (`mesimon ∙ <board>`). Off by default, per machine — a
    /// title is the terminal's, not a board's. The keys after it are the
    /// rest of the terminal integrations, each its own row.
    TabTitle,
    /// The tab title counts the tickets that need you.
    TabTitleNeedsYou,
    /// The tab title names the ticket whose pane is on screen.
    TabTitleFocus,
    /// A progress ring in the tab (OSC 9;4): spinning while an agent
    /// works, red while one needs you.
    TabProgress,
    /// iTerm2 paints the whole tab in the theme's hint-line colour (T-528),
    /// under the needs-you mark when that row paints the whole tab too.
    TabTheme,
    /// iTerm2 marks the tab in the attention colour while any ticket
    /// needs you: `off`, `dot` (the tab's indicator) or `tab` (its chrome).
    TabColor,
    /// iTerm2's tab subtitle counts what needs you and what is working.
    TabSubtitle,
    /// iTerm2's tab icon is the shin, resting or needs-you by state.
    TabIcon,
    KeepAwake,
    Notify,
    NotifyDone,
    NotifyFocused,
    NotifyInPane,
    /// Say what the crown is woken for, on the agents it started, to the
    /// person as well (T-678). Off by default: the crown is told instead.
    NotifyCrown,
    NotifyWords,
    NotifySoundNeedsYou,
    NotifySoundDone,
    /// iTerm2 bounces its dock icon once when an agent needs you (T-492).
    /// A notification, so it sits in that list and under that switch.
    NotifyDockBounce,
    /// Who posts the banner (T-676): a named value — `mesimon` (the
    /// helper, with the mascot; the default) or `terminal` (the terminal
    /// the board runs in, by an escape, signed as itself). About the
    /// terminal, so per machine.
    NotifyVia,
    /// The board's reply row (T-365): which rung `p`/`P` left it on.
    Peek,
    /// The crown's actions strike their tickets with a bolt (T-544). On by
    /// default; off keeps the board still and the card's word alone says
    /// what the crown did.
    CrownLightning,
    /// The subscription quota line above the board's keys (T-327): a named
    /// value — `near` (only while a provider warns, the default), `every`
    /// window, the `headline` alone, or `off`. The six after it say which
    /// windows and which providers it may use, and when it names a reset.
    UsageLine,
    UsageFiveHour,
    UsageWeekly,
    UsageModel,
    /// `near` (with a warning), `always` or `never`.
    UsageResets,
    UsageClaude,
    UsageCodex,
    /// What a card's corner says (T-327): its `age` (the default) or its
    /// ticket's estimated `cost` — `$` on the board flips it.
    CardCorner,
    /// How a ticket's `Summary` section shows on its card (T-696): the
    /// underline and the rows, the rows alone, or nothing. Per machine, a
    /// view preference like the replies' rung.
    Summary,
    /// A resting card names what its agent said since the person last
    /// looked, one muted line under its title (T-720). On by default; per
    /// machine, a view preference like the summary's.
    NewReplies,
    /// The board reads the mouse (T-716): clicks, the wheel and hover. On
    /// by default; off gives the terminal its own text selection back,
    /// which reading the mouse takes. Per machine: it is about the terminal.
    Mouse,
}

impl PrefKey {
    pub const ALL: [PrefKey; 39] = [
        PrefKey::Dark,
        PrefKey::Light,
        PrefKey::SnoozeNeedsYou,
        PrefKey::WeekStart,
        PrefKey::MergeTrain,
        PrefKey::MergeTrainNotice,
        PrefKey::StatusTop,
        PrefKey::TabTitle,
        PrefKey::TabTitleNeedsYou,
        PrefKey::TabTitleFocus,
        PrefKey::TabProgress,
        PrefKey::TabTheme,
        PrefKey::TabColor,
        PrefKey::TabSubtitle,
        PrefKey::TabIcon,
        PrefKey::KeepAwake,
        PrefKey::Notify,
        PrefKey::NotifyDone,
        PrefKey::NotifyFocused,
        PrefKey::NotifyInPane,
        PrefKey::NotifyCrown,
        PrefKey::NotifyWords,
        PrefKey::NotifySoundNeedsYou,
        PrefKey::NotifySoundDone,
        PrefKey::NotifyDockBounce,
        PrefKey::NotifyVia,
        PrefKey::Peek,
        PrefKey::CrownLightning,
        PrefKey::UsageLine,
        PrefKey::UsageFiveHour,
        PrefKey::UsageWeekly,
        PrefKey::UsageModel,
        PrefKey::UsageResets,
        PrefKey::UsageClaude,
        PrefKey::UsageCodex,
        PrefKey::CardCorner,
        PrefKey::Summary,
        PrefKey::NewReplies,
        PrefKey::Mouse,
    ];

    /// The JSON key in both files.
    pub const fn name(self) -> &'static str {
        match self {
            PrefKey::Dark => "dark",
            PrefKey::Light => "light",
            PrefKey::SnoozeNeedsYou => "snooze_needs_you",
            PrefKey::WeekStart => "week_start",
            PrefKey::MergeTrain => "merge_train",
            PrefKey::MergeTrainNotice => "merge_train_notice",
            PrefKey::StatusTop => "status_line_top",
            PrefKey::TabTitle => "tab_title",
            PrefKey::TabTitleNeedsYou => "tab_title_needs_you",
            PrefKey::TabTitleFocus => "tab_title_focus",
            PrefKey::TabProgress => "tab_progress",
            PrefKey::TabTheme => "tab_theme",
            PrefKey::TabColor => "tab_color",
            PrefKey::TabSubtitle => "tab_subtitle",
            PrefKey::TabIcon => "tab_icon",
            PrefKey::KeepAwake => "keep_awake",
            PrefKey::Notify => "notify",
            PrefKey::NotifyDone => "notify_done",
            PrefKey::NotifyFocused => "notify_focused",
            PrefKey::NotifyInPane => "notify_in_pane",
            PrefKey::NotifyCrown => "notify_crown",
            PrefKey::NotifyWords => "notify_words",
            PrefKey::NotifySoundNeedsYou => "notify_sound_needs_you",
            PrefKey::NotifySoundDone => "notify_sound_done",
            PrefKey::NotifyDockBounce => "notify_dock_bounce",
            PrefKey::NotifyVia => "notify_via",
            PrefKey::Peek => "peek",
            PrefKey::CrownLightning => "crown_lightning",
            PrefKey::UsageLine => "usage_line",
            PrefKey::UsageFiveHour => "usage_5h",
            PrefKey::UsageWeekly => "usage_week",
            PrefKey::UsageModel => "usage_model",
            PrefKey::UsageResets => "usage_resets",
            PrefKey::UsageClaude => "usage_claude",
            PrefKey::UsageCodex => "usage_codex",
            PrefKey::CardCorner => "card_corner",
            PrefKey::Summary => "summary",
            PrefKey::NewReplies => "new_replies",
            PrefKey::Mouse => "mouse",
        }
    }

    /// May one board set this on its own? The machine keeps: where the
    /// tmux status line sits and the eight terminal integrations (T-492),
    /// which are about the terminal the board runs in and not about a
    /// repo; which day a week starts on, which is about the person; and
    /// the reply row's rung (T-365), which is about how the person reads a
    /// board and has no Settings row to set it per board — `p` and `P` set
    /// it. The dock bounce is a notification and follows its group. The
    /// crown's lightning (T-544) is motion on the screen, which is about
    /// the person watching it, so one switch holds for every board. The
    /// usage line's seven (T-327) are about the person's subscription,
    /// which is one sign-in whichever board shows it. Who posts the banner
    /// (T-676) is a question about the terminal, so it is the machine's.
    pub fn board_overridable(self) -> bool {
        !matches!(
            self,
            PrefKey::StatusTop
                | PrefKey::TabTitle
                | PrefKey::TabTitleNeedsYou
                | PrefKey::TabTitleFocus
                | PrefKey::TabProgress
                | PrefKey::TabTheme
                | PrefKey::TabColor
                | PrefKey::TabSubtitle
                | PrefKey::TabIcon
                | PrefKey::NotifyVia
                | PrefKey::WeekStart
                | PrefKey::Peek
                | PrefKey::CrownLightning
                | PrefKey::UsageLine
                | PrefKey::UsageFiveHour
                | PrefKey::UsageWeekly
                | PrefKey::UsageModel
                | PrefKey::UsageResets
                | PrefKey::UsageClaude
                | PrefKey::UsageCodex
                | PrefKey::CardCorner
                | PrefKey::Summary
                | PrefKey::NewReplies
                | PrefKey::Mouse
        )
    }

    /// The words a status line calls it.
    pub fn label(self) -> &'static str {
        match self {
            PrefKey::Dark => "dark theme",
            PrefKey::Light => "light theme",
            PrefKey::SnoozeNeedsYou => "snooze needs-you",
            PrefKey::WeekStart => "week start",
            PrefKey::MergeTrain => "auto merge",
            PrefKey::MergeTrainNotice => "auto merge notice",
            PrefKey::StatusTop => "status line",
            PrefKey::TabTitle => "terminal tab title",
            PrefKey::TabTitleNeedsYou => "tab title counts needs-you",
            PrefKey::TabTitleFocus => "tab title follows the open session",
            PrefKey::TabProgress => "tab progress ring",
            PrefKey::TabTheme => "tab colour from the theme",
            PrefKey::TabColor => "tab colour when needs you",
            PrefKey::TabSubtitle => "tab subtitle",
            PrefKey::TabIcon => "tab icon",
            PrefKey::KeepAwake => "keep awake",
            PrefKey::Notify => "notifications",
            PrefKey::NotifyDone => "notify when a turn lands",
            PrefKey::NotifyFocused => "notify while focused",
            PrefKey::NotifyInPane => "notify in the pane",
            PrefKey::NotifyCrown => "notify for the crown's agents",
            PrefKey::NotifyWords => "notify with the agent's words",
            PrefKey::NotifySoundNeedsYou => "needs-you sound",
            PrefKey::NotifySoundDone => "done sound",
            PrefKey::NotifyDockBounce => "dock bounce",
            PrefKey::NotifyVia => "Delivered by",
            PrefKey::Peek => "replies",
            PrefKey::Summary => "summary on cards",
            PrefKey::NewReplies => "new replies on cards",
            PrefKey::CrownLightning => "crown lightning",
            PrefKey::UsageLine => "usage line",
            PrefKey::UsageFiveHour => "usage line's 5-hour window",
            PrefKey::UsageWeekly => "usage line's weekly window",
            PrefKey::UsageModel => "usage line's per-model windows",
            PrefKey::UsageResets => "usage line's reset times",
            PrefKey::UsageClaude => "usage line's claude",
            PrefKey::UsageCodex => "usage line's codex",
            PrefKey::CardCorner => "card corner",
            PrefKey::Mouse => "mouse",
        }
    }

    /// A plain switch, as opposed to a named value (a theme, a sound, a day).
    pub fn is_bool(self) -> bool {
        !matches!(
            self,
            PrefKey::Dark
                | PrefKey::Light
                | PrefKey::TabTitle
                | PrefKey::TabColor
                | PrefKey::WeekStart
                | PrefKey::NotifySoundNeedsYou
                | PrefKey::NotifySoundDone
                | PrefKey::NotifyVia
                | PrefKey::Peek
                | PrefKey::UsageLine
                | PrefKey::UsageResets
                | PrefKey::CardCorner
                | PrefKey::Summary
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-588: the Claude road is no setting, on either layer.
    #[test]
    fn the_claude_road_is_no_pref() {
        assert!(PrefKey::ALL.iter().all(|k| k.name() != "claude_integration"));
    }

    #[test]
    fn key_names_are_unique_and_the_terminal_keys_are_machine_only() {
        let mut names: Vec<_> = PrefKey::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PrefKey::ALL.len());
        let machine_only: Vec<_> =
            PrefKey::ALL.iter().filter(|k| !k.board_overridable()).copied().collect();
        assert_eq!(
            machine_only,
            [
                PrefKey::WeekStart,
                PrefKey::StatusTop,
                PrefKey::TabTitle,
                PrefKey::TabTitleNeedsYou,
                PrefKey::TabTitleFocus,
                PrefKey::TabProgress,
                PrefKey::TabTheme,
                PrefKey::TabColor,
                PrefKey::TabSubtitle,
                PrefKey::TabIcon,
                PrefKey::NotifyVia,
                PrefKey::Peek,
                PrefKey::CrownLightning,
                PrefKey::UsageLine,
                PrefKey::UsageFiveHour,
                PrefKey::UsageWeekly,
                PrefKey::UsageModel,
                PrefKey::UsageResets,
                PrefKey::UsageClaude,
                PrefKey::UsageCodex,
                PrefKey::CardCorner,
                PrefKey::Summary,
                PrefKey::NewReplies,
                PrefKey::Mouse,
            ]
        );
    }
}
