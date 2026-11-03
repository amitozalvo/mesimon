# The Reddit clip: one take from the crown to the merge, all of it on the
# board. An agent has read the board and said what 1.0 is missing; ^o
# crowns it, Shift+Enter on its card tells it what to do, and it files two
# tickets, tags one and starts an agent on the demo ticket. That agent
# works, raises its hand, and is answered with Shift+Enter on its card; the
# merge train lands its branch and DONE takes the card. The peek is on for
# every card, so each agent's latest words sit under its title.
# Run through record.sh, never alone:
#
#   assets/demo/record.sh reddit
#
# Larger cells than the README clips (Reddit plays it in a feed, often on
# a phone), the same 120 x 30 grid, and longer holds: a feed viewer reads
# one thing per beat. The mp4 is what Reddit's native video upload takes;
# the gif is for a comment or a sub that takes images only.

Output reddit.mp4
Output reddit.gif

Set Shell "bash"
Set FontFamily "JetBrains Mono,JetBrainsMono Nerd Font Mono,Menlo,monospace"
Set FontSize 18
# 120 x 30 cells at 18 px: the README's 1132 x 590 at 14 px, scaled.
Set Width 1450
Set Height 752
Set Padding 12
Set Framerate 24
Set TypingSpeed 70ms
Set CursorBlink false
Set Theme { "name": "graphite", "background": "#131417", "foreground": "#E9E7E1", "cursor": "#F1EFE9", "selection": "#272B31", "black": "#131417", "red": "#D5809A", "green": "#6FBFB0", "yellow": "#F0A93A", "blue": "#46ACFC", "magenta": "#D182DA", "cyan": "#00BCB9", "white": "#E9E7E1", "brightBlack": "#5E5B55", "brightRed": "#F47A7A", "brightGreen": "#7EB850", "brightYellow": "#BAA601", "brightBlue": "#00B8D8", "brightMagenta": "#9697FF", "brightCyan": "#20C188", "brightWhite": "#F1EFE9" }

Hide
Type 'cd "$DEMO_REPO" && clear && exec python3 "$DEMO_TERM" "$DEMO_MESIMON"'
Enter
# The board is up once the crown's card, idle in REVIEW, shows its words.
Wait+Screen@20s /1\.0 is missing a health/
# The cursor onto its card: right past the empty IN PROGRESS to REVIEW.
Type "l"
Sleep 0.3s
Type "l"
Sleep 0.5s
Show

# An agent that read the board and said what 1.0 is missing.
Sleep 4s

# ^o crowns its ticket; the status line says what the crown may do.
Ctrl+O
Sleep 3s

# Shift+Enter on its card tells the crown what to do.
Alt+Enter
Sleep 1.5s
Type "File what 1.0 is missing, then start Stats as JSON"
Sleep 1.2s
Enter

# The crowned agent runs the board: two tickets filed, one tagged, and an
# agent started on the demo ticket, whose card spins.
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Stats as JSON/
Sleep 4.5s

# That agent works until it raises its hand: its card lights with the
# question at the top of REVIEW, above the crown's card, where the cursor
# followed the crown.
Wait+Screen@30s /REVIEW +3/
Sleep 1.5s
Type "k"
Sleep 4.5s

# Shift+Enter on the card answers it from the board.
Alt+Enter
Sleep 1.5s
Type "Yes, include them"
Sleep 1s
Enter
Sleep 4s

# It finishes, the merge train lands its branch, and DONE takes the card.
Wait+Screen@40s /DONE +2/
Sleep 5s
