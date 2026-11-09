# KUAL extension: E-Ink Reader

Adds **E-Ink Reader (Experimental)** to KUAL with three entries:

| Entry | Starts |
|---|---|
| Open reader | `eink-reader` via `run-graphics.sh` |
| Open reader (with timing log) | same, with `--log` (writes `data/logs/reader.log`) |
| Hardware test window | `eink-reader-m0` (tap counter + Exit) |

`bin/launch.sh` only starts the device-verified launcher
(`/mnt/us/eink-reader-dev/run-graphics.sh`) in the background, after a short
delay so KUAL can close first. The launcher pauses the Kindle UI while the app
runs and restores it on every exit. Leave the app with its **Exit** button
(Library screen), or **Library → Exit**.

Installed by `scripts/deploy-kindle.sh` to `/mnt/us/extensions/eink-reader/`.
Uninstall: delete that folder (and `/mnt/us/eink-reader-dev` for the app).
Launch messages go to `/mnt/us/eink-reader-dev/logs/kual.log`, app output to
`logs/ui.log`.

Verified only on Kindle Basic 2022 (KT5) firmware 5.18.5.0.1.
