# Install

OpenRecorder is a preview. Pick one of: the macOS app, the Codex / Claude Code plugin (which carries the same app), or a build from source. Nothing needs Node.js or Python at run time.

## Check what you downloaded

Every release lists `SHA256SUMS` and each file's size. From the folder you downloaded into:

```sh
shasum -a 256 -c SHA256SUMS      # macOS / Linux
```

The same values are in the release notes and in `manifest.json`. Do not run a file whose checksum differs.

## macOS app (Apple silicon, macOS 26.0 or later)

**Minimum system: macOS 26.0 or later, Apple silicon.** This follows from the files themselves (read-only `otool -l`): the app executable is built for macOS 11.0 or later, but the bundled media helper `Contents/Helpers/ffmpeg` is built for macOS 26.0 or later (both against the macOS 26.5 SDK), and recording needs the helper, so the app and the plugin as shipped need macOS 26.0. Since preview.12 the app's `Info.plist` (`LSMinimumSystemVersion`) says 26.0 as well: the build reads the number from the shipped files instead of typing it. (The earlier preview.11 app still declared 12.3, the minimum of the screen-capture interface the source uses, which was never what that download needed; it stays as it was published.) Earlier systems are not supported and nothing was tested on them. Building the app yourself with a media helper that you build for an older target might run on an older system; that is not verified.

1. Download `OpenRecorder-0.6.0-preview.12-mac-arm64.zip` and verify it.
2. Unzip. The folder holds **OpenRecorder.app** and, beside it, the licence texts and notices (`LICENSE`, `THIRD_PARTY.md`, `licenses/`, `README.txt`, `INSTALL.md`); the signed app itself is unmodified. Move **OpenRecorder.app** to `~/Applications` (the plugin also looks there; one stable location keeps one app).
3. Open it. There is no Dock icon: look for the disc in the menu bar.

### macOS first open

The app is signed with a local development identity and is **not notarized by Apple**, so macOS shows a warning the first time. Apple documents the way to open an app that is not from an identified developer ([Open a Mac app from an unknown developer](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unknown-developer-mh40616/mac)): try to open the app, then choose Apple menu → System Settings → Privacy & Security, scroll to Security, click **Open Anyway** (available for about an hour after the attempt) and enter your password. Only do this for a file whose checksum you verified. OpenRecorder never asks you to weaken Gatekeeper or System Integrity Protection, never re-signs, and never resets any permission.

### Permissions

