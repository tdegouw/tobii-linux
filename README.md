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
A calibration blob at `~/.config/tobii-et5/calibration.bin` is uploaded on init if present.

## Findings (firmware on IS50F)

- Gaze streams **without** a calibration blob (an empty blob is rejected, `0x20000503`).
- Output rate is **~33 Hz**, one 1724-byte message per USB transfer (no batching).
  FREQUENCIES_GET/LIST report a single mode `{132, 33}` — 132 Hz camera, 33 Hz gaze output.
  No setter is known; probing `0x067C` resets the tracker.
- Inbound frames: `LE32 id, LE32 total_len (incl. 8-byte header)`; outbound: `LE32 0, LE32 content_len`.
- Responses carry `0x00000001` in the upper half of the property field.
- AUTH_VERIFY/MOUNTING_CONFIG answer `0x20000505`; streaming still works.
- Roughly every other session stays silent after a successful subscribe; reopening
  the device fixes it (`Device::open_streaming` does this automatically).

## Roadmap

1. `et5-calibrate`: layer-shell overlay (smithay-client-toolkit) on the 5120x1440 output,
   7-point calibration via `calibration.rs`, saves `calibration.bin`.
2. `et5d`: gaze → filter (Kalman / 1€) → uinput absolute pointer, toggle hotkey.
3. systemd user service, started via udev `ENV{SYSTEMD_USER_WANTS}` on plug-in.
