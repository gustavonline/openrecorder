OpenRecorder 0.6.0-preview.11 (build 16): first public preview.

**What it is.** A small native screen recorder, controlled from the menu bar (macOS) or tray, with a movable camera bubble, separate screen and camera files, a local library (Home), temporary protected sharing, and a plugin for Codex and Claude Code whose tools can organise and share recordings but cannot start one.

**Downloads in this release**

- `OpenRecorder-0.6.0-preview.11-mac-arm64.zip`: the macOS app (Apple silicon; **requires macOS 26.0 or later**, set by the bundled media helper: the app executable alone is built for 11.0, and earlier systems are not supported or tested).
- `openrecorder-plugin-0.6.0-preview.11-darwin-arm64.zip`: the Codex / Claude Code plugin (a local marketplace) with the same app.
- Experimental, never run on a real desktop: `OpenRecorder-0.6.0-preview.11-linux-x64.tar.gz`, `…-windows-x64.zip` and the matching `openrecorder-plugin-0.6.0-preview.11-linux-x64.zip` / `…-win32-x64.zip` (Linux needs glibc 2.43 or newer).
- `openrecorder-media-sources-0.6.0-preview.11.tar.gz`: corresponding sources of the bundled FFmpeg + x264 helper (GPL-2.0-or-later).
- `openrecorder-0.6.0-preview.11-source.tar.gz`: the source tree.
- `SHA256SUMS`, `manifest.json`: checksums and sizes of every file above. Check them before opening anything: `shasum -a 256 -c SHA256SUMS`.

**Please read**

- **Not notarized.** macOS shows a warning the first time; see docs/install.md (Apple's own steps for opening an app from an unknown developer). Only do this for a file whose checksum you verified.
- **The Linux and Windows builds are experimental.** They are compiled from the same source but have not been run on a real desktop; the Linux build needs a very new system (glibc 2.43 or newer).
- 30 fps recording was accepted on the author's Mac; 60 fps depends on your computer and is never assumed. No memory, CPU or benchmark claim is made.
- The author reports sharing working in their own use; what was checked for this release is a synthetic server with a real Firefox, and no separate public-tunnel test was run for it. Posters and playback in Codex’s embedded window were accepted by the author on the author’s own setup; no other host is claimed. Plugin installation was checked from the command line in isolated Codex and Claude Code configurations: marketplace registration, install, `--doctor` and an MCP `initialize`/tool list, but not the GUI, a recording or any device.
- Updating or removing OpenRecorder never deletes recordings or settings and never resets permissions.

Docs: README, docs/install.md, docs/architecture.md, docs/update-uninstall-recovery.md. Security reports: SECURITY.md.
