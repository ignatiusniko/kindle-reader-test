# UX guidelines (as implemented)

- Three views only: **Library → Reader → Reading Options**. No settings screen.
- Library: *Continue reading* card (one tap resumes), paginated book list
  (no scrolling on E-Ink), Rescan, Exit. Missing files are shown as
  “File not found — progress kept”.
- Reader: page only. Left 30% / centre 40% / right 30% invisible tap zones.
  Centre opens Reading Options. In **Right → Left** (manga default) the left
  edge is *next*. Arrow / PageUp / PageDown keys work in the desktop preview.
- Reading Options: Fit Page/Width, Direction, Previous/Next with page count,
  coarse jumps (Start / 25 / 50 / 75 % / End), More → Advanced (contrast,
  page number on/off, Refresh screen, diagnostics), Library, Close.
- Fit Width reads tall pages in screen-sized segments with 10% overlap;
  going back lands on the previous page's last segment.
- Touch targets are sized in millimetres: `Theme.mm` = DPI / 25.4, buttons
  are ≥ 11 mm. `--dpi` overrides the 300 ppi default.
- Duplicate taps during a decode or within 300 ms are ignored.
- Errors never require restarting the app.
