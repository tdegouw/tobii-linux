# tobii-linux

Use the **Tobii Eye Tracker 5** (USB `2104:0313`, "EyeChip") on Linux with plain Rust and
libusb. You don't need Nix, the Windows runtime, or a kernel module.

## Features

- Userspace driver library (`tobii-et5`): init and auth, gaze stream (~33 Hz), calibration,
  automatic recovery from the device's every-other-session quirk.
- `et5-cli`: device info, live gaze stream with rate and pixel coordinates.
- `et5-calibrate`: full-screen Wayland calibration (7 points), saved per tracker serial.
- `et5-cursor`: background daemon with two modes:
  - **focus** (default): the window you look at gets keyboard focus; the mouse pointer is
    left alone (Hyprland).
  - **cursor**: gaze moves the pointer through a virtual uinput device.
- Toggle hotkey with notification, systemd user service, very low CPU (~0.2 % of one core).

See [ARCHITECTURE.md](ARCHITECTURE.md) for how it works.

## Requirements

- Tobii Eye Tracker 5 (tested: IS50F, firmware `02a1a6a977`)
- Linux with libusb 1.0, a Wayland compositor with layer-shell for calibration,
  Hyprland for focus mode (tested 0.56, Lua config / Omarchy)
- Rust toolchain (rustup), `pkgconf`, `notify-send` (libnotify) for notifications

## Install

```sh
# 1. toolchain + build deps (Arch)
sudo pacman -S --needed libusb pkgconf libnotify
curl https://sh.rustup.rs | sh -s -- -y --no-modify-path && source ~/.cargo/env

# 2. device access for your user (tracker + /dev/uinput)
sudo cp udev/70-tobii-et5.rules /etc/udev/rules.d/
sudo modprobe uinput
sudo udevadm control --reload && sudo udevadm trigger   # then replug the tracker

# 3. build and install
cargo build --release
install -m755 target/release/et5-{cli,calibrate,cursor} ~/.local/bin/

# 4. service
cp systemd/et5-cursor.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now et5-cursor
```

**Hotkey** (Omarchy, `~/.config/hypr/bindings.lua`):

```lua
o.bind("SUPER + ALT + E", "Oogfocus aan/uit", "/home/<you>/.local/bin/et5-cursor toggle")
```

## Usage

```sh
et5-cli info                 # serial, model, firmware, frequency, calibration loaded?
et5-cli stream 10            # samples, pixel coordinates, measured Hz
et5-cli freq list            # supported frequency modes
et5-cli -v …                 # log request/response status;  ET5_DEBUG=1 logs USB transfers

et5-calibrate                # calibrate (sit normally, follow the dots); Esc/q aborts
et5-calibrate --dry-run      # animation only, no tracker

et5-cursor toggle|on|off     # control the daemon (notification "Oogfocus aan/uit")
et5-cursor mode focus|cursor # switch mode at runtime
et5-cursor status            # e.g. "on focus"
et5-cursor hypr-debug        # what focus mode sees (read-only)
```

What you see during calibration: a dark overlay, a pulsing dot while connecting, a 3 s
countdown, then 7 shrinking blue dots (centre, then two rounds of three). Look at each violet
centre until it is small. A green ring means saved; red means failed (see terminal output).

The daemon starts **OFF** after every login; press the hotkey to enable.

## Configuration

`~/.config/tobii-et5/config.toml`: display geometry (default: 49" 32:9, 5120×1440,
1193×336 mm, tracker centred 10 mm below the screen).

`~/.config/tobii-et5/cursor.toml` (restart the service after editing):

| key | default | effect |
|---|---|---|
| `mode` | `"focus"` | `"focus"` or `"cursor"` |
| `focus.dwell_ms` | 200 | how long you must look at another window before it gets focus |
| `focus.majority` | 0.7 | share of gaze samples during the dwell that must be on the new window (blinks/outliers tolerated) |
| `focus.max_gap_ms` | 100 | drop the candidate window after this long without a sample on it |
| `focus.smooth` | 0.7 | hit-test smoothing (1 = raw gaze, lower = steadier but slower) |
| `focus.saccade_px` | 300 | jumps larger than this skip the smoothing |
| `focus.log_latency` | true | log `dwell / ipc / gaze-to-focus` per switch to the journal |
| `focus.margin_px` | 80 | gaze must be this far inside the window (hysteresis) |
| `focus.mouse_quiet_ms` | 1000 | ignore gaze this long after the mouse moved |
| `filter.min_cutoff` | 0.6 | lower = smoother but laggier when still |
| `filter.beta` | 0.004 | higher = less lag on fast eye movements |
| `filter.deadzone_px` | 60 | cursor mode: radius in which the pointer stays put |
| `filter.follow` | 0.5 | cursor mode: fraction of the distance moved per sample |
| `filter.saccade_px` | 600 | jumps beyond this reset the filter / snap |

Values out of range are clamped and logged.

## Troubleshooting

- **Logs:** `journalctl --user -u et5-cursor -f`. Expect `tracker IS50F-… streaming`.
- **`no Tobii ET5 found` / kernel `device descriptor read error`:** unplug and replug the
  tracker. The daemon reconnects by itself within ~3 s.
- **`usb: Access denied`:** the udev rule is not active. Check
  `getfacl /dev/bus/usb/*/*` for your user, then replug.
- **Gaze is offset:** fix `config.toml` geometry, then run `et5-calibrate` again.
- **Focus doesn't switch:** check `et5-cursor status` (must be `on focus`), keep the mouse
  still for a second, and look well inside the window; `et5-cursor hypr-debug` shows
  `blocked true` when a fullscreen window, popup or launcher is active.
- **Only ~33 Hz:** that is the device's output rate (see Known limits in ARCHITECTURE.md).

## AI Disclaimer

Most of this code was written by an AI assistant (Grok Bot) under the repository owner's direction,
based on the reverse-engineering work of nottobii and tobiifree, and reviewed and tested by
the owner on their own hardware. It is not affiliated with Tobii, it is experimental, and you use it
at your own risk. See [AI_DISCLAIMER.md](AI_DISCLAIMER.md).

## Credits

- [nottobii](https://github.com/puffnfresh/nottobii) by puffnfresh: protocol constants,
  init/auth sequence, gaze and calibration format (GPL-3.0)
- [tobiifree](https://github.com/Aetherall/tobiifree) by Aetherall: TTP/TLV protocol work (GPL-3.0)
- [smithay-client-toolkit](https://github.com/Smithay/client-toolkit) (layer-shell example structure, MIT)
- One-euro filter: Casiez, Roussel, Vogel, CHI 2012

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).
