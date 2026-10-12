# AI disclaimer

Most of the code and documentation in this repository was written by an AI assistant
(Grok Bot) under the direction of the repository owner, who specified the goals, made the design
decisions, and reviewed and tested the result on their own hardware (Tobii Eye Tracker 5,
Arch Linux / Omarchy, Hyprland, 49" 5120x1440 display).

- The USB protocol is not documented by Tobii. Everything here builds on the
  reverse-engineering work of [nottobii](https://github.com/puffnfresh/nottobii) and
  [tobiifree](https://github.com/Aetherall/tobiifree) (both GPL-3.0).
- This project is **not affiliated with, endorsed by, or supported by Tobii AB**.
  "Tobii" and "Eye Tracker 5" are trademarks of their respective owner.
- The software is **experimental**. It talks to the device with undocumented commands;
  an unexpected command has been observed to reset the tracker. It is tested on one
  device and one firmware only.
- **Use at your own risk.** Provided under the GPL-3.0-or-later, without any warranty
  (see `LICENSE`, sections 15–16).
