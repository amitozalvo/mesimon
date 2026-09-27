# The README demo: start an agent from the board, watch it work, answer it
# from its ticket page when its card lights, merge its branch. Run through record.sh, never
# alone: it builds the sandbox this tape expects in DEMO_* and MESIMON_*.
#
# Alt+Enter below is Shift+Enter: kitty-term.py delivers it as the kitty
# report a real terminal sends (see that file for why).

Output ../demo.gif

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

# The board, one agent already at work.
Sleep 2.5s

# Shift+Enter on the first ticket: start an agent and ask it. A blank
# Enter sends the ticket's own title and description.
Alt+Enter
Sleep 1.5s
Enter

# It works: the card moves to IN PROGRESS and spins, until the agent
# raises its hand. The card lights with the question and waits in REVIEW.
Wait+Screen@30s /REVIEW +2/
Sleep 2.5s

# The ticket page: the agent's question, its words, and the shin.
Space
Sleep 3s

# Answer from the page with Shift+Enter; the page shows the agent at work.
Alt+Enter
Sleep 0.6s
Type "Yes, include them"
Sleep 0.6s
Enter

# Done, and the branch line offers the merge.
Wait+Screen@30s /1 to merge ∙ m merge/
Sleep 2s
Type "m"
Sleep 1.5s
Type "m"
Wait+Screen@20s /merged into main/
Sleep 2s
Escape
Sleep 0.5s
Type "q"
Sleep 0.8s

# Merged, so DONE takes it.
Type "L"
Sleep 3s
