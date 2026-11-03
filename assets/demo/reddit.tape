# The Reddit clip: you and the crown on one board, one take from a start to
# the merge, all of it on the board. You start Stats as JSON yourself,
# Shift+Enter on its card with your words. An agent that read the board
# sits idle in REVIEW with what 1.0 is missing; ^o crowns it and
# Shift+Enter tells it what to do, and it files two tickets, tags one and
# starts an agent on Custom slugs. Your worker raises its hand, its card
# lights, and you answer from the card; the merge train lands it. The
# crown's worker asks the crown, and the crown answers it on its own; the
# train lands that too. The peek is on for every card, so each agent's
# latest words sit under its title. Run through record.sh, never alone:
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
# The board is up once the idle agent's words are on its card in REVIEW.
# The cursor starts on the first TODO card, Stats as JSON.
Wait+Screen@20s /1\.0 is missing a health/
Show

# A board with one agent that has read it.
Sleep 3s

# Shift+Enter on a card with no agent starts one with your words.
Alt+Enter
Sleep 1.5s
Type "Add --json to shortlink stats"
Sleep 1.2s
Enter
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Stats as JSON/
Sleep 3s

# The cursor followed the card into IN PROGRESS; one step right is the
# idle agent at the top of REVIEW. ^o crowns it; the status line says
# what the crown may do.
Type "l"
Sleep 1s
Ctrl+O
Sleep 3s

# Shift+Enter on its card tells the crown what to do.
Alt+Enter
Sleep 1.5s
Type "File what 1.0 is missing, then start Custom slugs"
Sleep 1.2s
Enter

# The crowned agent runs the board: two tickets filed, one tagged, and an
# agent started on Custom slugs, whose card spins. Its own card went to
# IN PROGRESS for the turn and comes back to the top of REVIEW; the cursor
# follows it.
Wait+Screen@20s /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Custom slugs/
Sleep 3s

# Your worker raises its hand: its card lights with the question at the
# top of REVIEW (the column's third card), above the crown's. k onto it;
# Shift+Enter answers from the card.
Wait+Screen@40s /REVIEW +[3-9]/
Sleep 2s
Type "k"
Sleep 3s
Alt+Enter
Sleep 1.5s
Type "Yes, include them"
Sleep 1s
Enter
Sleep 3s

# It finishes and the merge train lands its branch. Then the crown's
# worker asks the crown, and the crown answers it on its own.
Wait+Screen@60s /Answered T-/
Sleep 4s

# The train lands that branch too, DONE holds both, and the crown parks
# its worker.
Wait+Screen@60s /DONE +[3-9]/
Wait+Screen@20s /merged and parked/
Sleep 5s
