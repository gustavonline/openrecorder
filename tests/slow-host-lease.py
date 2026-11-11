"""A stalled or busy host keeps the home: no second host, no second Store, no fake active take interrupted.

Usage: python3 tests/slow-host-lease.py /path/to/openrecorder-core
Deterministic stand-in for the 16.49 s duplicate-host reproduction: the owner host of an isolated home is suspended
(SIGSTOP), so every state probe times out, exactly like a host blocked inside a long operation. Only an isolated
home, a metadata-only fake active take and the normal Quit are used; no UI, hardware, tunnel or owner data.
"""
import json, os, signal, subprocess, sys, time
from hostfixture import Bridge, Fixture, alive

fx = Fixture(sys.argv[1], 'recorder-slow-host-')
started = time.monotonic()
stopped = None
report = {'binary': os.path.basename(fx.binary), 'checks': []}
try:
    b1 = Bridge(fx)
    assert 'result' in b1.initialize()
    c1 = fx.descriptor()
    assert c1 and alive(c1['pid']) and fx.is_own_host(c1['pid'])
    fx.fake_take(c1, 'completed fixture')
    active = fx.fake_take(c1, 'metadata-only active fixture', finish=False)
    library = fx.home / 'library.json'

    def statuses():
        return {r['id']: r['status'] for r in json.loads(library.read_text())['recordings']}
    assert statuses()[active['id']] == 'recording'

    os.kill(c1['pid'], signal.SIGSTOP)
    stopped = c1['pid']
    # A raw second `--library-host` and a fresh bridge both start while the owner cannot answer anything.
    raw = subprocess.Popen([fx.binary, '--library-host'], env=fx.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL)
    b2 = Bridge(fx)
    assert 'result' in b2.initialize(timeout=40), 'initialize must not depend on a responsive host'
    busy = b2.action('state', timeout=40)
    assert busy.get('isError') is True, 'a stalled owner is reported as busy, not replaced'
    assert 'busy' in busy['content'][0]['text'].lower() or 'did not' in busy['content'][0]['text'].lower(), busy
    code = raw.wait(timeout=30)
    assert code == 0, f'redundant host exited with {code}'
    report['checks'].append('raw second host and second bridge adopted nothing while the owner was stalled')

    after = fx.descriptor()
    assert after['pid'] == c1['pid'] and after['origin'] == c1['origin'], 'descriptor was republished'
    assert [p for p in fx.hosts() if p != c1['pid']] == [], 'a second live host exists'
    assert statuses()[active['id']] == 'recording', 'a second Store interrupted the active take'
    report['checks'].append('one live host pid, descriptor unchanged, fake active take still "recording" on disk')

    os.kill(c1['pid'], signal.SIGCONT)
    stopped = None
    state = b2.action('state', timeout=40)
    assert not state.get('isError'), state
    assert state['structuredContent']['active']['id'] == active['id']
    assert fx.descriptor()['pid'] == c1['pid']
    report['checks'].append('after resume the same host answers and still reports the fake active take')

    # Quit refuses while the take is active; after aborting only the fake entry it succeeds and frees the home.
    refused = b2.action('quit')
    assert refused.get('isError') is True and alive(c1['pid'])
    status, _ = fx.jpost(c1, 'native-abort', {'id': active['id'], 'error': 'metadata-only test complete'})
    assert status == 200
    assert not b2.action('quit').get('isError')
    assert fx.wait_gone(c1['pid'], 15), 'host did not exit after Quit'
    # Legitimate start after a normal quit: the same bridge reconnects to a new owner.
    again = b2.action('state', timeout=40)
    assert not again.get('isError'), again
    c2 = fx.descriptor()
    assert c2['pid'] != c1['pid'] and alive(c2['pid'])
    assert fx.hosts().count(c2['pid']) == 1
    report['checks'].append('Quit was refused during the active take, then allowed; a legitimate new host started')
    report['passed'] = True
finally:
    if stopped:
        try:
            os.kill(stopped, signal.SIGCONT)
        except OSError:
            pass
    fx.cleanup()
    report['seconds'] = round(time.monotonic() - started, 1)
print(json.dumps(report, indent=2))
