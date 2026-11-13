OpenRecorder 0.6.0-preview.12 (build 17): plugin-only lifecycle repair. It follows 0.6.0-preview.11, which stays published exactly as it was.

**What it is.** A small native screen recorder, controlled from the menu bar (macOS) or tray, with a movable camera bubble, separate screen and camera files, a local library (Home), temporary protected sharing, and a plugin for Codex and Claude Code whose tools can organise and share recordings but cannot start one.

**What changed since preview.11** (found by installing preview.11 from the plugin alone)

- **Home and the recorder app are separate, and Home says so.** The library (Home) starts on its own and works with the recorder app closed. While it is closed, Settings › Permissions says the permissions are unknown until OpenRecorder is open (not three "Not reported yet" lines), and the three **Open System Settings** buttons open the right page anyway. The main control reads **Open**: it installs the signed app to `~/Applications` on first use (never replacing a different build while anything may be recording), opens it as a menu-bar app, and Home says when it is ready, or that it did not report in time, or why it was refused, with Try again. Opening queues nothing: it never records, no countdown waits for the app, and Record appears once the app has reported.
- **The pane follows the app by itself.** The Permissions pane now updates in place (unknown, opening, ready, not responding, timed out) even while its Open button has focus; before, it stayed on Unknown until a reload.
- **Ready means ready.** An old idle report no longer shows an enabled Record button while the library says the recorder app is not responding.
- **A System Settings page that fails to open shows the error** instead of "Opening System Settings…". On Windows the Screen Recording button opens Microsoft's documented Graphics capture page (documented, not tried on Windows hardware).
- **Copy invitation** puts a plain-text Danish message on the clipboard: `Se min optagelse:`, the full link on its own line, `Adgangskode:`, the code on its own line, one line telling the recipient to open the link in a browser (no plugin), and what the end of access means. The Quick Tunnel link is temporary and works only while your computer and the tunnel stay online; a link with no end time is not permanent hosting, and it can be stopped at any time. Copy link and Copy code are unchanged.
- **An older library that is already running (for example preview.11) is never replaced or quit by ordinary use.** Its version is read from its own state; the new page operations are refused in plain words when it lacks them. Moving an installation to a new version is a separate step for whoever manages it, in a quiet moment (nothing recording or shared, no one using it), with the library's own normal Quit after `scripts/handoff-precheck.py` reads it as idle. It is not an auto-updater.
- The macOS app has its own icon (the black disc), and its `Info.plist` now says macOS 26.0, read from the shipped files.
- No change to recording, the media helper or the capture paths.

**Downloads in this release**

- `OpenRecorder-0.6.0-preview.12-mac-arm64.zip`: the macOS app (Apple silicon; **requires macOS 26.0 or later**, set by the bundled media helper: the app executable alone is built for 11.0, and earlier systems are not supported or tested), with the licence texts beside it.
- `openrecorder-plugin-0.6.0-preview.12-darwin-arm64.zip`: the Codex / Claude Code plugin (a local marketplace) with the same app. Adding it only registers it; nothing is installed or opened until a person presses Open.
- Experimental, desktop capture never run: `OpenRecorder-0.6.0-preview.12-linux-x64.tar.gz`, `…-windows-x64.zip` and the matching `openrecorder-plugin-0.6.0-preview.12-linux-x64.zip` / `…-win32-x64.zip`.
- `openrecorder-media-sources-0.6.0-preview.12.tar.gz`: corresponding sources of the bundled FFmpeg + x264 helper (GPL-2.0-or-later).
- `openrecorder-0.6.0-preview.12-source.tar.gz`: the source tree.
- `SHA256SUMS`, `manifest.json`: checksums and sizes of every file above. Check them before opening anything: `shasum -a 256 -c SHA256SUMS`.

**Please read**

- **Not notarized.** The app is signed with the author's local development identity, not an Apple Developer ID. macOS shows a warning the first time; see docs/install.md (Apple's own steps for opening an app from an unknown developer). Only do this for a file whose checksum you verified.
- **Linux (x64) needs glibc 2.43 or newer** plus `libdbus`, `libpipewire` and `libasound`, and is experimental: the library host alone (no recorder window, no capture) was built and run on a Linux machine and idled normally, but desktop capture, the tray and permissions on Linux are unverified. **The Windows build is cross-compiled and unverified on Windows hardware.** Neither is a promise that it works.
- Two private memory measurements on the author's own setup, not a promise and not total recording memory: the macOS recorder app plus its library host, idle with no capture and excluding any browser, about 44.5 MiB resident; the library host alone about 8.7 MiB; the Linux library host alone on one x64 machine about 7.3 MiB resident (4.1 MiB proportional). No CPU or frame-rate benchmark is claimed; 60 fps depends on your computer and is never assumed.
- On the author's Mac the signed app's first open showed one page for the three permissions and a single Done, and the Permissions pane followed the app's readiness. That is one machine, not every Mac.
- The author reports sharing working in their own use; no separate public-tunnel test was run for this version. Posters and playback in Codex's embedded window were accepted on the author's own setup; no other host is claimed. Plugin installation was checked from the command line with preview.11 in isolated Codex and Claude Code configurations; repeat it for this version before relying on it.
- Using the plugin or the app takes no Node.js, Python or Rust. Updating or removing OpenRecorder never deletes recordings or settings and never resets permissions.

Docs: README, docs/install.md, docs/architecture.md, docs/update-uninstall-recovery.md. Security reports: SECURITY.md.
