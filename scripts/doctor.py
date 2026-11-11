#!/usr/bin/env python3
"""Developer wrapper around the native diagnostic. Packaged installs need neither Node.js nor Python.

Usage: python3 scripts/doctor.py [/path/to/OpenRecorder-or-openrecorder-core]
The real diagnostic is the packaged executable's own `--doctor`; it checks the bundled media helper and prints one
JSON object without touching camera, microphone, screen capture, a browser or a tunnel. This wrapper only locates a
binary for a source checkout and cross-checks the checkout's manifests.
"""
import json, os, pathlib, subprocess, sys

root = pathlib.Path(__file__).resolve().parents[1]
failed = False


def check(name, ok, detail):
    global failed
    print(f"{'OK' if ok else 'FAIL'} {name}: {detail}")
    failed = failed or not ok


def candidates():
    if len(sys.argv) > 1:
        yield pathlib.Path(sys.argv[1])
    if os.environ.get('OPENRECORDER_BIN'):
        yield pathlib.Path(os.environ['OPENRECORDER_BIN'])
    target = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', root / 'engine/target'))
    for profile in ('release', 'debug'):
        yield target / profile / 'openrecorder-core'
    yield root / '.runtime/OpenRecorder.app/Contents/MacOS/OpenRecorder'
    yield root / '.runtime/openrecorder-core'


try:
    manifest = json.loads((root / 'plugin.json').read_text())
    claude = json.loads((root / '.claude-plugin/plugin.json').read_text())
    check('versions', manifest['version'] == claude['version'], manifest['version'])
    server = json.loads((root / 'mcp.json').read_text())['mcpServers']['openrecorder']
    command = server['command']
    contained = command.startswith('./') and '..' not in command.split('/') and '${' not in command
    check('mcp.json', contained and server['args'][-1] == '--stdio' and 'node' not in command.lower(),
          'contained ./ command to the native executable (the generated Claude .mcp.json is separate)')
except (OSError, ValueError, KeyError) as e:
    check('manifests', False, f'{type(e).__name__}: {e}')

binary = next((p for p in candidates() if p.is_file() and os.access(p, os.X_OK)), None)
if binary is None:
    check('native executable', False, 'not found; build it (scripts/build-native.py) or pass its path')
else:
    try:
        run = subprocess.run([str(binary), '--doctor'], capture_output=True, text=True, timeout=20)
        report = json.loads(run.stdout) if run.returncode == 0 else {}
        check('native --doctor', run.returncode == 0 and report.get('ok') is True,
              run.stdout.strip() if run.returncode == 0 else (run.stderr.strip() or f'exit {run.returncode}'))
    except (OSError, ValueError, subprocess.SubprocessError) as e:
        check('native --doctor', False, f'{type(e).__name__}: {e}')
cloudflared = pathlib.Path(os.environ.get('OPENRECORDER_CLOUDFLARED') or ('/opt/homebrew/bin/cloudflared' if sys.platform == 'darwin' else '/usr/local/bin/cloudflared'))
print(f"OPTIONAL temporary sharing: {'cloudflared found' if cloudflared.is_file() else 'cloudflared not found; recording and the local library still work'} (presence only; nothing was started)")
print('No browser, camera, microphone or screen-access prompt was opened.')
sys.exit(1 if failed else 0)
