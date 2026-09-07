# The mesimon shin

The three uneven prongs borrow the silhouette of ש. Two small eyes make it a
creature. The detached amber tip means an actual needs-you event; a completed
turn uses the resting drawing.

- `resting.txt`: the 24-column, 12-row installer drawing.
- `compact.txt`: the 20-column, 7-row preview drawing, tuned for terminal cells.
- `resting.svg` / `needs-you.svg`: filled vector drawings on a graphite tile.
- `resting.png` / `needs-you.png`: 256px notification assets embedded in the binary.

Run `python3 -B ci/mascot.py` to regenerate all assets and the installer's
embedded drawing. `python3 -B ci/mascot.py --check` checks for drift. The PNG
renderer uses the same filled geometry as the SVG, with antialiasing and a
transparent exterior; no rasterizer or runtime dependency is needed.

The preview uses its theme's `dim1`, with no animation or attention color. It
gives space back to the instructions when the full drawing cannot fit. Mono
gets the wordmark. The installer draws only after success, to a UTF-8 TTY;
pipes and dumb terminals get its normal text.

Linux uses `notify-send --icon`. On macOS, Mesimon copies an installed
terminal-notifier app bundle into its own notification state directory, sets the
Mesimon name, bundle identity and ICNS icon, and signs and verifies the copy.
The installed helper stays untouched. macOS can ask for notification permission
for **Mesimon** separately from terminal-notifier.

The application icon is the stable resting mascot. Needs-you posts also carry
the amber mascot as a `-contentImage` attachment. An unidentified helper gets an
attachment on either kind of post. `-appIcon` is never used: accepting the option
in terminal-notifier 2 did not make its private API work on macOS 26. The separate
bundle was confirmed visually by the user on macOS 26.6.2.
The osascript, OSC and custom-helper paths retain their existing interfaces.

The shared home is `~/.local/state/mesimon/notifications/`. PNGs and app bundles
are materialized only for an actual image-capable banner; discovery, doctor,
sound previews and disabled notifications never write them. Each app generation
is keyed by the helper's complete contents and the icon, prepared in isolation,
and published only after signature verification. Concurrent boards reuse one
generation; an upgrade does not replace an executable handling an older click.
Setup commands have a ten-second deadline and are reaped. Setup failure leaves
the normal notification path available and reports the missing app icon.
