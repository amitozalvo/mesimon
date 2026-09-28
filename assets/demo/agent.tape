# The tour's first clip: from a working agent's ticket page, step into the
# agent's own terminal, type to it the ordinary way, and step back to the
# board. Run through record.sh (`record.sh agent`), never alone: it builds
# the sandbox this tape expects in DEMO_* and MESIMON_*. Same terminal as
# demo.tape: theme, size and font. The take in the README is the real
# claude (record.sh with a key file), so every wait on the agent is for
# the shape of its screen, never its words; each also matches the stand-in.

Output agent.gif

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
# The board is up once the agent seed.py started is spinning. The cursor
# goes to its card, in the next column, before the take opens.
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Rate/
Type "l"
Sleep 0.5s
Show

# The board, one agent at work.
Sleep 1.3s

# Its ticket page: the agent's live output, read by the board.
Space
Sleep 1.8s

# Enter steps into the agent itself, full screen: claude at work, or the
# stand-in under the rule that says what it is.
Enter
Wait+Screen@10s /esc to interrupt|stand-in/
Sleep 1s

# Typed the ordinary way. A busy claude queues it and takes it up at its
# next step, however long the step it is on runs, so the wait is cut: the
# take picks up at the agent's answer under the message.
Type "Also log each 429"
Sleep 0.4s
Enter
Sleep 1.5s
Hide
Wait+Screen@120s /(?s)Also log each 429.*⏺|with the client/
Show
Sleep 2.5s

# Step back: the ticket page, then the board, the card still spinning.
Ctrl+]
Wait+Screen@5s /PREVIEW/
Sleep 0.6s
Type "q"
Sleep 1.8s
