"""An accepted preview.11 library is already running when a preview.12 plugin session arrives.

Usage: python3 tests/version-handoff.py /path/to/new/openrecorder-core /path/to/accepted/preview.11/OpenRecorder
Real processes in throw-away HOME / OPENRECORDER_HOME directories with a disabled (or deliberately slow) tunnel executable. The older
binary is only copied and run as `--stdio` / `--library-host` for that home; no GUI, browser, device, signer, owner library or owner
process is touched, and no Quit is ever sent to anything but a library this test started.

The contract: ORDINARY use never replaces, quits or touches a running library, whatever its version or state (an idle one, one with a
take, a starting share or the recorder app, a stalled one, or one that answers with a malformed state); it only describes it and refuses
the page operations that library lacks in plain words. Moving the owner's library to a new version is ONE separately controlled step in
a quiet window: `scripts/handoff-precheck.py` says it is idle, the operator sends the library its normal Quit, and the new version then
takes the operating-system lease with the same data. That sequence is run here, on throw-away libraries only.
"""
import copy, fcntl, http.server, importlib.util, json, os, pathlib, re, shutil, signal, stat, sys, tempfile, threading, time
from hostfixture import Bridge, Fixture, HOME_URI, alive, ps

NEW, OLD = str(pathlib.Path(sys.argv[1]).resolve()), pathlib.Path(sys.argv[2]).resolve()
OLD_VERSION, checks, cleanups = '0.6.0-preview.11', [], []
spec = importlib.util.spec_from_file_location('handoff_precheck', pathlib.Path(__file__).resolve().parents[1] / 'scripts/handoff-precheck.py')
precheck = importlib.util.module_from_spec(spec); spec.loader.exec_module(precheck)
NEW_VERSION = precheck.target_version()
IN_BUNDLE = '.app/Contents/MacOS/' in NEW


def ok(name):
    checks.append(name)


def make(prefix, slow_tunnel=False):
    """An older fixture (binary = a copy of the accepted preview.11 core) and a newer one sharing the same home."""
    scratch = pathlib.Path(tempfile.mkdtemp(prefix='recorder-old-binary-'))
    old_binary = scratch / 'OpenRecorder'
    shutil.copyfile(OLD, old_binary)
    old_binary.chmod(0o755)
    old = Fixture(old_binary, prefix)
    if slow_tunnel:
        slow = old.root / 'tunnel-slow'
        slow.write_text('#!/bin/sh\nexec sleep 60\n')
        slow.chmod(slow.stat().st_mode | stat.S_IXUSR)
        old.env['OPENRECORDER_CLOUDFLARED'] = str(slow)
    new = copy.copy(old)
    new.binary = NEW
    new.bridges = []                   # its own bridges (their stderr logs share names with the older ones'; they are throw-away)
    cleanups.append((old, new, scratch))
    return old, new


def old_library(old):
    bridge = Bridge(old)
    assert 'result' in bridge.initialize()
    home = bridge.tool('openrecorder_home')
    assert not home.get('isError'), home
    c = old.descriptor()
    assert 'version' not in c, 'the preview.11 descriptor carries no version'
    assert home['structuredContent']['version'] == OLD_VERSION and 'backend' not in home['structuredContent'] and 'recorder' not in home['structuredContent']
    return bridge, c


def new_home(new):
    bridge = Bridge(new)
    assert 'result' in bridge.initialize()
    return bridge, bridge.tool('openrecorder_home', timeout=40)


def ui_key(bridge):
    text = json.dumps(bridge.rpc('resources/read', {'uri': HOME_URI}))
    return re.search(r'uiKey\\?"\s*:\s*\\?"([0-9a-f]+)', text).group(1)


def kept(old, c, result):
    backend = result['structuredContent']['backend']
    assert (backend['current'], backend['handoff'], backend['version'], backend['pageOps']) == (False, 'none', OLD_VERSION, False), backend
    assert 'never replaces' in backend['reason'], backend
    assert old.descriptor()['pid'] == c['pid'] and alive(c['pid']), 'the older library must be left alone'
    assert old.jpost(c, 'state')[1]['version'] == OLD_VERSION


