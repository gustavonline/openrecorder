# Install

OpenRecorder is a preview. Pick one of: the macOS app, the Codex / Claude Code plugin (which carries the same app), or a build from source. Nothing needs Node.js or Python at run time.

## Check what you downloaded

Every release lists `SHA256SUMS` and each file's size. From the folder you downloaded into:

```sh
shasum -a 256 -c SHA256SUMS      # macOS / Linux
```

The same values are in the release notes and in `manifest.json`. Do not run a file whose checksum differs.

## macOS app (Apple silicon, macOS 26.0 or later)

**Minimum system: macOS 26.0 or later, Apple silicon.** This follows from the files themselves (read-only `otool -l`): the app executable is built for macOS 11.0 or later, but the bundled media helper `Contents/Helpers/ffmpeg` is built for macOS 26.0 or later (both against the macOS 26.5 SDK), and recording needs the helper, so the app and the plugin as shipped need macOS 26.0. The `LSMinimumSystemVersion` of 12.3 in the app's `Info.plist` is the minimum of the screen-capture interface the source uses; it is not what this download needs, and the signed app is not altered to say otherwise. Earlier systems are not supported and nothing was tested on them. Building the app yourself with a media helper that you build for an older target might run on an older system; that is not verified.

1. Download `OpenRecorder-0.6.0-preview.11-mac-arm64.zip` and verify it.
2. Unzip. The folder holds **OpenRecorder.app** and, beside it, the licence texts and notices (`LICENSE`, `THIRD_PARTY.md`, `licenses/`, `README.txt`, `INSTALL.md`); the signed app itself is unmodified. Move **OpenRecorder.app** to `~/Applications` (the plugin also looks there; one stable location keeps one app).
3. Open it. There is no Dock icon: look for the disc in the menu bar.

### macOS first open

The app is signed with a local development identity and is **not notarized by Apple**, so macOS shows a warning the first time. Apple documents the way to open an app that is not from an identified developer ([Open a Mac app from an unknown developer](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unknown-developer-mh40616/mac)): try to open the app, then choose Apple menu → System Settings → Privacy & Security, scroll to Security, click **Open Anyway** (available for about an hour after the attempt) and enter your password. Only do this for a file whose checksum you verified. OpenRecorder never asks you to weaken Gatekeeper or System Integrity Protection, never re-signs, and never resets any permission.

### Permissions

macOS asks for Screen Recording, Camera and Microphone at the moment you first use each, never at launch, in a menu or in Settings. Opening a menu, listing devices and choosing a device never turn a device on. If you decline, the app says so and offers a button that opens the right System Settings page; it never retries in a loop.

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
4. Ask your agent to open OpenRecorder Home (`openrecorder_home`). In a host that shows embedded apps it appears inline; otherwise ask it to open the library in your browser.

These commands follow the local-marketplace layout (`.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`) that each tool documents. **What was checked.** The manifests are checked by `tests/package-manifests.py`. Separately, the commands above were run against the unzipped `openrecorder/` folder of a macOS plugin ZIP of this version, extracted with `ditto`, using Codex CLI 0.167.1 and Claude Code 2.1.311, each in an isolated, throw-away configuration. Both registered the marketplace and installed `openrecorder@openrecorder`; the installed runtime binary kept its execute bit (mode 0755); `--doctor` on it reported `ok` with `deviceAccessRequested: false`; and an MCP `initialize` over stdio answered with server version `0.6.0-preview.11` and listed the three tools. That proves command-line registration, installation and server start-up for those two tool versions. It did not open the GUI, record, use any device or tunnel, or exercise any other version. (The runtime binary in that test is byte-identical to the one in the final ZIP; some documentation text in it predates the final ZIP.) Extract with Finder, `ditto -x -k` or `unzip`: an extractor that discards file modes leaves the binary non-executable. On the author’s own setup, posters and playback in Codex’s embedded window were accepted; no other host is claimed. If a command differs in your version, the manifests are plain JSON and the folder is a normal plugin directory.

On macOS the plugin starts the library host from its own copy and, when you open the recorder, installs that same signed app to `~/Applications/OpenRecorder.app` (never replacing a different build while any OpenRecorder process may be recording). The app and the plugin use the same data folder and the same single-owner lease, so there is always one library host.

## Linux and Windows (experimental)

The release carries **experimental** builds, compiled from the same source as the macOS app and **never run on a real desktop by the author**: tray, capture and permissions are unverified, and nothing here is a promise that they work.

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
