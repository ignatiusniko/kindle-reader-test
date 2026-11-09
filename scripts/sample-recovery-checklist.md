# Recovery checklist (fill in BEFORE any graphics test — gate G3)

Print this, fill it in for **your** device and firmware, and rehearse every
step once without the new app. If any line is blank, do not run graphics.

| # | Question | Your answer (tested) |
|---|---|---|
| 1 | How do I reach SSH independently of the new UI (USBNetwork / Wi-Fi), and from which host? | |
| 2 | Command that shows whether the prototype is running (e.g. `pidof eink-reader`) | |
| 3 | How do I ask the prototype to exit normally? (UI Exit button; `kill -TERM <pid>`) | |
| 4 | How do I stop **only** the prototype if it hangs? (`kill -KILL <pid>` — never `killall` system services) | |
| 5 | Which Kindle services did `handoff_display` change, and the exact command that restores each? | |
| 6 | How do I confirm the Amazon UI and KOReader work again? | |
| 7 | If SSH is lost: which tested physical recovery do I use (e.g. hold power ~40 s restart)? | |
| 8 | Firmware version this checklist was verified on | |
| 9 | Date and git revision of the launcher it was verified with | |

Rules:

- Keep an SSH session open in another terminal during every graphics test.
- Never edit `/etc`, init scripts, system partitions or KOReader files.
- No boot-time autostart for experimental builds.
- Don't treat a soft reboot as a guaranteed restore method.
