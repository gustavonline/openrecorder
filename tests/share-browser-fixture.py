#!/usr/bin/env python3
"""Disposable browser fixture for the guest share page. TEST-ONLY: nothing here is a product route, flag or binary.

It runs the real guest handler (`GuestServer` over `Access`, via the ignored test `share_browser_fixture_server` in
engine/src/share_access.rs) on a loopback port, and puts a TLS front on https://127.0.0.1:<port> with a fresh,
disposable self-signed certificate (generated into the run directory, deleted on stop; never trusted system-wide, never
committed). The front copies bytes both ways and rewrites nothing, so the request the handler sees is exactly the request
the browser made (Host, Origin, Sec-Fetch-*, Referer, cookies, body). Synthetic credentials and a tiny synthetic video
encoded with ffmpeg from generated gradient frames (no capture, no recording of anyone). No real share, tunnel, cloudflared or production state.

  share-browser-fixture.py start   [--dir DIR] [--expiry never|finite:SECONDS|expired] [--policy no-referrer|same-origin]
  share-browser-fixture.py status  [--dir DIR]
  share-browser-fixture.py control [--dir DIR] [--policy no-referrer|same-origin] [--revoke]
  share-browser-fixture.py selfcheck [--dir DIR]     (read-only protocol check with Python, not a browser)
  share-browser-fixture.py stop    [--dir DIR]

`--policy no-referrer` serves the guest pages as preview.10 did (Referrer-Policy: no-referrer), so a real browser's form
POST carries `Origin: null` and is refused before the code is read; `same-origin` is the fix. The policy can be switched
while running with `control` (reload the page afterwards: the policy is read from the response that served the form).
Expiry modes: `never` (no deadline while the fixture runs), `finite:N` (ends N seconds after start), `expired`
(already ended: shows the ended page). `control --revoke` stops the link (the production `Access::stop`).
Wrong codes are limited to five per rolling minute by the real handler; wait a minute or restart after that.
"""
import argparse, json, os, pathlib, signal, socket, ssl, subprocess, sys, tempfile, threading, time

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_DIR = pathlib.Path(tempfile.gettempdir()) / 'openrecorder-share-browser-fixture'
TOKEN = '0123456789ABCDEF0123456789ABCDEF'      # synthetic, fixed, public
CODE = 'K7QM4P2X'                                # synthetic, fixed, public (shown K7QM-4P2X)
FFMPEG_CANDIDATES = [os.environ.get('OPENRECORDER_FFMPEG', ''), 'ffmpeg']


def files(d):
    return {n: d / n for n in ('ready.json', 'rust.port', 'control.json', 'stop', 'cert.pem', 'key.pem', 'video.mp4', 'fixture.pid', 'rust.log')}


def ffmpeg():
    for c in FFMPEG_CANDIDATES:
        if c:
            try:
                if subprocess.run([c, '-hide_banner', '-version'], capture_output=True).returncode == 0:
                    return c
            except OSError:
                pass
    sys.exit('ffmpeg not found (set OPENRECORDER_FFMPEG)')


def pump(a, b):
    try:
        while True:
            data = a.recv(65536)
            if not data:
                break
            b.sendall(data)
    except OSError:
        pass
    finally:
        for s in (a, b):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


def front(listener, context, rust_port, stop):
    listener.settimeout(0.3)
    while not stop.is_set():
        try:
            raw, _ = listener.accept()
        except socket.timeout:
            continue
        except OSError:
            break

        def serve(raw=raw):
            try:
                tls = context.wrap_socket(raw, server_side=True)
                up = socket.create_connection(('127.0.0.1', rust_port), timeout=5)
                up.settimeout(None)
                t = threading.Thread(target=pump, args=(up, tls), daemon=True)
                t.start()
                pump(tls, up)
                t.join(2)
            except (OSError, ssl.SSLError):
                pass
            finally:
                try:
                    raw.close()
                except OSError:
                    pass
        threading.Thread(target=serve, daemon=True).start()


