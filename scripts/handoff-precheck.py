#!/usr/bin/env python3
"""Read-only precheck for ONE separately controlled, quiescent, normal handoff of a running older library to this version.

  python3 scripts/handoff-precheck.py --home DIR        # ask the running library (its own `state`) and look at the running programs
  python3 scripts/handoff-precheck.py --state FILE      # judge a saved `state` answer (and the running programs)

This is NOT part of the app. Ordinary use (a plugin session, Home, an agent, a UI control) never replaces a running library and never
sends it a Quit. The deployment owner may, once, in a window in which every caller of that library is quiet and stays quiet until the
new library has been read back, send the library its normal Quit and let the new version take the operating-system lease. This script
only says whether the library looks idle right now. It sends nothing but `state`, prints no key, and kills nothing.

Idle means ALL of, each read strictly: a well-formed older version; `active` present and explicitly null; `shares` a list whose items are
all `failed` (a live, starting, unknown or malformed one is not idle: links end when the library quits); `capture` present with
`known` false and phase `unavailable` (no recorder app has reported); no OpenRecorder program other than MCP bridges, `--doctor` and
library hosts is running; the library answered. A missing field is NOT the same as a null one, and anything malformed means "not idle".
Whether callers are quiet cannot be read from here: the operator must know it, and this script says so.
Exit status 0 only when idle.
"""
import argparse, json, os, pathlib, re, subprocess, sys, urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
# The program itself, judged by its executable path (`comm`), never by what its arguments mention.
HELPER = re.compile(r'OpenRecorder[^/]*\.app/Contents/Helpers/')


def version_key(text):
    """Strict ordering key for X.Y.Z or X.Y.Z-preview.N; None for anything else (never guessed from stray digits)."""
    m = re.fullmatch(r'(\d+)\.(\d+)\.(\d+)(?:-preview\.(\d+))?', text) if isinstance(text, str) else None
    if not m:
        return None
    a, b, c, n = m.groups()
    return (int(a), int(b), int(c), 0, int(n)) if n is not None else (int(a), int(b), int(c), 1, 0)


def target_version():
    return re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / 'engine/Cargo.toml').read_text(), re.M).group(1)


def open_programs(rows, ignore_pids=()):
    """Rows (pid, executable path, arguments) that could be the recorder app or a take: an OpenRecorder executable (or its bundled media
    helper) that is not an MCP bridge, `--doctor` or a library host. A row that does not parse counts."""
    found = []
    for row in rows:
        try:
            pid, comm, args = row
            pid = int(pid)
        except (TypeError, ValueError):
            found.append(str(row))
            continue
        if pid in ignore_pids:
            continue
        name = pathlib.PurePath(comm).name
        if (name in ('OpenRecorder', 'openrecorder-core') or HELPER.search(comm)) and not any(f in args.split() for f in ('--stdio', '--doctor', '--library-host')):
            found.append(f'{pid} {comm} {args}'[:160])
    return found


def blockers(state, listing, target=None, ignore_pids=()):
    """Reasons the library is not idle (empty list = idle). `listing` is the process rows (see `open_programs`), or None if they could not be read."""
    target = target or target_version()
    why = []
    if not isinstance(state, dict):
        return ['its answer was not a state object']
    key, ours = version_key(state.get('version')), version_key(target)
    if key is None:
        why.append(f"its version {state.get('version')!r} is missing or malformed")
    elif ours is None or key >= ours:
        why.append(f"it is {state.get('version')}, which is not older than {target}")
    if 'active' not in state or state['active'] is not None:
        why.append('a recording is in progress or being saved, or `active` is missing or malformed' if state.get('active') is not None or 'active' not in state else '')
    shares = state.get('shares')
    if not isinstance(shares, list) or not all(isinstance(s, dict) and s.get('status') == 'failed' for s in shares):
        why.append('a share is live, starting, unknown or malformed (or `shares` is missing)')
    capture = state.get('capture')
    if not (isinstance(capture, dict) and capture.get('known') is False and capture.get('phase') == 'unavailable'):
        why.append('the recorder app has reported, or `capture` is missing or malformed')
    if listing is None:
        why.append('the running programs could not be listed')
    else:
        running = open_programs(listing, ignore_pids)
        if running:
            why.append('an OpenRecorder program that could be the app or a take is running: ' + '; '.join(running)[:300])
    return [w for w in why if w]


def ask_state(home):
    c = json.loads((pathlib.Path(home) / 'rust-runtime.json').read_text())
    request = urllib.request.Request(c['origin'] + '/api/action', data=json.dumps({'op': 'state', 'data': {}}).encode(),
                                     headers={'Authorization': 'Bearer ' + c['key'], 'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=5) as r:
        return json.loads(r.read())


def process_listing():
    def columns(name):
        out = subprocess.run(['/bin/ps', '-axww', '-o', f'pid=,{name}='], capture_output=True, text=True)
        if out.returncode != 0:
            return None
        rows = {}
        for line in out.stdout.splitlines():
            pid, _, rest = line.strip().partition(' ')
            if pid.isdigit():
                rows[int(pid)] = rest.strip()
        return rows
    comm, args = columns('comm'), columns('args')
    return None if comm is None or args is None else [(pid, path, args.get(pid, '')) for pid, path in comm.items()]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    source = ap.add_mutually_exclusive_group(required=True)
    source.add_argument('--home'); source.add_argument('--state')
    ap.add_argument('--target')
    a = ap.parse_args()
    try:
        state = ask_state(a.home) if a.home else json.loads(pathlib.Path(a.state).read_text())
        asked = None
    except Exception as e:  # unreadable, refused, timed out, not JSON: all mean "not idle"
        state, asked = None, str(e)
    why = blockers(state, process_listing(), a.target, ignore_pids={os.getpid()}) if state is not None else [f'the library did not give a readable state ({asked})']
    print(json.dumps({'idle': not why, 'version': state.get('version') if isinstance(state, dict) else None, 'target': a.target or target_version(), 'reasons': why,
                      'callersQuiet': 'NOT CHECKED here: confirm every caller (agent sessions, Home windows, bridges) is quiet and stays quiet until the new library is read back'}, indent=2))
    sys.exit(0 if not why else 1)


if __name__ == '__main__':
    main()
