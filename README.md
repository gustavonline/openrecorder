<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/icon-dark.svg">
  <img src="assets/icon.svg" alt="OpenRecorder" width="40" height="40">
</picture>

# OpenRecorder

A small native screen recorder. Control it from the menu bar (macOS) or tray (Windows, Linux), put a camera bubble on the screen, and keep every recording, and the separate screen and camera files, as ordinary files on your computer. An optional plugin lets Codex or Claude Code find, organise and share your recordings. **The plugin’s tools cannot start a recording**: you start one yourself, in the app. That is a limit of the tools the plugin provides, not a sandbox: an agent that can run shell commands or control your computer acts as you.

[Website](https://openrecorder.gustavonline.com/) · [Releases](https://github.com/gustavonline/openrecorder/releases) · [Install](docs/install.md) · [How it works](docs/architecture.md)

**Version 0.6.0-preview.12 (build 17), a preview.** Free and open source (MIT). Made by Gustav Anderson. It follows 0.6.0-preview.11, the first public release, which stays published exactly as it was ([CHANGELOG](CHANGELOG.md)).

## Status, honestly

| | |
|---|---|
| **macOS 26.0 or later, Apple silicon** | Built and used by its author. The shipped app and plugin need macOS 26.0 or later because the bundled media helper is built for 26.0 (the app executable alone is built for 11.0, and this version's `Info.plist` says 26.0 too); earlier systems are not supported and untested ([details](docs/install.md#macos-app-apple-silicon-macos-260-or-later)). A release build is published only when its files and SHA-256 checksums have been verified (see [Releases](https://github.com/gustavonline/openrecorder/releases)). **Not notarized**: macOS will warn on first open ([steps](docs/install.md#macos-first-open)). |
| **Linux x64, Windows x64** | **Experimental builds** compiled from the same source, published with their checksums but **never run on a real desktop by the author**: tray, capture and permissions are unverified. The Linux build was made on a very new system (it needs glibc 2.43 or newer plus `libdbus`, `libpipewire` and `libasound`), so on most distributions build from source instead. For this version the Linux library host (the backend, no recorder window, no capture) was also built and run on a Linux machine and idled normally; **desktop capture, the tray and permissions on Linux are still unverified**. The Windows build is cross-compiled and **unverified on Windows hardware**. |
| **Frame rate and performance** | 30 fps recording was accepted on the author's Mac. 60 fps depends on your computer and is never assumed: a take that cannot keep up stops with its files kept. Two private measurements on the author's own setup, not a promise and not total recording memory: the macOS recorder app plus its library host, idle with no capture and excluding any browser, held about 44.5 MiB resident; the library host alone about 8.7 MiB; on one Linux x64 machine the library host alone idled at about 7.3 MiB resident (4.1 MiB proportional). No CPU or frame-rate benchmark is claimed. |
| **Sharing** | Temporary links through an optional Cloudflare Quick Tunnel (`cloudflared`, not bundled). The author reports it working in their own use; what was checked for this release is a synthetic in-process server with a real Firefox, and **no separate public-tunnel test was run for it**. Copy invitation puts a plain-text Danish message on the clipboard (link and code each on their own line); a recipient needs only a browser. The Quick Tunnel link is temporary and works only while your computer and the tunnel stay online: a link with no end time is not permanent hosting. |
| **Agent window and plugin install** | With the plugin installed, posters and playback in Codex’s embedded window were accepted by the author on the author’s own setup; other hosts and versions are untested. Installation was separately checked in isolated Codex and Claude Code command-line configurations (see [docs/install.md](docs/install.md#plugin-for-codex-and-claude-code)): marketplace registration, install, `--doctor` and an MCP `initialize`/tool list, but not the GUI, a recording or any device. The browser always works as a fallback. |

## What it does

- **Menu-bar / tray control**: Record (3-second countdown, cancellable), Stop, Source (display or window), Camera and Microphone with the real device names and one status vocabulary (Off, On, Starting…, Live, Not connected, Access off). Choosing a device never opens it.
- **Camera bubble**: Circle, Rounded square, Wide 16:9 or Portrait 9:16; size, mirror preview and Show camera are live during a take. The bubble appears in the combined video; `camera.mp4` keeps the full, unmirrored frame in its own aspect.
- **Real files**: each take is a folder with `recording.mp4`, and (optionally) `screen.mp4` and `camera.mp4`. A take that stops early keeps what was written and says so.
- **Home**: a local library page (browser, or embedded in an agent window) with search, sort, favorites, folders, Archive with Undo, Settings, and Record. Recording from Home is one press.
- **Protected sharing**: a link plus a separate access code for one recording's combined video, with a lifetime of 15 minutes, 1 hour, 8 hours or none while OpenRecorder stays open. Copy link, Copy code or Copy invitation; nothing is sent for you.
- **Plugin for Codex and Claude Code**: read the library, open Home, change settings, share and unshare, stop a take when you ask. Its tools cannot start recording, turn the camera or microphone on, choose a source or touch permissions.

## Install

Short version; the details, checksums and uninstall are in [docs/install.md](docs/install.md).

**macOS app**: download `OpenRecorder-0.6.0-preview.12-mac-arm64.zip` from the release (Linux and Windows: the experimental `OpenRecorder-0.6.0-preview.12-linux-x64.tar.gz` / `…-windows-x64.zip`, see [docs/install.md](docs/install.md#linux-and-windows-experimental)), unzip (the folder also holds the licence texts), move `OpenRecorder.app` to `~/Applications`, open it once (it is not notarized: see the steps), then use the menu-bar icon.

**Codex** (after downloading and unzipping `openrecorder-plugin-0.6.0-preview.12-darwin-arm64.zip`; add the `openrecorder` folder *inside* the unzipped archive, not the folder that contains it):

```sh
codex plugin marketplace add /path/to/unzipped/openrecorder
codex plugin add openrecorder@openrecorder
```

**Claude Code**:

```text
/plugin marketplace add /path/to/unzipped/openrecorder
/plugin install openrecorder@openrecorder
```

The plugin carries the same native binary, so using it takes **no Node.js, Python or Rust** (Rust is only for building from source). It shares the app's data and its single-owner lease. Adding the plugin only registers it. Home (the library) then starts on its own and works with the recorder app closed; the recorder app is installed to `~/Applications` and opened the first time a person presses **Open OpenRecorder**, the main control in Home while the app is closed. Opening it queues nothing: it never records, and no countdown waits for it; Record appears once the app has reported, and records only when pressed ([what opens when](docs/install.md#what-opens-when)). A library that is already running (for example from preview.11) is never replaced or quit by ordinary use; moving an installation to a new version is a separate, deliberate step in a quiet moment, not an auto-updater ([details](docs/architecture.md#a-library-that-is-already-running)).

**From source** (any platform; needs Rust and a media helper):

```sh
python3 scripts/build-native.py /absolute/path/to/ffmpeg   # builds engine/runtime/<platform>-<arch>.tar.gz
python3 scripts/package-plugin.py darwin-arm64 /path/to/openrecorder-plugin.zip
```

The FFmpeg helper is a purpose-built FFmpeg 8.0 + x264 (GPL-2.0-or-later) that runs as a separate process. Its configuration and matching sources are described in [THIRD_PARTY.md](THIRD_PARTY.md) and published beside each release.

## Your data

Your recordings, library index and settings are ordinary files on your computer. The library lives in `~/Library/Application Support/OpenRecorder/Native` (macOS), `%LOCALAPPDATA%\OpenRecorder\Native` (Windows) or `$XDG_DATA_HOME/openrecorder/native` (Linux), and recordings default to `~/Movies/OpenRecorder` (macOS) or `~/Videos/OpenRecorder`. Updating or removing the app or plugin **never deletes recordings or settings and never resets permissions**. No account, no analytics, no crash reporting, no cloud storage, and OpenRecorder does not upload your recordings. Two optional features involve other parties: if you use the plugin, your AI host (Codex or Claude Code) receives what the tools return, such as titles, file names and information, library state and any share link and code you ask for, and handles it under its provider’s terms; if you create a share link, the video is served from your computer through a `cloudflared` tunnel that you install yourself, and that traffic (and `cloudflared`’s own connection) goes through Cloudflare under its terms. See [the privacy page](https://openrecorder.gustavonline.com/privacy.html) and [docs/update-uninstall-recovery.md](docs/update-uninstall-recovery.md).

## Develop

```sh
node tests/ui-cap-contracts.cjs                      # Home page contracts (Node is a test tool only)
cd engine && cargo test --features desktop-ui        # Rust unit tests
python3 tests/package-manifests.py                   # package manifest checks (needs engine/runtime archives)
python3 tests/host-lifecycle.py path/to/openrecorder-core   # real host process, throw-away home, no hardware
```

[CONTRIBUTING.md](CONTRIBUTING.md) · [SECURITY.md](SECURITY.md) · [SUPPORT.md](SUPPORT.md) · [DESIGN.md](DESIGN.md) · [Credits and licences](THIRD_PARTY.md)
