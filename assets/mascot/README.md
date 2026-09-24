# The mesimon shin

The mascot is the letter ש as a creature (T-451): the middle arm grows out of
the left arm, as in the printed letter, and each arm ends in the letter's head
stroke. It is pixel art drawn in terminal half blocks, two square pixels to a
cell.

- `shin.txt`: the one source of pixels. Three drawings (small, medium, large)
  with the face's anchors marked in the grid; the file's header says how.
- `resting.png` / `needs-you.png`: 256px notification assets embedded in the
  binary. Both are the medium shin on a graphite tile: resting is the empty
  seat's face, and needs-you is the pose the ticket page opens it on, with the
  amber "!" in pixels beside the waving arm.

`crates/mesimon-tui/src/creature.rs` is the engine: it parses `shin.txt`,
stamps a face over the anchors, places props around the body, and plays one
timeline per state. The PNGs and the installer's embedded welcome are goldens
of that engine. Remint them with
`MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui creature`; a plain
`cargo ut` fails when they drift from the pixels.

On the ticket page the medium shin stands over the empty seat's words and over
a session with nothing to read, acting out its state; beside a reply, the small
shin stands in the zone's top-right corner on a zone at least 72 columns wide.
The body is the theme's greyscale ramp, the blush and the props borrow the tag
ring's tints, and only the needs-you "!" wears `attn`. Below truecolor there is
no blush; mono draws no picture and keeps the wordmark. The installer draws the
large shin in two tones (it writes no escapes), only after success, to a UTF-8
TTY; pipes and dumb terminals get its normal text.

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
