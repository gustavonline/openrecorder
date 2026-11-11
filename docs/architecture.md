# How OpenRecorder works

One native executable (Rust) plays four roles, chosen by its first argument. Everything runs on your computer; nothing here needs Node.js, Python or a browser engine.

```
 Codex / Claude Code ──stdio──▶ OpenRecorder --stdio   (MCP bridge, no state)
                                       │ loopback HTTP, owner key
                                       ▼
 menu bar / tray ◀──── OpenRecorder (native app) ◀──── OpenRecorder --library-host
   Record, Stop,             egui + capture + encoder        one per home, holds the lease,
   devices, bubble           (FFmpeg helper process)         serves Home and the library
                                                                    │ loopback only
 browser / agent window ◀──────── Home (one HTML page) ─────────────┘
 a guest ◀── cloudflared (optional) ◀── guest server (its own listener, unlock page + one video)
```

## The pieces

- **Native app** (`--desktop`, the default): the menu-bar or tray icon, the countdown, the camera bubble and the capture pipeline (screen via the `scap` adapter, camera, microphone). It draws frames into a packed layout and feeds one **FFmpeg helper** process that writes `recording.mp4` and the optional `screen.mp4` / `camera.mp4` with the platform's hardware H.264 encoder when it works (VideoToolbox on macOS), and libx264 otherwise. It is the only component that opens a camera, a microphone or the screen, and only on your action.
- **Library host** (`--library-host`): a small local HTTP service that owns the library (an index plus one folder per take). An operating-system file lock makes it one per home; a slow host is never replaced by a second one and a crashed host frees the lock. It serves **Home**, the single embedded HTML page, and the owner operations the page and the app need. It exits only on Quit (refused while a take is active).
- **MCP bridge** (`--stdio`): translates MCP over stdio into calls to the library host and starts the host when none runs. It holds no recording state. It is what an agent talks to.
- **Home**: one page (`ui/`, inlined by the host) used in the browser and inside an agent window, with Library, a recording page with a player, Settings (General, Recording, Camera & Microphone, Permissions, Sharing, About) and the Record controls.
- **Guest server** (sharing): a separate bounded HTTP listener bound to loopback per share, exposed by an optional Cloudflare Quick Tunnel (`cloudflared`, started only when you share). It serves the unlock page, the stylesheet, and after a correct code the player, the video (GET/HEAD/Range) and a download of **one pinned combined file**. It has no owner route, no library, no Settings and no MCP.

## What an agent can and cannot do

The MCP server lists three tools:

| Tool | Visible to the model | Purpose |
|---|---|---|
| `openrecorder_home` | yes | Library state and the app resource; never an access link |
| `openrecorder_action` | yes | An allowlist: `state`, `open-studio`, `open-library`, `settings`, `folder`, `update`, `stop`, `reveal`, `open-file`, `share`, `unshare`, `quit` |
| `openrecorder_ui` | no (`visibility: ["app"]`) | For the app window only: bounded media reads, a fresh media address and your own Record press |

None of the three tools can start recording, toggle the camera or microphone, choose a source, check 60 fps or change permissions. `stop` is allowed because you may ask an agent to stop a take. This describes the tools the plugin provides, not everything an agent can do on your computer.

The app-only tool is not protected by being hidden. Each call needs a per-session key that exists only in the window's own copy of the page (never in a tool answer), and Record carries a launch, take, expiry and single-use binding that the library host and the native app each check again at the moment of execution: a late, replayed or pre-restart request cannot start anything. This is a guard for tool calls, not a sandbox against an agent that has unrestricted shell or computer-use access as your user.

## Sharing, precisely

A share needs a link (a fresh tunnel host plus a 128-bit path) **and** a separate eight-character code. Wrong codes are limited to five per rolling minute across the link. A correct code issues a 256-bit session in an `HttpOnly; Secure; SameSite=Strict` cookie scoped to that share, which ends with the link or after a week, whichever is first. Links and codes live in memory only. A link ends on Revoke, Archive, Quit, host exit, its expiry, or when its tunnel drops; "no end time" is only while OpenRecorder and the tunnel keep running. The guest pages allow no script, no inline style and no external resource.

## Data and privacy

Recordings, the library index and settings are local files (see [update, uninstall and recovery](update-uninstall-recovery.md)). The app makes no network request of its own except to its loopback services and, when you share, through `cloudflared`. No analytics, no accounts.

## Source layout

`engine/` Rust crate (`src/`, vendored `scap`), `ui/` Home page, `skills/` the agent skill, `scripts/` build and packaging, `tests/` contract and lifecycle checks, `site/` the landing page, `marketplace/` submission preparation, `docs/`.