def start(args):
    d = pathlib.Path(args.dir); d.mkdir(parents=True, exist_ok=True)
    f = files(d)
    if f['fixture.pid'].exists():
        sys.exit(f'a fixture already runs here (pid {f["fixture.pid"].read_text().strip()}); stop it first')
    for n in ('ready.json', 'rust.port', 'stop', 'control.json'):
        f[n].unlink(missing_ok=True)
    subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', str(f['key.pem']), '-out', str(f['cert.pem']), '-days', '1',
                    '-subj', '/CN=127.0.0.1', '-addext', 'subjectAltName=IP:127.0.0.1'], check=True, capture_output=True)
    os.chmod(f['key.pem'], 0o600)
    # Synthetic frames made here (a moving gradient) and written as a raw-video temp file: no capture, no lavfi filter needed.
    w, h, n = 320, 180, 60
    frames = b''.join(bytes(c for y in range(h) for x in range(w) for c in ((x * 255 // w + i * 4) % 256, (y * 255 // h + i * 2) % 256, 96 + (i * 3) % 128)) for i in range(n))
    (d / 'frames.rgb').write_bytes(frames)
    subprocess.run([ffmpeg(), '-hide_banner', '-loglevel', 'error', '-y', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{w}x{h}', '-r', '15', '-i', str(d / 'frames.rgb'), '-pix_fmt', 'yuv420p',
                    '-c:v', 'libx264', '-preset', 'ultrafast', '-movflags', '+faststart', str(f['video.mp4'])], check=True)
    (d / 'frames.rgb').unlink(missing_ok=True)
    listener = socket.socket(); listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(('127.0.0.1', 0)); listener.listen(32)
    tls_port = listener.getsockname()[1]
    f['control.json'].write_text(json.dumps({'referrerPolicy': args.policy}))
    env = dict(os.environ, FIXTURE_TLS_PORT=str(tls_port), FIXTURE_MEDIA=str(f['video.mp4']), FIXTURE_CONTROL=str(f['control.json']), FIXTURE_READY=str(f['rust.port']),
               FIXTURE_STOP=str(f['stop']), FIXTURE_TOKEN=TOKEN, FIXTURE_CODE=CODE, FIXTURE_EXPIRY=args.expiry, FIXTURE_POLICY=args.policy)
    env.setdefault('PATH', '/usr/bin:/bin')
    log = open(f['rust.log'], 'w')
    child = subprocess.Popen(['cargo', 'test', '--offline', '--features', 'desktop-ui', 'share_browser_fixture_server', '--', '--ignored', '--nocapture'],
                             cwd=ROOT / 'engine', env=env, stdout=log, stderr=subprocess.STDOUT)
    deadline = time.time() + 600
    while not f['rust.port'].exists():
        if child.poll() is not None or time.time() > deadline:
            listener.close(); child.kill(); f['key.pem'].unlink(missing_ok=True)
            sys.exit(f'the handler did not start; see {f["rust.log"]}')
        time.sleep(0.2)
    rust_port = int(f['rust.port'].read_text())
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(f['cert.pem'], f['key.pem'])
    stop = threading.Event()
    threading.Thread(target=front, args=(listener, context, rust_port, stop), daemon=True).start()
    f['fixture.pid'].write_text(str(os.getpid()))
    f['ready.json'].write_text(json.dumps({
        'ready': True, 'synthetic': True, 'url': f'https://127.0.0.1:{tls_port}/{TOKEN}', 'code': f'{CODE[:4]}-{CODE[4:]}', 'tlsPort': tls_port, 'handlerPort': rust_port,
        'expiry': args.expiry, 'referrerPolicy': args.policy, 'fixturePid': os.getpid(), 'handlerPid': child.pid, 'dir': str(d),
        'note': 'disposable self-signed certificate for 127.0.0.1 in this directory; trust only inside the disposable browser profile'}, indent=1))
    print(f['ready.json'].read_text(), flush=True)

    def finish(*_):
        stop.set(); f['stop'].write_text('stop')
        try:
            child.wait(15)
        except subprocess.TimeoutExpired:
            child.kill()
        listener.close()
        for n in ('key.pem', 'fixture.pid', 'ready.json'):
            f[n].unlink(missing_ok=True)
        os._exit(0)
    signal.signal(signal.SIGTERM, finish); signal.signal(signal.SIGINT, finish)
    while child.poll() is None:
        time.sleep(0.5)
    finish()


def read_ready(d):
    try:
        return json.loads(files(pathlib.Path(d))['ready.json'].read_text())
    except OSError:
        sys.exit('no fixture is running in ' + str(d))


def control(args):
    f = files(pathlib.Path(args.dir)); read_ready(args.dir)
    state = json.loads(f['control.json'].read_text())
    if args.policy:
        state['referrerPolicy'] = args.policy
    if args.revoke:
        state['revoke'] = True
    f['control.json'].write_text(json.dumps(state))
    print(json.dumps(state))


def selfcheck(args):
    """Read-only protocol check over TLS with Python (trusting only the fixture's own certificate). It states the Origin a
    browser would send for the served policy (Fetch standard); it is NOT a browser result."""
    import http.client, urllib.parse
    r = read_ready(args.dir); f = files(pathlib.Path(args.dir))
    ctx = ssl.create_default_context(cafile=str(f['cert.pem']))
    origin = f'https://127.0.0.1:{r["tlsPort"]}'; path = f'/{TOKEN}'

    def call(method, route, headers=None, body=None):
        c = http.client.HTTPSConnection('127.0.0.1', r['tlsPort'], context=ctx, timeout=10)
        c.request(method, path + route, body=body, headers=headers or {})
        resp = c.getresponse(); data = resp.read(); h = {k.lower(): v for k, v in resp.getheaders()}; c.close()
        return resp.status, h, data
    results = []

    def check(name, ok, detail=''):
        results.append({'check': name, 'ok': bool(ok), 'detail': detail}); print(('PASS ' if ok else 'FAIL ') + name + (f' ({detail})' if detail else ''))
    status, h, body = call('GET', '')
    policy = h.get('referrer-policy', '')
    check('root page 200 styled form', status == 200 and b'<form method=post' in body and b'style.css' in body, f'policy {policy}')
    status, _, css = call('GET', '/style.css'); check('stylesheet 200 text/css', status == 200 and b'prefers-color-scheme' in css)
    browser_origin = 'null' if policy == 'no-referrer' else origin     # Fetch: append a request Origin header
    form = lambda value: urllib.parse.urlencode({'code': value}).encode()
    post = lambda value, o=browser_origin: call('POST', '/unlock', {'Origin': o, 'Sec-Fetch-Site': 'same-origin', 'Sec-Fetch-Mode': 'navigate', 'Content-Type': 'application/x-www-form-urlencoded'}, form(value))
    if policy == 'no-referrer':
        status, h, body = post(f'{CODE[:4]}-{CODE[4:]}')
        check('preview.10 behaviour: the form POST carries Origin: null and is refused before the code is read', status == 403 and body == b'', f'status {status}, body {len(body)} bytes')
        print(json.dumps({'selfcheck': results}, indent=1)); return
    status, h, body = post('AAAA-AAAA'); check('wrong code: 403 with the form again and a message', status == 403 and b'role=alert' in body and b'name=code' in body)
    status, h, body = post(f' {CODE[:4]}‑{CODE[4:].lower()}\n')
    cookie = h.get('set-cookie', '')
    check('formatted code (no-break space, non-breaking hyphen, lower case, newline): 303 + cookie + Location', status == 303 and h.get('location') == path + '/player' and cookie.startswith('__Secure-share=') and all(a in cookie for a in ('HttpOnly', 'Secure', 'SameSite=Strict', f'Path={path}/')), f'status {status}')
    jar = cookie.split(';')[0]
    nav = {'Cookie': jar, 'Referer': origin + path, 'Sec-Fetch-Site': 'same-origin'}
    status, _, body = call('GET', '/player', nav); check('player page 200 with a video element', status == 200 and b'<video controls' in body)
    status, h, body = call('GET', '/video', dict(nav, Range='bytes=0-99', **{'Sec-Fetch-Dest': 'video'}))
    check('Range playback 206 with Content-Range', status == 206 and h.get('content-range', '').startswith('bytes 0-99/') and len(body) == 100, h.get('content-range', ''))
    status, h, _ = call('GET', '/video', dict(nav, Range='bytes=99999999-')); check('Range past the end 416', status == 416)
    status, _, _ = call('GET', '/video', {}); check('media without a session 403', status == 403)
    print(json.dumps({'selfcheck': results}, indent=1))
    if not all(x['ok'] for x in results):
        sys.exit(1)


def stop(args):
    f = files(pathlib.Path(args.dir))
    try:
        pid = int(f['fixture.pid'].read_text())
    except (OSError, ValueError):
        sys.exit('no fixture pid recorded; nothing to stop')
    os.kill(pid, signal.SIGTERM)
    for _ in range(60):
        try:
            os.kill(pid, 0)
        except OSError:
            print('stopped'); return
        time.sleep(0.25)
    sys.exit('the fixture did not stop in 15 s')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest='cmd', required=True)
    for name in ('start', 'status', 'control', 'selfcheck', 'stop'):
        s = sub.add_parser(name); s.add_argument('--dir', default=str(DEFAULT_DIR))
        if name in ('start', 'control'):
            s.add_argument('--policy', choices=['no-referrer', 'same-origin'], default='same-origin' if name == 'start' else None)
        if name == 'start':
            s.add_argument('--expiry', default='finite:3600')
        if name == 'control':
            s.add_argument('--revoke', action='store_true')
    a = p.parse_args()
    if a.cmd == 'status':
        print(json.dumps(read_ready(a.dir), indent=1))
    else:
        {'start': start, 'control': control, 'selfcheck': selfcheck, 'stop': stop}[a.cmd](a)
