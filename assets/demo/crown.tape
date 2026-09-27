# Tour clip, the crown: crown a working agent's ticket with ^o and watch
# its agent run the board -- file two tickets, move one, tag one, start an
# agent on another. Run through record.sh, never alone:
#
#   assets/demo/record.sh crown
#
# stub-claude.py is the crowned agent; every edit is its call through the
# real `mesimon mcp` shim, and each card it touches lights on its own.

Output crown.gif

Set Shell "bash"
Set FontFamily "JetBrains Mono,JetBrainsMono Nerd Font Mono,Menlo,monospace"
Set FontSize 14
# 120 x 30 cells, the width the board's goldens are drawn at.
Set Width 1132
Set Height 590
Set Padding 12
Set Framerate 24
Set TypingSpeed 55ms
Set CursorBlink false
Set Theme { "name": "graphite", "background": "#131417", "foreground": "#E9E7E1", "cursor": "#F1EFE9", "selection": "#272B31", "black": "#131417", "red": "#D5809A", "green": "#6FBFB0", "yellow": "#F0A93A", "blue": "#46ACFC", "magenta": "#D182DA", "cyan": "#00BCB9", "white": "#E9E7E1", "brightBlack": "#5E5B55", "brightRed": "#F47A7A", "brightGreen": "#7EB850", "brightYellow": "#BAA601", "brightBlue": "#00B8D8", "brightMagenta": "#9697FF", "brightCyan": "#20C188", "brightWhite": "#F1EFE9" }

Hide
Type 'cd "$DEMO_REPO" && clear && exec python3 "$DEMO_TERM" "$DEMO_MESIMON"'
Enter
# The board is up once the agent seed.py started is spinning.
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Plan 1.0/
# The cursor onto its card, in IN PROGRESS.
Type "l"
Sleep 0.5s
Show

# One agent at work, planning the release.
Sleep 1.5s

# ^o crowns its ticket.
Ctrl+O

# The crowned agent runs the board: two tickets filed, one moved back,
# one tagged, and an agent started on a fourth, whose card spins.
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Expire old/
Sleep 2s

# Its own words on the ticket page: what it did, and the starts left.
Space
Wait+Screen@10s /starts? left/
Sleep 3s
