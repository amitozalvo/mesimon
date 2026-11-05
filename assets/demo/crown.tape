# The crown clip, the README's first: file a ticket in TODO with its brief,
# crown it with ^o, start its agent with Shift+Enter, and watch that agent
# run the board -- file two tickets, tag one, start an agent on another.
# Run through record.sh, never alone:
#
#   assets/demo/record.sh crown
#
# stub-claude.py is the crowned agent; every edit is its call through the
# real `mesimon mcp` shim, and each card it touches lights on its own.
#
# Alt+Enter below is Shift+Enter: kitty-term.py delivers it as the kitty
# report a real terminal sends (see that file for why).

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
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Rate/
Show

# The board: three tickets in TODO, one agent at work.
Sleep 2s

# `o` writes a new ticket; `tab` opens the editor for its brief, `^s`
# keeps the brief, and Enter files the ticket in TODO.
Type "o"
Sleep 0.6s
Type "Plan 1.0"
Sleep 0.6s
Tab
Sleep 0.6s
Type@35ms "File what 1.0 is missing, then start the next feature."
Sleep 1s
Ctrl+S
Sleep 1.2s
Enter
Sleep 1.5s

# ^o crowns it before its agent starts.
Ctrl+O
Sleep 2s

# Shift+Enter starts its agent; a blank Enter sends the title and brief.
Alt+Enter
Sleep 1s
Enter

# The crowned agent runs the board: two tickets filed, one tagged, and an
# agent started on another, whose card spins.
Wait+Screen@30s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Expire old/
Sleep 1s

# Its turn ends: its card goes to REVIEW and says what it did.
Wait+Screen@20s /Filed T-/
Wait+Screen@20s /REVIEW +2/
Sleep 4.5s