def refused_in_plain_words(bridge, key):
    for op, data in (('open-recorder', {}), ('open-privacy-settings', {'kind': 'screen'})):
        r = bridge.ui(op, data, key=key)
        text = r['content'][0]['text']
        assert r['isError'] is True and OLD_VERSION in text and 'never replaced by ordinary use' in text and 'Unsupported' not in text, (op, r)


class Fake(http.server.BaseHTTPRequestHandler):
    """A library that answers `state` with whatever it is told to, and records every operation it is asked."""
    answer, seen = {}, []

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
        Fake.seen.append(body.get('op'))
        payload = json.dumps(Fake.answer if body.get('op') == 'state' else {'error': 'fake'}).encode()
        self.send_response(200 if body.get('op') == 'state' else 400)
        self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(payload))); self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *a):
        pass


try:
    # ---- 1. real accepted 11 library, ordinary 12 calls: described, never touched --------------------------------------------------------
    old, new = make('handoff-defer-', slow_tunnel=True)
    old_bridge, c = old_library(old)
    take = old.fake_take(c, 'finished fixture take')
    bridge, home = new_home(new)
    kept(old, c, home)                                            # an IDLE 11 library is kept too
    key = ui_key(bridge)
    refused_in_plain_words(bridge, key)
    assert bridge.action('state')['structuredContent']['version'] == OLD_VERSION, 'the operations the older library has still work'
    for _ in range(3):
        kept(old, c, bridge.tool('openrecorder_home', timeout=40))
    ok('an IDLE accepted 11 library is KEPT by ordinary 12 calls (repeated): same pid, still serving, described as 11 with page operations refused in plain words; no Quit is sent')
    status, _ = old.jpost(c, 'share', {'id': take['id'], 'minutes': 15}, timeout=10)
    assert status == 200 and old.jpost(c, 'state')[1]['shares'][0]['status'] == 'starting'
    kept(old, c, bridge.tool('openrecorder_home', timeout=40))
    again = old.jpost(c, 'state')[1]['shares']
    assert again and again[0]['status'] == 'starting', 'a share born after the last look is not lost: nothing quit the library'
    ok('a share created right after a 12 call survives further 12 calls (nothing ever quits the library)')
    assert old.jpost(c, 'unshare', {'id': take['id']})[0] == 200
    active = old.fake_take(c, 'take in progress', finish=False)
    kept(old, c, bridge.tool('openrecorder_home', timeout=40))
    assert old.jpost(c, 'state')[1]['active'] is not None
    assert old.jpost(c, 'native-finish', {'id': active['id'], 'duration': 1})[0] == 200
    ok('a library with a take in progress is kept')
    os.kill(c['pid'], signal.SIGSTOP)
    try:
        stuck = Bridge(new)
        assert 'result' in stuck.initialize()
        r = stuck.tool('openrecorder_home', timeout=40)
        assert r.get('isError') is True and 'busy' in r['content'][0]['text'], r
        assert alive(c['pid']) and old.descriptor()['pid'] == c['pid'] and len(old.hosts()) == 1 and not new.hosts(), 'no second host over a stalled owner'
    finally:
        os.kill(c['pid'], signal.SIGCONT)
    ok('a stalled (SIGSTOPped) library is neither replaced nor killed; the session reports it busy')

    # ---- 2. libraries that answer with malformed or odd state: kept, and never sent anything but `state` ----------------------------------------
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fake)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    idle = {'version': OLD_VERSION, 'active': None, 'shares': [], 'capture': {'phase': 'unavailable', 'generation': 0, 'known': False}}
    variants = {
        'idle older': idle, 'missing version': {k: v for k, v in idle.items() if k != 'version'}, 'garbage1': dict(idle, version='garbage1'), 'numeric version': dict(idle, version=11),
        'newer': dict(idle, version='0.7.0'), 'same version, no recorder block': dict(idle, version=NEW_VERSION), 'missing active': {k: v for k, v in idle.items() if k != 'active'},
        'missing shares': {k: v for k, v in idle.items() if k != 'shares'}, 'shares object': dict(idle, shares={'status': 'failed'}), 'share ready': dict(idle, shares=[{'status': 'ready'}]),
        'missing capture': {k: v for k, v in idle.items() if k != 'capture'}, 'capture known': dict(idle, capture={'phase': 'idle', 'known': True}), 'not an object': ['idle'],
    }
    probe = Fixture(NEW, 'handoff-fake-')
    cleanups.append((probe, probe, None))
    for name, answer in variants.items():
        Fake.answer, Fake.seen = answer, []
        (probe.home).mkdir(parents=True, exist_ok=True)
        (probe.home / 'rust-runtime.json').write_text(json.dumps({'pid': 1, 'origin': f'http://127.0.0.1:{server.server_address[1]}', 'key': 'k' * 64}))
        session = Bridge(probe)
        assert 'result' in session.initialize()
        for _ in range(2):
            r = session.tool('openrecorder_home', timeout=40)
        assert set(Fake.seen) <= {'state'}, f'{name}: ordinary use sent {sorted(set(Fake.seen))}'
        assert not r.get('isError') or name == 'not an object', (name, r)
        session.close(5)
    server.shutdown()
    ok('libraries answering with an idle, odd or malformed state (13 variants: missing or garbage version, missing active, missing or malformed shares, capture missing or known, not an object) are only ever asked `state`: never a Quit, never another operation')

    # ---- 3. the ONE controlled quiescent operator handoff, on a throw-away 11 library: precheck, normal Quit, 12 takes the lease ------------------
    old2, new2 = make('handoff-manual-', slow_tunnel=True)
    quiet_client, d = old_library(old2)                                 # an older plugin session, then silent
    assert old2.jpost(d, 'settings', {'preferences': {'cameraShape': 'wide', 'countdownSeconds': 0}})[0] == 200
    kept_take = old2.fake_take(d, 'recording to keep')
    before = old2.jpost(d, 'state')[1]
    listing = precheck.process_listing()
    mine = {os.getpid()}
    assert precheck.blockers(before, listing, NEW_VERSION, mine) == [], precheck.blockers(before, listing, NEW_VERSION, mine)
    ok('precheck: an idle accepted 11 library (0 shares, no take, no app, callers quiet by the operator) reads as idle')
    # The same precheck refuses every state that is not clearly idle (strict, missing is not null).
    def blocked(state, listing_text=listing):
        return precheck.blockers(state, listing_text, NEW_VERSION, mine)
    assert blocked(dict(before, active={'status': 'recording'})) and blocked({k: v for k, v in before.items() if k != 'active'}) and blocked(dict(before, version='garbage1')) and blocked(dict(before, version=NEW_VERSION))
    for odd in ('x0.6.0-preview.9y', '0.6.0-preview.9-extra', ' 0.6.0-preview.9', '0.6.0-preview.9\n', 'v0.6.0-preview.9', '0.6.0-preview.', '0.6-preview.9', '0.6.0-preview.9.1'):
        assert blocked(dict(before, version=odd)), f'a malformed version must not count as older: {odd!r}'
    assert blocked(dict(before, shares=[{'status': 'starting'}])) and blocked(dict(before, shares=[{'status': 'ready'}])) and blocked(dict(before, shares=[{}])) and blocked(dict(before, shares=None)) and not blocked(dict(before, shares=[{'status': 'failed'}]))
    assert blocked(dict(before, capture={'phase': 'idle', 'known': True})) and blocked({k: v for k, v in before.items() if k != 'capture'}) and blocked(dict(before, capture={'phase': 'unavailable'}))
    app = '/Applications/OpenRecorder.app/Contents/MacOS/OpenRecorder'
    assert blocked(before, None) and blocked(before, [(4242, NEW, f'{NEW} --desktop --config /x/rust-connection.json')]) and blocked(before, [(4243, app, 'OpenRecorder')]) and blocked(before, [(4244, '/Applications/OpenRecorder.app/Contents/Helpers/ffmpeg', 'ffmpeg -i x')]) and blocked(before, [('x', 'y')])
    assert not blocked(before, [(4245, app, 'OpenRecorder --stdio'), (4246, app, 'OpenRecorder --library-host'), (4247, app, 'OpenRecorder --doctor'), (4248, '/bin/zsh', f'zsh -c {NEW} --desktop and /x/OpenRecorder'), (4249, '/Applications/ChatGPT.app/Helpers/handler', '--database=/Users/u/Library/ArcitAI OpenRecorder Test/Crashpad')])
    assert precheck.version_key('garbage1') is None and precheck.version_key('0.6.0-preview.9') < precheck.version_key('0.6.0-preview.12') < precheck.version_key('0.6.0')
    ok('precheck is strict: a take, a missing or malformed field, a missing/garbage/equal version, a live, starting or malformed share, a reporting app, a running app or helper, or an unreadable process list each mean "not idle"')
    # The operator's step: the library's own normal Quit (an existing public operation), nothing else.
    status, quitting = old2.jpost(d, 'quit')
    assert status == 200 and quitting['quitting'] is True, (status, quitting)
    assert old2.wait_gone(d['pid'], 15), 'the older library quit by itself'
    assert not old2.hosts() and not new2.hosts(), 'nothing runs until a 12 session starts the library'
    # A 12 plugin session (the next call) starts the new library, which takes the lease with the same user data.
    session, started = new_home(new2)
    backend = started['structuredContent']['backend']
    assert (backend['current'], backend['pageOps'], backend['handoff'], backend['version']) == (True, True, 'none', NEW_VERSION), backend
    c2 = new2.descriptor()
    assert c2['pid'] != d['pid'] and new2.is_own_host(c2['pid']) and new2.hosts() == [c2['pid']]
    state = new2.jpost(c2, 'state')[1]
    assert state['version'] == NEW_VERSION and state['recorder']['state'] == 'not-running'
    assert [r['id'] for r in state['recordings']] == [r['id'] for r in before['recordings']] and state['settings']['cameraShape'] == 'wide' and state['settings']['countdownSeconds'] == 0
    assert (pathlib.Path(kept_take['directory']) / 'recording.mp4').read_bytes() == b'not media: synthetic fixture bytes'
    with open(new2.home / 'rust-host.lease', 'a+') as lease:
        try:
            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            raise AssertionError('the new library does not hold the lease')
        except BlockingIOError:
            pass
    key2 = ui_key(session)
    r = session.ui('open-privacy-settings', {'kind': 'https://example.com'}, key=key2)
    assert r['isError'] is True and 'Invalid privacy page' in r['content'][0]['text'], r      # reaches the 12 backend, not refused as "older"
    if not IN_BUNDLE:
        r = session.ui('open-recorder', {}, key=key2)
        assert r['isError'] is True and 'packaged OpenRecorder app' in r['content'][0]['text'], r
    ok('controlled quiescent handoff: normal Quit of the idle 11 library, then 12 takes the OS lease: state is 12 with a recorder block, same recordings and bytes, same settings, one host, and the new page operations reach the 12 backend')
    # The older plugin session that stayed open adopts the 12 library; it keeps its old page operations until it is refreshed.
    r = quiet_client.tool('openrecorder_home', timeout=40)
    assert not r.get('isError') and r['structuredContent']['version'] == NEW_VERSION and new2.hosts() == [c2['pid']] and not old2.hosts()
    assert quiet_client.ui('open-recorder', {}, key='x')['isError'] is True
    ok('the older stdio client that stayed open adopts the 12 library (one host) and cannot name the new page operations until refreshed')

    # ---- 4. the recorder app has reported: kept, as everywhere else -----------------------------------------------------------------------------
    old3, new3 = make('handoff-app-')
    _, e = old_library(old3)
    assert old3.jpost(e, 'native-window-ready', {'pid': os.getpid()})[0] == 200
    assert old3.jpost(e, 'native-progress', {'phase': 'idle', 'generation': 0, 'launch': 'launch-handoff-0001'})[0] == 200
    _, kept_app = new_home(new3)
    kept(old3, e, kept_app)
    ok('a library whose recorder app has reported is kept')
    print(json.dumps({'passed': True, 'checks': checks}, indent=2))
finally:
    for old, new, scratch in cleanups:
        for fx in {id(old): old, id(new): new}.values():
            for b in fx.bridges:
                try:
                    b.close(5)
                except Exception:
                    pass
        try:
            old.stop_hosts()
            new.stop_hosts()
        finally:
            shutil.rmtree(old.root, ignore_errors=True)
            if scratch:
                shutil.rmtree(scratch, ignore_errors=True)
