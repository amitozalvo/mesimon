# A README tour clip: `/` finds a ticket the screen does not show. Run
# through `record.sh search`, never alone: it builds the sandbox this tape
# expects in DEMO_* and MESIMON_*, and seeds the board seed.py calls SEARCH.

Output search.gif

Set Shell "bash"
Set FontFamily "JetBrains Mono,JetBrainsMono Nerd Font Mono,Menlo,monospace"
Set FontSize 14
# 120 x 30 cells, the demo's size.
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
# The board is up once the agent seed.py started is spinning, and still
# once the header has its memory reading, which lands a few seconds later.
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Rate/
Wait+Screen@20s /GiB/
Show

# Fifteen tickets, and TODO runs off the bottom of the screen.
Sleep 2s

# The picker opens as the whole board; each letter narrows it.
Type "/"
Sleep 1s
Type@400ms "csv"
Sleep 1.4s

# Enter puts the cursor on the card, scrolled into view.
Enter
Sleep 2.5s