On its first open the recorder app shows one page for the three permissions (on the author's Mac, where they were already granted, all three read Allowed and a single Done dismissed it). macOS asks for Screen Recording, Camera and Microphone at the moment you first use each, never at launch, in a menu or in Settings. Opening a menu, listing devices and choosing a device never turn a device on. If you decline, the app says so and offers a button that opens the right System Settings page; it never retries in a loop.

## What opens when

OpenRecorder is two programs in one app: the **library** (Home, your recordings, sharing; it starts on its own and needs no permission) and the **recorder** (the menu-bar or tray app that shows permissions, lists devices and records).

- **The plugin carries the recorder app.** Adding the plugin to Codex or Claude Code only registers it; nothing is installed into `~/Applications` and nothing opens.
- **Home works without the recorder.** Asking your agent for Home (or opening it in the browser) starts only the library. Home then says "OpenRecorder isn't running" with an **Open OpenRecorder** button, and shows permissions as unknown rather than guessing: macOS tells OpenRecorder what it is allowed only while the recorder is open.
- **Open OpenRecorder** (the button in Home, the main control while the app is closed, or asking your agent for a new recording) is the first time anything is installed or launched. On a Mac it copies the signed app from the plugin to `~/Applications/OpenRecorder.app` (never replacing a different build while anything may be recording), opens it as a menu-bar app with no Dock icon, and Home says when it is ready. It does not record, open the camera or microphone, or leave a Record press waiting: nothing is captured until you press Record yourself, in the app or in Home. On Linux and Windows the same button starts the bundled program from the plugin folder.
- **While the recorder is closed, the main control in Home reads Open, not Record.** It is the same Open OpenRecorder: it opens the app and queues nothing. Nothing is recorded, and no countdown waits for the app to come up; once the app has reported, the control becomes Record and recording starts only when you press it.
- The **Open System Settings** buttons in Home › Settings › Permissions work whether or not the recorder is open. They open the page; they ask for nothing and change nothing. If the app cannot open a page it says why.
- A library that is already running, from an earlier preview, is never replaced or quit by any of this. Moving an installation to a new version is a separate, deliberate step in a quiet moment (nothing recording or shared, no one using it), done by whoever manages the installation with the library's own normal Quit; it is not an auto-updater (see [how it works](architecture.md#a-library-that-is-already-running)).
- No Node.js, Python or Rust is needed to run any of this (Rust is only for building from source).

## Plugin for Codex and Claude Code

The plugin ZIP (`openrecorder-plugin-<version>-<platform>.zip`) is a complete local plugin: manifest, skill, licences and the native runtime in `.runtime/`. It declares one MCP server that starts the bundled binary with `--stdio`.

1. Download, verify and unzip it somewhere permanent. The archive has one top-level folder, `openrecorder/`: that inner folder is what you add, and it is the plugin's home, so do not delete it afterwards. Adding the folder that *contains* `openrecorder/` fails in both tools (no marketplace manifest there).
2. **Codex**:
   ```sh
   codex plugin marketplace add /path/to/unzipped/openrecorder
   codex plugin add openrecorder@openrecorder
   ```
3. **Claude Code**:
   ```text
   /plugin marketplace add /path/to/unzipped/openrecorder
   /plugin install openrecorder@openrecorder
   ```
4. Ask your agent to open OpenRecorder Home (`openrecorder_home`). In a host that shows embedded apps it appears inline; otherwise ask it to open the library in your browser. A conversation that was already open when the plugin was installed or updated may keep its old tool list or show only text and data for Home; start a fresh session (see [OpenAI's plugin documentation](https://learn.chatgpt.com/docs/plugins)).

These commands follow the local-marketplace layout (`.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`) that each tool documents. The manifests are plain JSON and the folder is a normal plugin directory, so if a command differs in your version of a tool, adjust it.

**What was checked, kept apart.**

- *Preview.11 installation (historical).* With a preview.11 plugin ZIP extracted with `ditto`, the commands above were run with Codex CLI 0.167.1 and Claude Code 2.1.311, each in an isolated, throw-away configuration. Both registered the marketplace and installed `openrecorder@openrecorder`; the installed runtime binary kept its execute bit (mode 0755); `--doctor` on it reported `ok` with `deviceAccessRequested: false`; and an MCP `initialize` over stdio answered and listed the three tools. No GUI, recording, device or tunnel was involved. This is evidence about preview.11 only. It is not proof that preview.12 installs, and the preview.11 runtime is not the preview.12 runtime.
- *Preview.12 installation (the published plugin ZIP).* The published preview.12 plugin ZIP was installed with Codex CLI 0.167.2 and Claude Code 2.1.311, each in an isolated, throw-away configuration (`marketplace add`, then `plugin add` / `plugin install openrecorder@openrecorder`). The installed runtime was executable and passed `--doctor` (version 0.6.0-preview.12, `deviceAccessRequested: false`); an MCP `initialize` reported 0.6.0-preview.12 and listed the three tools. No GUI, recording, device or tunnel was involved, and no embedded Home window in an existing conversation is claimed.
- *Preview.12 runtime (separate evidence).* The signed preview.12 core in this release's Mac app and plugin passed the three protocol suites and `--doctor` (version 0.6.0-preview.12, `deviceAccessRequested: false`) on its own, outside the install commands, and `tests/package-manifests.py` checks the plugin manifests. A fresh smoke check of the install commands against the preview.12 plugin you downloaded (registration, then `--doctor`, then an MCP `initialize` that reports 0.6.0-preview.12) is the way to confirm it on your machine.

Extract with Finder, `ditto -x -k` or `unzip`: an extractor that discards file modes leaves the binary non-executable. On the author’s own setup, posters and playback in Codex’s embedded window were accepted; no other host is claimed.

On macOS the plugin starts the library host from its own copy and, when you open the recorder, installs that same signed app to `~/Applications/OpenRecorder.app` (never replacing a different build while any OpenRecorder process may be recording). The app and the plugin use the same data folder and the same single-owner lease, so there is always one library host.

## Linux and Windows (experimental)

The release carries **experimental** builds, compiled from the same source as the macOS app and **never run on a real desktop by the author**: tray, capture and permissions are unverified, and nothing here is a promise that they work. For preview.12 the Linux library host alone (the backend: no recorder window, no capture, no camera or microphone) was also built and run on a Linux x64 machine in a throw-away home and idled normally; the Windows build is cross-compiled and has not been run on Windows hardware.

- `OpenRecorder-<version>-linux-x64.tar.gz`: `openrecorder-core` and its media helper `ffmpeg`, with the licences. Built on a very new system: it needs **glibc 2.43 or newer**, `libdbus-1`, `libpipewire-0.3` and `libasound`. Extract, then run `./openrecorder-core` (add `--doctor` first). On an older distribution it will not start; build from source instead.
- `OpenRecorder-<version>-windows-x64.zip`: `openrecorder-core.exe`, `ffmpeg.exe` and the MinGW runtime `libwinpthread-1.dll`, cross-compiled. Windows may warn about an unsigned program; there is no signature.
- `openrecorder-plugin-<version>-linux-x64.zip` and `…-win32-x64.zip`: the same plugin as above with that runtime.

Verify each file's checksum first. You can also build one yourself:

```sh
# a media helper is required: FFmpeg 8.0 + x264 as described in THIRD_PARTY.md
python3 scripts/build-native.py /absolute/path/to/ffmpeg
python3 scripts/package-plugin.py linux-x64 /path/to/openrecorder-plugin-linux-x64.zip   # win32-x64 via scripts/package-windows.py
```

Linux needs a desktop with a StatusNotifierItem host for the tray and a screen-capture portal (PipeWire); without a tray OpenRecorder opens Home and says controls are there. Windows is unverified on hardware. The Linux build is not a general-purpose Linux download: unless your system has glibc 2.43 or newer, build from source. Report what you find in an issue, with the output of `OpenRecorder --doctor` (it checks the media helper and touches no device).

## Uninstall

See [update, uninstall and recovery](update-uninstall-recovery.md). In short: quit OpenRecorder, delete the app and the plugin folder; your recordings and settings stay where they are.
