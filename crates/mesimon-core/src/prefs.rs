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
    KeepAwake,
    Notify,
    NotifyDone,
    NotifyFocused,
    NotifyInPane,
    NotifyWords,
    NotifySoundNeedsYou,
    NotifySoundDone,
    /// The board's reply row (T-365): which rung `p`/`P` left it on.
    Peek,
}

impl PrefKey {
    pub const ALL: [PrefKey; 16] = [
        PrefKey::Dark,
        PrefKey::Light,
        PrefKey::SnoozeNeedsYou,
        PrefKey::WeekStart,
        PrefKey::MergeTrain,
        PrefKey::MergeTrainNotice,
        PrefKey::StatusTop,
        PrefKey::KeepAwake,
        PrefKey::Notify,
        PrefKey::NotifyDone,
        PrefKey::NotifyFocused,
        PrefKey::NotifyInPane,
        PrefKey::NotifyWords,
        PrefKey::NotifySoundNeedsYou,
        PrefKey::NotifySoundDone,
        PrefKey::Peek,
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
            PrefKey::KeepAwake => "keep_awake",
            PrefKey::Notify => "notify",
            PrefKey::NotifyDone => "notify_done",
            PrefKey::NotifyFocused => "notify_focused",
            PrefKey::NotifyInPane => "notify_in_pane",
            PrefKey::NotifyWords => "notify_words",
            PrefKey::NotifySoundNeedsYou => "notify_sound_needs_you",
            PrefKey::NotifySoundDone => "notify_sound_done",
            PrefKey::Peek => "peek",
        }
    }

    /// May one board set this on its own? Three stay the machine's: where
    /// the tmux status line sits is about the terminal, which day a week
    /// starts on is about the person, and neither changes with the repo;
    /// the reply row's rung (T-365) is about how the person reads a board,
    /// and no Settings row exists to set it per board — `p` and `P` set it.
    pub fn board_overridable(self) -> bool {
        !matches!(self, PrefKey::StatusTop | PrefKey::WeekStart | PrefKey::Peek)
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
            PrefKey::KeepAwake => "keep awake",
            PrefKey::Notify => "notifications",
            PrefKey::NotifyDone => "notify when a turn lands",
            PrefKey::NotifyFocused => "notify while focused",
            PrefKey::NotifyInPane => "notify in the pane",
            PrefKey::NotifyWords => "notify with the agent's words",
            PrefKey::NotifySoundNeedsYou => "needs-you sound",
            PrefKey::NotifySoundDone => "done sound",
            PrefKey::Peek => "replies",
        }
    }

    /// A plain switch, as opposed to a named value (a theme, a sound, a day).
    pub fn is_bool(self) -> bool {
        !matches!(
            self,
            PrefKey::Dark
                | PrefKey::Light
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
    fn key_names_are_unique_and_three_are_machine_only() {
        let mut names: Vec<_> = PrefKey::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PrefKey::ALL.len());
        let machine_only: Vec<_> =
            PrefKey::ALL.iter().filter(|k| !k.board_overridable()).copied().collect();
        assert_eq!(machine_only, [PrefKey::WeekStart, PrefKey::StatusTop, PrefKey::Peek]);
    }
}
