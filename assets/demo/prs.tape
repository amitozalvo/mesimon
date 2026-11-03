# The second Reddit clip: a board that merges through pull requests, with a
# column for each environment, running a sprint on its own. Ten cards, two
# workers the crown started, a stand-in GitHub (forge.py) behind it. One
# worker opens a pull request and the board follows it: IN PROGRESS until
# its pipeline passes, REVIEW until a teammate merges it, STAGING and
# PRODUCTION as the merge reaches each. The other worker's pipeline fails;
# the crown hears it, sends the worker back, and the fix goes the same way.
# The person types nothing until the end: ^h on a card lists its pull
# request, its pipeline and where it is deployed. The peek is on, so each
# agent's latest words sit under its title.
# Run through record.sh, never alone:
#
#   assets/demo/record.sh prs
#
# The same cells and holds as reddit.tape.

Output prs.mp4
Output prs.gif

Set Shell "bash"
Set FontFamily "JetBrains Mono,JetBrainsMono Nerd Font Mono,Menlo,monospace"
Set FontSize 18
Set Width 1450
Set Height 752
Set Padding 12
Set Framerate 24
Set TypingSpeed 70ms
Set CursorBlink false
Set Theme { "name": "graphite", "background": "#131417", "foreground": "#E9E7E1", "cursor": "#F1EFE9", "selection": "#272B31", "black": "#131417", "red": "#D5809A", "green": "#6FBFB0", "yellow": "#F0A93A", "blue": "#46ACFC", "magenta": "#D182DA", "cyan": "#00BCB9", "white": "#E9E7E1", "brightBlack": "#5E5B55", "brightRed": "#F47A7A", "brightGreen": "#7EB850", "brightYellow": "#BAA601", "brightBlue": "#00B8D8", "brightWhite": "#F1EFE9", "brightMagenta": "#9697FF", "brightCyan": "#20C188" }

Hide
Type 'cd "$DEMO_REPO" && clear && exec python3 "$DEMO_TERM" "$DEMO_MESIMON"'
Enter
# The board is up once the crown's card is drawn.
Wait+Screen@20s /♛ Ship 1\.1/
Show

# Two workers at work, a pipeline running, cards in every environment.
Sleep 5s

# A worker opens its pull request; the pipeline passes; REVIEW takes it.
Wait+Screen@40s /REVIEW +[2-9]/
Sleep 3s

# The other worker's pipeline fails; the crown sends it back.
Wait+Screen@40s /Sent T-/
Sleep 4s

# A teammate merges the first one; the deploys carry it to STAGING, then
# PRODUCTION, while the fix is pushed and goes green.
Wait+Screen@60s /PRODUCTION +[4-9]/
Sleep 3s

# The fix lands too.
Wait+Screen@60s /PRODUCTION +[6-9]/
Sleep 2s

# ^h on the card that just arrived lists its pull request, pipeline and
# deployments.
Type "l"
Sleep 0.3s
Type "l"
Sleep 0.3s
Type "l"
Sleep 0.3s
Type "l"
Sleep 1s
Ctrl+H
Sleep 5s
Escape
Sleep 3s
