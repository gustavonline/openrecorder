"""MCP bridge protocol contract over an isolated home: negotiation, standard errors, bounds, hints, reconnect.

Usage: python3 tests/rust-mcp-lifecycle.py /path/to/openrecorder-core
Protocol only; no native launch, hardware, browser or tunnel. Never prints the owner key.
"""
import json, sys
from hostfixture import HOME_URI, Bridge, Fixture, alive

fx = Fixture(sys.argv[1], 'recorder-mcp-life-')
checks = []
try:
    b = Bridge(fx)
    # Initialize negotiates either supported revision, falls back to the latest, and rejects a missing version.
    for requested, expected in (('2025-11-25', '2025-11-25'), ('2026-07-28', '2026-07-28'), ('2024-11-05', '2026-07-28')):
        r = b.initialize(requested)['result']
        assert r['protocolVersion'] == expected, (requested, r)
        assert r['serverInfo']['name'] == 'OpenRecorder' and r['capabilities']['tools'] == {}
    assert b.rpc('initialize', {})['error']['code'] == -32602
    discover = b.rpc('server/discover')['result']
    assert discover['supportedVersions'] == ['2026-07-28'] and discover['resultType'] == 'complete'
    assert discover['_meta']['io.modelcontextprotocol/serverInfo']['name'] == 'OpenRecorder'
    checks.append('initialize negotiates 2025-11-25/2026-07-28 (latest fallback); discover contract kept')

    # Caching hints (required by 2026-07-28) are present, private and stale on every cacheable result, both versions.
    for version in ('2025-11-25', '2026-07-28'):
        meta = {'_meta': {'io.modelcontextprotocol/protocolVersion': version}}
        for method in ('server/discover', 'tools/list', 'resources/list', 'resources/templates/list'):
            r = b.rpc(method, meta)['result']
            assert r['ttlMs'] == 0 and r['cacheScope'] == 'private', (method, version, r)
        read = b.rpc('resources/read', {'uri': HOME_URI, **meta})['result']
        assert read['ttlMs'] == 0 and read['cacheScope'] == 'private'
        assert 'ttlMs' not in b.rpc('ping')['result'] and 'ttlMs' not in b.action('state')
    tools = b.rpc('tools/list')['result']['tools']
    assert [t['name'] for t in tools] == ['openrecorder_home', 'openrecorder_action', 'openrecorder_ui']
    assert tools[0]['_meta']['ui']['resourceUri'] == HOME_URI
    checks.append('ttlMs=0/cacheScope=private on discover, tools/list, resources/list, templates/list, resources/read; not on ping/tools/call')

    # Standard JSON-RPC errors are distinguished; notifications and client replies never get a response.
    def code(raw):
        b.send(raw if isinstance(raw, bytes) else json.dumps(raw).encode() + b'\n')
        m = b.recv(10)
        return m['error']['code'], m['id']
    assert code(b'{not json\n') == (-32700, None)
    assert code(b'\xff\xfe\n') == (-32700, None)
    assert code([1, 2]) == (-32600, None)
    assert code({'jsonrpc': '2.0', 'id': 5}) == (-32600, 5)
    assert code({'jsonrpc': '1.0', 'id': 6, 'method': 'ping'}) == (-32600, 6)
    assert code({'jsonrpc': '2.0', 'id': None, 'method': 'ping'}) == (-32600, None)
    assert code({'jsonrpc': '2.0', 'id': 7, 'method': 'nope'}) == (-32601, 7)
    assert code({'jsonrpc': '2.0', 'id': 8, 'method': 'tools/call', 'params': {}}) == (-32602, 8)
    assert code({'jsonrpc': '2.0', 'id': 9, 'method': 'tools/call', 'params': {'name': 'nope'}}) == (-32602, 9)
    assert code({'jsonrpc': '2.0', 'id': 10, 'method': 'resources/read', 'params': {'uri': 'ui://other'}}) == (-32602, 10)
    for silent in ({'jsonrpc': '2.0', 'method': 'notifications/initialized'}, {'jsonrpc': '2.0', 'method': 'ping'},
                   {'jsonrpc': '2.0', 'id': 11, 'result': {}}, {'method': 5}):
        b.send(silent)
        assert b.silent(), silent
    b.send(b'\n   \n')
    assert b.silent()
    assert b.rpc('ping')['result'] == {}
    checks.append('-32700/-32600/-32601/-32602 differentiated; notifications, replies and blank lines are silent')

    # Oversized input is rejected safely and the bridge keeps working; exactly 131072 bytes is accepted.
    frame = b'{"jsonrpc":"2.0","id":"exact","method":"ping","params":{"x":""}}'
    exact = frame.replace(b'""', b'"' + b'a' * (131072 - len(frame)) + b'"')
    assert len(exact) == 131072
    b.send(exact + b'\n')
    assert b.recv(10)['id'] == 'exact'
    b.send(b'x' * 131073 + b'\n')
    rejected = b.recv(10)
    assert rejected['error']['code'] == -32600 and rejected['id'] is None
    b.send(b'y' * 600000)           # no newline: discarded without being kept...
    b.send(b'\n')                   # ...until the line ends
    assert b.recv(10)['error']['code'] == -32600
    assert b.rpc('ping')['result'] == {}
    assert b.p.poll() is None
    assert 'input limit' in open(b.log.name).read()
    checks.append('131072-byte line accepted; larger lines get -32600 and are drained; bridge survives')

    # Internal capture lifecycle stays blocked for agents; the visible schema never lists it.
    for op in ('native-begin', 'native-finish', 'native-abort', 'capture-command', 'native-progress'):
        r = b.action(op)
        assert r['isError'] and 'not available to agents' in r['content'][0]['text']
    assert not b.action('state').get('isError') and b.action('state')['structuredContent']['active'] is None
    assert b.action('state', None)['structuredContent']['version']
    bad = b.tool('openrecorder_action', {'op': 'state', 'data': 3})
    assert bad['isError'] and 'object' in bad['content'][0]['text']
    checks.append('agents cannot invoke internal capture lifecycle; invalid data rejected')

    # Same bridge survives a service Quit (reconnect only after the host really went away).
    old = fx.descriptor()['pid']
    assert not b.action('quit').get('isError')
    assert not b.tool('openrecorder_home').get('isError')
    new = fx.descriptor()['pid']
    assert new != old and fx.wait_gone(old, 15)
    checks.append('same MCP client reconnects after service Quit')
    # Closing stdin ends this bridge (exit 0) without touching the host.
    assert b.close() == 0 and alive(new)
    checks.append('EOF exits the bridge cleanly and leaves the host running')
    print(json.dumps({'passed': True, 'checks': checks}, indent=2))
finally:
    fx.cleanup()
