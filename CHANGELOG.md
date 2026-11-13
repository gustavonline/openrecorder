# Changelog

## 0.6.0-preview.12 (build 17)

Plugin-only lifecycle repair (found after installing preview.11 from the plugin alone).

- Home › Settings › Permissions no longer shows three "Not reported yet" rows and buttons that do nothing when the recorder app is closed. It says the permissions are unknown until OpenRecorder is open, and the three **Open System Settings** buttons open their page through the library whether or not the recorder is running; a failure is shown, not swallowed.
- Home says plainly when the recorder app is not running and offers **Open OpenRecorder**: the app opens idle and ready, Home waits for its first report, says "ready", or says it did not report in time and offers Try again. It never records, opens a device or leaves a Record press waiting. A refused launch (for example while something may be recording) shows its reason.
- **The main control is no longer an automatic Record while the recorder app is closed.** It reads Open, opens the app idle and ready, and queues nothing: no countdown, input or recording waits for the app's first report (the 20-second waiting press is removed from the library). Once the app reports, the control is Record and records only when pressed.
- **Copy invitation** now puts a clean plain-text Danish message on the clipboard (nothing depends on Markdown): `Se min optagelse:`, the full link alone on its line, `Adgangskode:`, the code alone on its line (no quotation marks, backticks or closing punctuation), one line telling the recipient to open the link in a browser (no plugin needed), and what the end of access means. A timed share says when the link and code expire; a share with no end time says plainly that it is not permanent hosting: the link only works while this computer and the sharing stay on and online, and it can be stopped at any time. Copy link and Copy code are unchanged. The clipboard has only ever carried plain text, so no rich format was added.
- A library that is already running (preview.11) is no longer adopted blindly: its version is read from its own state and shown in `openrecorder_home`, and the new page operations are refused in plain words when it lacks them. Ordinary use never replaces or quits it. Updating an installation is a separate quiet-window step (`scripts/handoff-precheck.py`, then the library's normal Quit).
- The recorder is only ever shown as ready while the library says it is: an old idle report no longer shows an enabled Record button, and the opening poll cannot announce ready from it. A System Settings page that fails to open now shows the error (the opener's exit status is checked, with a bounded wait) instead of "Opening System Settings…".
- Windows: the Screen Recording button opens Microsoft's documented Graphics capture page (`ms-settings:privacy-graphicscaptureprogrammatic`), not the File system page; documented, not tried on Windows hardware.
- Passive Home still starts only the library. The agent's `open-studio` is the same passive "new recording" open and now reports `ready`.
- The macOS app now has its own icon (the black disc), and its `LSMinimumSystemVersion` is read from the shipped files (26.0) instead of the stale 12.3.
- The release packager no longer serializes the build machine's extended attributes: the Mac ZIP is made with `ditto --norsrc`, so it carries no `._name` AppleDouble entries or `__MACOSX` folder (a candidate once carried `Resources/._OpenRecorder.icns`), the archive checks reject such entries, and the README.txt beside the app now states what the bundle's own `Info.plist` really says (26.0) instead of a hard-coded number.
- Known limits: macOS 26.0 or later on Apple silicon; not notarized (local development signature); Linux x64 needs glibc 2.43 or newer and desktop capture, tray and permissions there are unverified (the library host alone was run); Windows is cross-compiled and unverified on hardware; no CPU or frame-rate benchmark claim.
- No change to recording, the media helper or the engine's capture paths.

## 0.6.0-preview.11 (build 16)

First public release. Highlights since the earlier private previews:

- Menu-bar / tray control with real device names and one status vocabulary; four camera bubble shapes; Show camera; one-press Record from Home.
- Settings in Home (one flow for browser and agent window); calmer recovery for takes that stop early.
- Embedded agent window: posters and playback through bounded, validated reads; honest card states.
- Sharing: the unlock form works in real browsers (Referrer-Policy `same-origin`), formatted codes are accepted, "no end time" links, Copy invitation, styled guest pages.
- Pipeline: a writer thread hands frames to the encoder without copying the screen, and the camera is never held behind the encoder.
- Known limits: requires macOS 26.0 or later on Apple silicon (the bundled media helper is built for it); not notarized; experimental Linux and Windows builds never run on hardware; real public-tunnel sharing not yet confirmed by an independent run of this build; the Codex embedded window was accepted on the author’s own setup only.
