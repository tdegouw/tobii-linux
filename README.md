# tobii-linux

Use the Tobii Eye Tracker 5 (USB `2104:0313`, "EyeChip", IS5) on Linux — plain Rust + libusb,
no Nix, no Windows runtime, no kernel module.

Protocol knowledge comes from [nottobii](https://github.com/puffnfresh/nottobii) and
[tobiifree](https://github.com/Aetherall/tobiifree) (both GPL-3). This project is GPL-3.0-or-later.

## Layout

```
tobii-et5/   library: rusb transport, TLV framing, init/auth/subscribe, gaze parsing,
             frequency queries, calibration commands (calibration.rs, untested)
et5-cli/     command-line tool
et5-calibrate/ fullscreen Wayland (layer-shell) calibration overlay
et5-cursor/  gaze cursor daemon (uinput absolute pointer) + toggle client
systemd/     et5-cursor.service (user unit)
udev/        70-tobii-et5.rules (USB + /dev/uinput access for the logged-in user)
```

## Setup (Arch)

```sh
sudo pacman -S --needed libusb pkgconf          # build deps
curl https://sh.rustup.rs | sh -s -- -y --no-modify-path && source ~/.cargo/env
sudo cp udev/70-tobii-et5.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger   # then replug the tracker
```

## Build & run

```sh
cargo build --release
target/release/et5-cli info          # serial, model, firmware, frequency
target/release/et5-cli stream 10     # print samples + measured rate
target/release/et5-cli freq list     # supported frequency modes
target/release/et5-cli -v ...        # log every request/response status
ET5_DEBUG=1 target/release/et5-cli stream 2   # log every USB transfer
```

## Configuration

`~/.config/tobii-et5/config.toml` is created on first run. Default is a 49" 32:9
(5120x1440, ~1193x336 mm) with the tracker centred under the bottom edge — measure
your screen and adjust `width_mm`, `height_mm`, `bottom_offset_mm`, `tracker_x_offset_mm`.
A calibration blob at `~/.config/tobii-et5/<serial>/calibration.bin` is uploaded on init if present.

## Calibration

```sh
cargo build --release && target/release/et5-calibrate
```

Sit normally (~60–80 cm). A dark overlay covers the screen: a pulsing dot while the tracker
connects (a few seconds), a 3 s countdown (closing ring + dots), then 7 blue dots one after
another (centre, then two rounds of three). Look at the violet centre of each dot until it has
shrunk (1.5 s). A green ring = saved, red = failed (details on stderr). Esc or q aborts.
`--dry-run` shows the animation without touching the tracker.

## Gaze cursor

```sh
install -m755 target/release/et5-{cli,calibrate,cursor} ~/.local/bin/
cp systemd/et5-cursor.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now et5-cursor
```

The daemon starts **off**. `et5-cursor toggle|on|off|status` talks to it over
`$XDG_RUNTIME_DIR/et5-cursor.sock` and shows a notification ("Oogbesturing aan/uit").
Hyprland/Omarchy binding (`~/.config/hypr/bindings.lua`):

```lua
o.bind("SUPER + ALT + E", "Oogbesturing aan/uit", "/home/ted/.local/bin/et5-cursor toggle")
```

Behaviour: one-euro filter per axis → fixation dead zone (cursor stays put while you
look within `deadzone_px` of it) → eases toward the gaze (`follow`) or jumps on a saccade
(`saccade_px`). No eyes detected = cursor frozen. Tracker unplugged/silent = reconnect
every 3 s. Only moves the pointer; clicking stays on the mouse.

Tuning: `~/.config/tobii-et5/cursor.toml` (created on first start), then
`systemctl --user restart et5-cursor`:

| key | default | effect |
|---|---|---|
| `min_cutoff` | 0.6 | lower = smoother but laggier when still |
| `beta` | 0.004 | higher = less lag on fast movements |
| `deadzone_px` | 60 | bigger = steadier cursor, needs larger gaze shift to move |
| `follow` | 0.5 | fraction of the distance moved per sample (33 Hz) |
| `saccade_px` | 600 | jumps beyond this snap immediately |

## Findings (firmware on IS50F)

- Gaze streams **without** a calibration blob (an empty blob is rejected, `0x20000503`).
- Output rate is **~33 Hz**, one 1724-byte message per USB transfer (no batching).
  FREQUENCIES_GET/LIST report a single mode `{132, 33}` — 132 Hz camera, 33 Hz gaze output.
  No setter is known; probing `0x067C` resets the tracker.
- Inbound frames: `LE32 id, LE32 total_len (incl. 8-byte header)`; outbound: `LE32 0, LE32 content_len`.
- Responses carry `0x00000001` in the upper half of the property field.
- AUTH_VERIFY/MOUNTING_CONFIG answer `0x20000505`; streaming still works.
- Roughly every other session misbehaves (silent gaze stream, or an unanswered request
  such as PAUSE_SET); reopening fixes it (`Device::open_ready` / `open_streaming` retry).

## Roadmap

1. ~~`et5-calibrate`~~ done; validate accuracy after a real calibration.
2. ~~gaze cursor~~ done (`et5-cursor`).
3. Optional: gaze + key click, per-monitor mapping, udev-triggered service start.
