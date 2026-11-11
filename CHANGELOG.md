# Changelog

## 0.6.0-preview.11 (build 16)

First public release. Highlights since the earlier private previews:

- Menu-bar / tray control with real device names and one status vocabulary; four camera bubble shapes; Show camera; one-press Record from Home.
- Settings in Home (one flow for browser and agent window); calmer recovery for takes that stop early.
- Embedded agent window: posters and playback through bounded, validated reads; honest card states.
- Sharing: the unlock form works in real browsers (Referrer-Policy `same-origin`), formatted codes are accepted, "no end time" links, Copy invitation, styled guest pages.
- Pipeline: a writer thread hands frames to the encoder without copying the screen, and the camera is never held behind the encoder.
- Known limits: requires macOS 26.0 or later on Apple silicon (the bundled media helper is built for it); not notarized; experimental Linux and Windows builds never run on hardware; real public-tunnel sharing not yet confirmed by an independent run of this build; the Codex embedded window was accepted on the author’s own setup only.
