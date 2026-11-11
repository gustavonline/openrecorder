# Contributing

OpenRecorder is a small personal project that welcomes focused fixes. Please open an issue before a large change.

## Ground rules

- **Recording is a human action.** A change must not let an agent, a tool call, startup, a poll, a refresh or a permission check start a recording or open a camera or microphone. Tests for that boundary exist; keep them meaningful.
- **No reset or surprise**: never reset permissions, delete recordings, or write outside the library and the chosen recordings folder.
- **Keep it small**: no accounts, editor, analytics, framework, extra webview or required Node.js/Python at run time. Node and Python are optional build and test tools only.
- **Be honest in copy and docs**: say what was verified and what was not. Preview status, Windows/Linux limits and the macOS signing limits are stated on purpose.
- The look is in [DESIGN.md](DESIGN.md): neutral, calm, no orange or purple.

## Checks to run

```sh
node tests/ui-cap-contracts.cjs
# on Linux, first: sudo apt-get install pkg-config libwayland-dev   (Debian/Ubuntu names; the Wayland client library's pkg-config file)
cd engine && cargo test --features desktop-ui
python3 tests/package-manifests.py        # needs engine/runtime archives from scripts/build-native.py
python3 scripts/check-site.py             # the static website
```

`tests/host-lifecycle.py`, `tests/rust-mcp-lifecycle.py` and `tests/slow-host-lease.py` take a built `openrecorder-core` and run against a throw-away home; they never touch a device. Anything that records, shares publicly, or needs a real permission is a manual check: do not automate it, and describe what you ran.

## Pull requests

Describe the behaviour change and how it was checked. Add a test that fails without the change where a test can observe it. Do not commit recordings, keys, certificates or paths from your machine.
