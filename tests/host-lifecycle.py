"""Isolated host lifecycle and credential-boundary test. Protocol and local HTTP only: no native launch.

Usage: python3 tests/host-lifecycle.py /path/to/openrecorder-core
Runs against a throw-away HOME/OPENRECORDER_HOME with a disabled tunnel executable. The owner key is read only from
the fixture's own descriptor and is never printed. open-library (which would open a browser) is never called.
"""
import json, os, re, signal, sys, time, urllib.request
from hostfixture import Bridge, Fixture, HOME_URI, alive

fx = Fixture(sys.argv[1], 'recorder-host-life-')
checks = []


def ok(name):
    checks.append(name)


def settle(cond, what, timeout=10):
    end = time.monotonic() + timeout
    while not cond():
        assert time.monotonic() < end, f'timed out: {what}'
        time.sleep(0.1)


try:
    # 1. Concurrent cold bridges converge on one host.
    bridges = [Bridge(fx) for _ in range(6)]
    for b in bridges:
        assert 'result' in b.initialize()
    homes = [b.tool('openrecorder_home') for b in bridges]
    assert all(not h.get('isError') for h in homes), homes
    origins = {h['structuredContent']['origin'] for h in homes}
    assert len(origins) == 1, 'bridges disagree about the host'
    c = fx.descriptor()
    settle(lambda: len(fx.hosts()) == 1, 'redundant starters to exit')
    time.sleep(1.0)
    assert fx.hosts() == [c['pid']], fx.hosts()
    assert c['origin'] in origins
    ok('six concurrent cold bridges -> one host pid, one origin, redundant starters gone')

    # 2. Passive initialization and listing: no UI, helper, journal or storage; stdout is protocol only.
    b = bridges[0]
    for version in ('2025-11-25', '2026-07-28'):
        assert 'result' in b.rpc('tools/list', {'_meta': {'io.modelcontextprotocol/protocolVersion': version}})
    b.rpc('resources/list'); b.rpc('ping')
    view = b.rpc('resources/read', {'uri': HOME_URI})
    assert 'result' in view
    state = b.action('state')['structuredContent']
    assert state['active'] is None and state['recordings'] == [] and state['shares'] == []
    assert not (fx.home / 'native-events.jsonl').exists(), 'passive start wrote a journal'
    assert not (fx.home / 'onboarding.json').exists()
    assert not (fx.user / 'Movies').exists() and not (fx.user / 'Videos').exists(), 'storage root was created'
    assert fx.children_of(c['pid']) == [], 'host started a child (helper/UI)'
    toolbar = [a for p, pp, st, a in __import__('hostfixture').ps() if a.startswith(fx.binary) and any(f in a for f in ('--desktop', '--toolbar', '--dev-toolbar'))]
    assert toolbar == [], 'the native desktop app was launched'
    ok('initialize/list/read/state leave no journal, storage, child, helper or desktop app; stdout parsed as JSON-RPC only')

    # 3. The model-visible Home result and the embedded resource never carry the owner key.
    home_raw = json.dumps(homes[0]) + json.dumps(view)
    assert c['key'] not in home_raw, 'owner key reached model-visible output'
    assert 'url' not in homes[0]['structuredContent'] and '#' + c['key'] not in home_raw
    assert homes[0]['structuredContent']['origin'] == c['origin']
    assert 'open-library' in homes[0]['structuredContent']['browser']
    assert homes[0]['_meta']['ui']['resourceUri'] == HOME_URI
    modern = b.tool('openrecorder_home', _meta={'io.modelcontextprotocol/protocolVersion': '2026-07-28'})
    assert modern['_meta']['ui']['resourceUri'] == HOME_URI, 'UI metadata lost under 2026 wrapping'
    assert modern['_meta']['io.modelcontextprotocol/serverInfo']['name'] == 'OpenRecorder'
    assert modern['_meta']['openai/outputTemplate'] == HOME_URI
    ok('Home has no key/url, keeps UI metadata and merges serverInfo for 2026-07-28')

    # 4. A scoped media capability opens media only; it can never run an owner or native operation.
    take = fx.fake_take(c, 'media fixture')
    text = view['result']['contents'][0]['text']
    assert c['key'] not in text
    media = re.search(r'"mediaKey":"([0-9a-f]+)"', text)
    assert media, 'embedded resource has no media capability'
    media = media.group(1)
    assert media != c['key'] and view['result']['ttlMs'] == 0 and view['result']['cacheScope'] == 'private'
    url = f"{c['origin']}/file/{take['id']}/recording.mp4"
    def get(u, headers=None, method='GET', data=None):
        return fx.http(urllib.request.Request(u, headers=headers or {}, method=method, data=data))
    assert get(url + '?media=' + media)[0] == 200
    assert get(url + '?media=' + media, method='HEAD')[0] == 200
    assert get(url)[0] == 403 and get(url + '?media=' + 'x' * len(media))[0] == 403
    assert get(url + '?key=' + c['key'])[0] == 200
    for path in ('/api/state', '/api/view'):
        assert get(c['origin'] + path + '?media=' + media)[0] == 403, path
        assert get(c['origin'] + path, {'Authorization': 'Bearer ' + media})[0] == 403, path
    for op in ('state', 'open-studio', 'native-begin', 'native-command', 'quit', 'prepare-recording'):
        status, _ = fx.post({'origin': c['origin'], 'key': media}, op, timeout=5)
        assert status == 403, (op, status)
        status, _ = fx.post({'origin': c['origin'], 'key': media}, op, timeout=5, headers={'Origin': 'http://evil.example'})
        assert status == 403
    assert get(url + '?media=' + media, method='POST', data=b'{}')[0] == 403
    assert get(f"{c['origin']}/file/{take['id']}/..%2Fsettings.json?media={media}")[0] in (403, 404)
    status, body = fx.jpost(c, 'prepare-recording')
    assert status == 400 and 'Unsupported' in body['error'], 'prepare-recording must not exist'
    assert fx.jpost(c, 'state')[0] == 200
    ok('media capability: GET/HEAD /file only; /api, native ops, quit and POST refused; owner key still works; no prepare op')

    # 4a. The page-only tool, the app window's session key and the media headers (real host, real bridge, no hardware).
    ui_key = re.search(r'"uiKey":"([0-9a-f]+)"', text)
    assert ui_key and ui_key.group(1) not in (c['key'], media), 'the window’s own copy of the page carries its session key'
    ui_key = ui_key.group(1)
    assert text.count(ui_key) == 1 and c['key'] not in text
    tools = {t['name']: t for t in b.rpc('tools/list')['result']['tools']}
    assert list(tools) == ['openrecorder_home', 'openrecorder_action', 'openrecorder_ui']
    assert tools['openrecorder_ui']['_meta']['ui']['visibility'] == ['app']
    assert all(t['title'] == 'OpenRecorder' or t['name'] == 'openrecorder_ui' or t['title'] == 'OpenRecorder library' for t in tools.values())
    assert 'OpenRecorder_Home' not in json.dumps(tools)
    for args in ({}, {'ui': 'nope'}):
        r = b.tool('openrecorder_ui', {'op': 'media-session', 'data': {}, **args})
        assert r['isError'] is True and 'belongs to the OpenRecorder window' in r['content'][0]['text'], args
    r = b.ui('start-recording', {}, key=ui_key)
    assert r['isError'] is True, 'a start with no envelope is refused even with the key'
    session = b.ui('media-session', key=ui_key)['structuredContent']
    assert session['mediaKey'] == media and session['origin'] == c['origin'] and c['key'] not in json.dumps(session)
    piece = b.ui('media-read', {'id': take['id'], 'name': 'recording.mp4', 'start': 4, 'length': 7}, key=ui_key)['structuredContent']
    import base64
    FAKE = b'not media: synthetic fixture bytes'
    assert (piece['total'], piece['start'], piece['length'], piece['mime']) == (len(FAKE), 4, 7, 'video/mp4')
    assert base64.b64decode(piece['data']) == FAKE[4:11]
    assert b.ui('media-read', {'id': take['id'], 'name': 'recording.mp4', 'start': len(FAKE)}, key=ui_key)['isError'] is True
    assert b.ui('media-read', {'id': take['id'], 'name': 'library.json'}, key=ui_key)['isError'] is True
    for call in (b.action('state'), b.tool('openrecorder_home')):
        assert ui_key not in json.dumps(call) and media not in json.dumps(call) and c['key'] not in json.dumps(call), 'a model-visible answer carried a credential'
    # Headers: only a request that presents the media capability is told it may be read across origins.
    status, body, headers = fx.http_full(urllib.request.Request(url + '?media=' + media))
    assert status == 200 and headers.get('cross-origin-resource-policy') == 'cross-origin' and headers.get('access-control-allow-origin') == '*'
    status, body, headers = fx.http_full(urllib.request.Request(url + '?key=' + c['key']))
    assert status == 200 and 'access-control-allow-origin' not in headers and 'cross-origin-resource-policy' not in headers, 'owner reads get no cross-origin grant'
    status, body, headers = fx.http_full(urllib.request.Request(url + '?media=' + media, method='OPTIONS', headers={'Access-Control-Request-Method': 'GET', 'Access-Control-Request-Private-Network': 'true'}))
    assert status == 204 and headers.get('access-control-allow-private-network') == 'true' and headers.get('access-control-allow-methods') == 'GET, HEAD, OPTIONS'
    assert 'authorization' not in headers.get('access-control-allow-headers', '').lower()
    for refused in (url, url + '?media=' + 'x' * len(media), c['origin'] + '/api/state?media=' + media):
        assert fx.http_full(urllib.request.Request(refused, method='OPTIONS'))[0] == 403, refused
    status, _, headers = fx.http_full(urllib.request.Request(url + '?key=' + c['key'], method='OPTIONS'))
    assert status != 204 and 'access-control-allow-origin' not in headers and 'access-control-allow-private-network' not in headers, 'the owner key is never answered as a cross-origin preflight'
    ok('page-only tool: hidden, keyed to the window, bounded media reads; CORS/PNA only with the media capability; no credential in any model-visible answer')

    # 4b. Agents cannot reach internal or removed operations through MCP either.
    for op in ('native-begin', 'native-finish', 'native-command', 'prepare-recording', 'native-window-ready'):
        r = b.action(op)
        assert r['isError'] is True and 'not available to agents' in r['content'][0]['text'], op
    enum = next(t for t in b.rpc('tools/list')['result']['tools'] if t['name'] == 'openrecorder_action')['inputSchema']['properties']['op']['enum']
    assert 'open-studio' in enum and 'prepare-recording' not in enum and not any(o.startswith('native-') for o in enum)
    ok('MCP internal ops and prepare-recording blocked; open-studio remains')

    # 4d. Owner-only capture controls (menu / Home): never an agent operation, never reachable with the media
    # capability or from a foreign origin; validated, short-lived and delivered only to the registered native app.
    for op in ('capture-command', 'native-progress'):
        r = b.action(op, {'command': 'record'})
        assert r['isError'] is True and 'not available to agents' in r['content'][0]['text'], op
        status, _ = fx.post({'origin': c['origin'], 'key': media}, op, {'command': 'record'}, timeout=5)
        assert status == 403, (op, status)
        status, _ = fx.post(c, op, {'command': 'record'}, timeout=5, headers={'Origin': 'http://evil.example'})
        assert status == 403, (op, status)
    assert not any(o in enum for o in ('capture-command', 'native-progress', 'record'))
    status, view = fx.jpost(c, 'state')
    assert view['capture']['phase'] == 'unavailable' and view['capture']['known'] is False, 'never invented before the app reports'
    status, _ = fx.jpost(c, 'native-window-ready', {'pid': os.getpid()})
    assert status == 200
    LAUNCH = 'launch-lifecycle-0001'
    def control(body, generation=None, nonce=None, **override):
        data = dict(body, launch=LAUNCH, nonce=nonce or ('n' + os.urandom(8).hex()), expires=int(time.time() * 1000) + 4000)
        if generation is not None and 'generation' not in body:
            data['generation'] = generation
        data.update(override)
        return data
    # A request is refused until the app has reported its launch: there is nothing to bind it to.
    status, body = fx.jpost(c, 'capture-command', control({'command': 'mirror', 'value': True}))
    assert status == 400 and 'restarted or' in body['error'], body
    status, _ = fx.jpost(c, 'native-progress', {'phase': 'idle', 'generation': 1, 'launch': LAUNCH})
    assert status == 200
    for bad in ({}, {'command': 'delete-everything'}, {'command': 'input', 'kind': 'screen', 'on': True}, {'command': 'source', 'kind': 'display'},
                {'command': 'shape', 'value': 'oval'}, {'command': 'fps', 'value': 120}, {'command': 'stop', 'generation': 'x'}):
        status, body = fx.jpost(c, 'capture-command', control(bad, 1))
        assert status == 400, (bad, status)
    # The session envelope is mandatory, and bound to this launch, this take, a short expiry and a single use.
    assert fx.jpost(c, 'capture-command', {'command': 'mirror', 'value': True})[0] == 400, 'a bare request is refused'
    assert fx.jpost(c, 'capture-command', control({'command': 'record'}))[0] == 400, 'Record must name the take it saw'
    assert fx.jpost(c, 'capture-command', control({'command': 'record'}, 1, launch='launch-from-before-0'))[0] == 400, 'another launch'
    assert fx.jpost(c, 'capture-command', control({'command': 'record'}, 0))[0] == 400, 'an older take'
    assert fx.jpost(c, 'capture-command', control({'command': 'record'}, 1, expires=int(time.time() * 1000) - 5))[0] == 400, 'already expired'
    assert fx.jpost(c, 'capture-command', control({'command': 'record'}, 1, expires=int(time.time() * 1000) + 600000))[0] == 400, 'a long-lived request'
    request = control({'command': 'record'}, 1, nonce='lifecycle-once-0001')
    status, body = fx.jpost(c, 'capture-command', request)
    assert status == 200 and body['queued'] is True and body['ttlMs'] == 5000, body
    status, body = fx.jpost(c, 'capture-command', dict(request, expires=request['expires'] + 1))
    assert status == 400 and 'already used' in body['error'], 'a replay with fresh-looking fields is refused'
    status, delivered = fx.jpost(c, 'native-command', {'pid': os.getpid()}, timeout=30)
    payload = delivered['human']['payload']
    assert delivered['command'] == 'human' and delivered['human']['name'] == 'record' and delivered['human']['issuedAt'] > 0, delivered
    assert (payload['launch'], payload['generation'], payload['nonce']) == (LAUNCH, 1, 'lifecycle-once-0001') and payload['expires'] == request['expires'], payload
    status, _ = fx.jpost(c, 'native-progress', {'phase': 'recording', 'generation': 2, 'elapsedMs': 1500, 'launch': LAUNCH})
    assert status == 200
    status, view = fx.jpost(c, 'state')
    assert (view['capture']['phase'], view['capture']['generation'], view['capture']['elapsedMs'], view['capture']['launch']) == ('recording', 2, 1500, LAUNCH)
    status, late = fx.jpost(c, 'native-progress', {'phase': 'idle', 'generation': 1, 'launch': LAUNCH})
    assert late.get('accepted') is False, 'an older take cannot overwrite a newer one'
    # A restarted app publishes a new launch, which starts its own numbering; requests made for the old one are refused.
    status, accepted = fx.jpost(c, 'native-progress', {'phase': 'idle', 'generation': 0, 'launch': 'launch-lifecycle-0002'})
    assert status == 200 and accepted.get('accepted') is True
    status, body = fx.jpost(c, 'capture-command', control({'command': 'record'}, 0, nonce='lifecycle-late-0001'))
    assert status == 400 and 'restarted or' in body['error'], 'a request delivered after the restart is not a fresh start'
    status, view = fx.jpost(c, 'state')
    assert view['capture']['phase'] == 'idle' and 'elapsedMs' not in view['capture'], 'the terminal report carries no timer'
    status, _ = fx.jpost(c, 'native-window-ready', {'pid': os.getpid()})
    ok('owner-only capture controls: refused for agents, media capability and foreign origin; validated; bound to the app launch, the take, a short expiry and a single use; replay and post-restart delivery refused; terminal report carries no timer')

    # 4c. Preference keys and metadata-only folders over the owner HTTP API.
    status, st = fx.jpost(c, 'settings', {'preferences': {'rememberInputs': False, 'countdownSeconds': 0, 'cameraSize': 168, 'cameraShape': 'wide'}})
    assert status == 200 and st['settings']['rememberInputs'] is False and st['settings']['countdownSeconds'] == 0, st
    assert 'countdown' not in st['settings'] and st['settings']['cameraShape'] == 'wide'
    for bad in ({'cameraSize': 150}, {'countdownSeconds': 5}, {'rememberInputs': 'no'}, {'cameraShape': 'oval'}, {'countdown': 3}):
        status, body = fx.jpost(c, 'settings', {'preferences': bad})
        assert status == 400 and 'error' in body, bad
    assert fx.jpost(c, 'state')[1]['settings']['cameraSize'] == 168, 'a rejected request changed a setting'
    status, folder = fx.jpost(c, 'folder', {'name': 'Fixture folder'})
    assert status == 200
    status, moved = fx.jpost(c, 'update', {'id': take['id'], 'patch': {'folder': folder['id']}})
    assert status == 200 and moved['folder'] == folder['id'] and moved['directory'] == take['directory'], 'filing must not move bytes'
    assert os.path.isfile(os.path.join(take['directory'], 'recording.mp4'))
    status, body = fx.jpost(c, 'update', {'id': take['id'], 'patch': {'folder': 'missing'}})
    assert status == 400 and fx.jpost(c, 'state')[1]['recordings'][0]['folder'] == folder['id']
    ok('settings keys validated over HTTP (invalid rejected, nothing changed); folder filing is metadata only')

    # 5. Descriptor files removed or damaged while the owner lives are restored by the owner.
    for damage in ('delete', 'corrupt'):
        for name in ('rust-runtime.json', 'rust-connection.json'):
            p = fx.home / name
            p.unlink() if damage == 'delete' else p.write_text('{not json')
        settle(lambda: (fx.descriptor() or {}).get('pid') == c['pid'] and (fx.home / 'rust-connection.json').exists(), f'descriptor restored after {damage}', 8)
        assert b.action('state').get('isError') is not True
    ok('owner restores deleted/corrupt descriptors; nobody is stranded by a live-but-unfindable host')

    # 6. EOF ends bridges, never the host.
    codes = [x.close() for x in bridges]
    assert codes == [0] * 6, codes
    assert alive(c['pid']) and fx.jpost(c, 'state')[0] == 200
    ok('EOF on every bridge exits 0 and leaves the host serving')

    # 7. Quit, then the same bridge reconnects to a new owner with no sleep in between.
    q = Bridge(fx)
    assert 'result' in q.initialize()
    assert not q.action('quit').get('isError')
    after = q.action('state', timeout=40)
    assert not after.get('isError'), after
    c2 = fx.descriptor()
    assert c2['pid'] != c['pid'] and fx.wait_gone(c['pid'], 15)
    settle(lambda: fx.hosts() == [c2['pid']], 'single host after quit/reconnect')
    ok('Quit then immediate call: same bridge reconnects (new pid), old host gone, one host')

    # 8. Crash (SIGKILL of the verified own host) with a stale descriptor: same bridge and a fresh bridge recover.
    assert fx.is_own_host(c2['pid'])
    os.kill(c2['pid'], signal.SIGKILL)
    assert fx.wait_gone(c2['pid'], 10)
    assert (fx.home / 'rust-runtime.json').exists(), 'crash should leave a stale descriptor'
    again = q.action('state', timeout=40)
    assert not again.get('isError'), again
    c3 = fx.descriptor()
    assert c3['pid'] not in (c['pid'], c2['pid']) and alive(c3['pid'])
    fresh = Bridge(fx)
    assert 'result' in fresh.initialize()
    assert fresh.tool('openrecorder_home')['structuredContent']['origin'] == c3['origin']
    assert fx.hosts() == [c3['pid']]
    ok('SIGKILLed host + stale descriptor: existing and fresh bridges recover onto one new host')

    # 9. Stale descriptor naming a dead port with no host at all.
    assert not fresh.action('quit').get('isError')
    assert fx.wait_gone(c3['pid'], 15)
    (fx.home / 'rust-runtime.json').write_text(json.dumps({'pid': 1, 'origin': 'http://127.0.0.1:9', 'key': 'k' * 64}))
    third = Bridge(fx)
    assert 'result' in third.initialize(timeout=40)
    assert not third.action('state', timeout=40).get('isError')
    assert fx.descriptor()['pid'] not in (1, c3['pid'])
    ok('descriptor for a dead port with no owner is replaced by a new host')
    print(json.dumps({'passed': True, 'checks': checks}, indent=2))
finally:
    fx.cleanup()
