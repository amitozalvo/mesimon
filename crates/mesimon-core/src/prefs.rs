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
    /// The board follows the OS's light/dark appearance while it is open
    /// (T-485): a change moves it to the other slot's theme.
    FollowOs,
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
    NotifyWords,
    NotifySoundNeedsYou,
    NotifySoundDone,
    /// iTerm2 bounces its dock icon once when an agent needs you (T-492).
    /// A notification, so it sits in that list and under that switch.
    NotifyDockBounce,
    /// The board's reply row (T-365): which rung `p`/`P` left it on.
    Peek,
}

impl PrefKey {
    pub const ALL: [PrefKey; 25] = [
        PrefKey::Dark,
        PrefKey::Light,
        PrefKey::FollowOs,
        PrefKey::SnoozeNeedsYou,
        PrefKey::WeekStart,
        PrefKey::MergeTrain,
        PrefKey::MergeTrainNotice,
        PrefKey::StatusTop,
        PrefKey::TabTitle,
        PrefKey::TabTitleNeedsYou,
        PrefKey::TabTitleFocus,
        PrefKey::TabProgress,
        PrefKey::TabColor,
        PrefKey::TabSubtitle,
        PrefKey::TabIcon,
        PrefKey::KeepAwake,
        PrefKey::Notify,
        PrefKey::NotifyDone,
        PrefKey::NotifyFocused,
        PrefKey::NotifyInPane,
        PrefKey::NotifyWords,
        PrefKey::NotifySoundNeedsYou,
        PrefKey::NotifySoundDone,
        PrefKey::NotifyDockBounce,
        PrefKey::Peek,
    ];

    /// The JSON key in both files.
    pub const fn name(self) -> &'static str {
        match self {
            PrefKey::Dark => "dark",
            PrefKey::Light => "light",
            PrefKey::FollowOs => "follow_os",
            PrefKey::SnoozeNeedsYou => "snooze_needs_you",
            PrefKey::WeekStart => "week_start",
            PrefKey::MergeTrain => "merge_train",
            PrefKey::MergeTrainNotice => "merge_train_notice",
            PrefKey::StatusTop => "status_line_top",
            PrefKey::TabTitle => "tab_title",
            PrefKey::TabTitleNeedsYou => "tab_title_needs_you",
            PrefKey::TabTitleFocus => "tab_title_focus",
            PrefKey::TabProgress => "tab_progress",
            PrefKey::TabColor => "tab_color",
            PrefKey::TabSubtitle => "tab_subtitle",
            PrefKey::TabIcon => "tab_icon",
            PrefKey::KeepAwake => "keep_awake",
            PrefKey::Notify => "notify",
            PrefKey::NotifyDone => "notify_done",
            PrefKey::NotifyFocused => "notify_focused",
            PrefKey::NotifyInPane => "notify_in_pane",
            PrefKey::NotifyWords => "notify_words",
            PrefKey::NotifySoundNeedsYou => "notify_sound_needs_you",
            PrefKey::NotifySoundDone => "notify_sound_done",
            PrefKey::NotifyDockBounce => "notify_dock_bounce",
            PrefKey::Peek => "peek",
        }
    }

    /// May one board set this on its own? The machine keeps: where the
    /// tmux status line sits and the seven terminal integrations (T-492),
    /// which are about the terminal the board runs in and not about a
    /// repo; which day a week starts on, which is about the person; and
    /// the reply row's rung (T-365), which is about how the person reads a
    /// board and has no Settings row to set it per board — `p` and `P` set
    /// it. The dock bounce is a notification and follows its group.
    pub fn board_overridable(self) -> bool {
        !matches!(
            self,
            PrefKey::StatusTop
                | PrefKey::TabTitle
                | PrefKey::TabTitleNeedsYou
                | PrefKey::TabTitleFocus
                | PrefKey::TabProgress
                | PrefKey::TabColor
                | PrefKey::TabSubtitle
                | PrefKey::TabIcon
                | PrefKey::WeekStart
                | PrefKey::Peek
        )
    }

    /// The words a status line calls it.
    pub fn label(self) -> &'static str {
        match self {
            PrefKey::Dark => "dark theme",
            PrefKey::Light => "light theme",
            PrefKey::FollowOs => "follow the OS appearance",
            PrefKey::SnoozeNeedsYou => "snooze needs-you",
            PrefKey::WeekStart => "week start",
            PrefKey::MergeTrain => "auto merge",
            PrefKey::MergeTrainNotice => "auto merge notice",
            PrefKey::StatusTop => "status line",
            PrefKey::TabTitle => "terminal tab title",
            PrefKey::TabTitleNeedsYou => "tab title counts needs-you",
            PrefKey::TabTitleFocus => "tab title follows the open session",
            PrefKey::TabProgress => "tab progress ring",
            PrefKey::TabColor => "tab colour when needs you",
            PrefKey::TabSubtitle => "tab subtitle",
            PrefKey::TabIcon => "tab icon",
            PrefKey::KeepAwake => "keep awake",
            PrefKey::Notify => "notifications",
            PrefKey::NotifyDone => "notify when a turn lands",
            PrefKey::NotifyFocused => "notify while focused",
            PrefKey::NotifyInPane => "notify in the pane",
            PrefKey::NotifyWords => "notify with the agent's words",
            PrefKey::NotifySoundNeedsYou => "needs-you sound",
            PrefKey::NotifySoundDone => "done sound",
            PrefKey::NotifyDockBounce => "dock bounce",
            PrefKey::Peek => "replies",
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
                | PrefKey::Peek
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                PrefKey::TabColor,
                PrefKey::TabSubtitle,
                PrefKey::TabIcon,
                PrefKey::Peek
            ]
        );
    }
}
