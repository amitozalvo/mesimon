# The ticket page: describe a card with Tab, open its page, read the note
# its agent left, and follow a link from it with ^k. Run through
# `record.sh ticket-page`, never alone: seed.py has already run the agent
# that left the note, and record.sh points the link opener at `true`, so the
# last Enter says `opening …` and starts no browser.

Output ticket-page.gif

Set Shell "bash"
Set FontFamily "JetBrains Mono,JetBrainsMono Nerd Font Mono,Menlo,monospace"
Set FontSize 14
# 120 x 30 cells, the width the board's goldens are drawn at.
Set Width 1132
Set Height 590
Set Padding 12
Set Framerate 24
Set TypingSpeed 30ms
Set CursorBlink false
Set Theme { "name": "graphite", "background": "#131417", "foreground": "#E9E7E1", "cursor": "#F1EFE9", "selection": "#272B31", "black": "#131417", "red": "#D5809A", "green": "#6FBFB0", "yellow": "#F0A93A", "blue": "#46ACFC", "magenta": "#D182DA", "cyan": "#00BCB9", "white": "#E9E7E1", "brightBlack": "#5E5B55", "brightRed": "#F47A7A", "brightGreen": "#7EB850", "brightYellow": "#BAA601", "brightBlue": "#00B8D8", "brightMagenta": "#9697FF", "brightCyan": "#20C188", "brightWhite": "#F1EFE9" }

Hide
Type 'cd "$DEMO_REPO" && clear && exec python3 "$DEMO_TERM" "$DEMO_MESIMON"'
Enter
# The board is up once the agent seed.py started is spinning.
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Rate/
Show

# The board, the cursor on a ticket whose agent has already been at it.
Sleep 0.9s

# Tab: the card grows into its description. Two lines of what to do, under
# the one already there, and ^s keeps them.
Tab
Sleep 0.6s
Down
Enter
Type "- Count from the last click, not creation."
Enter
Type "- Free its slug and log it."
Sleep 0.4s
Ctrl+S
Sleep 0.6s

# Enter on a quiet ticket is its page: the description, the agent's last
# words beside the shin, and the NOTES rail with the agent's note.
Enter
Sleep 1.7s

# Down the rail, past the session and the description, to that note.
Type "j"
Sleep 0.3s
Type "j"
Sleep 1.3s

# ^k lists what the ticket's words point at: a file in the repo, a URL.
Ctrl+K
Sleep 1s
Down
Sleep 0.4s
Enter
Sleep 1.2s
