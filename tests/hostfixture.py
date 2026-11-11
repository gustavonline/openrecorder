"""Shared fixture for the isolated host/bridge protocol tests.

Everything runs against a throw-away HOME and OPENRECORDER_HOME with a disabled tunnel executable. Nothing here
launches the native toolbar, a browser, capture hardware or a tunnel, and the access key is never printed. Cleanup
only touches processes this fixture started and could attribute: hosts published in its own descriptor, verified by
their command line, and its own bridge children.
"""
import json, os, pathlib, select, shutil, signal, subprocess, tempfile, time, urllib.error, urllib.request


def package_version():
    """The release this checkout builds, from the one place it is declared."""
    import re
    text = (pathlib.Path(__file__).resolve().parent.parent / 'engine' / 'Cargo.toml').read_text()
    return re.search(r'^version\s*=\s*"([^"]+)"', text, re.M).group(1)


HOME_URI = 'ui://openrecorder/home-v' + package_version()


def ps():
    out = subprocess.run(['/bin/ps', '-axww', '-o', 'pid=,ppid=,stat=,args='], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        parts = line.split(None, 3)
        if len(parts) == 4 and parts[0].isdigit() and parts[1].isdigit():
            rows.append((int(parts[0]), int(parts[1]), parts[2], parts[3]))
    return rows


def alive(pid):
    for p, _, stat, _ in ps():
        if p == pid:
            return not stat.startswith('Z')
    return False


class Bridge:
    def __init__(self, fx):
        self.fx = fx
        self.log = open(fx.root / f'bridge-{len(fx.bridges)}.stderr', 'w')
        self.p = subprocess.Popen([fx.binary, '--stdio'], env=fx.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=self.log, bufsize=0)
        self.buffer = b''
        self.next_id = 0
        self.raw = []
        fx.bridges.append(self)

    def send(self, obj):
        data = obj if isinstance(obj, bytes) else (json.dumps(obj) + '\n').encode()
        self.p.stdin.write(data)
        self.p.stdin.flush()

    def recv(self, timeout=20):
        end = time.monotonic() + timeout
        while b'\n' not in self.buffer:
            left = end - time.monotonic()
            if left <= 0:
                raise TimeoutError('no protocol message')
            ready, _, _ = select.select([self.p.stdout], [], [], left)
            if ready:
                chunk = os.read(self.p.stdout.fileno(), 65536)
                if not chunk:
                    raise EOFError('bridge closed stdout')
                self.buffer += chunk
        line, self.buffer = self.buffer.split(b'\n', 1)
        self.raw.append(line)
        message = json.loads(line)  # stdout carries protocol messages only: anything else fails here
        assert message.get('jsonrpc') == '2.0', 'stdout line is not a JSON-RPC message'
        return message

    def silent(self, wait=0.4):
        ready, _, _ = select.select([self.p.stdout], [], [], wait)
        return not ready and b'\n' not in self.buffer

    def rpc(self, method, params=None, timeout=20, **extra):
        self.next_id += 1
        message = {'jsonrpc': '2.0', 'id': self.next_id, 'method': method, **extra}
        if params is not None:
            message['params'] = params
        self.send(message)
        reply = self.recv(timeout)
        assert reply['id'] == self.next_id, reply
        return reply

    def initialize(self, version='2025-11-25', timeout=30):
        return self.rpc('initialize', {'protocolVersion': version, 'capabilities': {}, 'clientInfo': {'name': 'isolated-test', 'version': '1'}}, timeout)

    def tool(self, name, arguments=None, timeout=30, **params):
        reply = self.rpc('tools/call', {'name': name, 'arguments': arguments or {}, **params}, timeout)
        return reply.get('result', reply)

    def action(self, op, data=None, timeout=30):
        return self.tool('openrecorder_action', {'op': op, 'data': data or {}}, timeout)

    def ui(self, op, data=None, key=None, timeout=30):
        """The page-only tool, as the app window's own copy of the page calls it (key omitted = as a model would)."""
        args = {'op': op, 'data': data or {}}
        if key is not None:
            args['ui'] = key
        return self.tool('openrecorder_ui', args, timeout)

    def close(self, timeout=10):
        if self.p.poll() is None:
            try:
                self.p.stdin.close()
            except OSError:
                pass
            try:
                return self.p.wait(timeout)
            except subprocess.TimeoutExpired:
                self.p.terminate()
                return self.p.wait(5)
        return self.p.returncode


class Fixture:
    def __init__(self, binary, prefix='recorder-host-test-'):
        self.binary = str(pathlib.Path(binary).resolve())
        self.root = pathlib.Path(tempfile.mkdtemp(prefix=prefix))
        self.home = self.root / 'library'
        self.user = self.root / 'user'
        self.user.mkdir()
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith('TUNNEL_')}
        env.update(HOME=str(self.user), OPENRECORDER_HOME=str(self.home),
                   OPENRECORDER_CLOUDFLARED=str(self.root / 'tunnel-disabled'))
        env.pop('OPENRECORDER_FFMPEG', None)
        self.env = env
        self.bridges = []
        self.seen = {}  # pid -> descriptor origin, hosts this fixture observed publishing

    # ---- host descriptor and owner HTTP -------------------------------------------------------------------------
    def descriptor(self):
        try:
            c = json.loads((self.home / 'rust-runtime.json').read_text())
        except (OSError, ValueError):
            return None
        if isinstance(c, dict) and 'pid' in c:
            self.seen[c['pid']] = c.get('origin')
        return c

    def post(self, c, op, data=None, timeout=10, headers=None):
        request = urllib.request.Request(c['origin'] + '/api/action', data=json.dumps({'op': op, 'data': data or {}}).encode(),
                                         headers={'Authorization': 'Bearer ' + c['key'], **(headers or {})})
        return self.http(request, timeout)

    def http(self, request, timeout=10):
        try:
            with urllib.request.urlopen(request, timeout=timeout) as r:
                body = r.read()
                return r.status, body
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    def http_full(self, request, timeout=10):
        """(status, body, headers) with lower-cased header names."""
        try:
            with urllib.request.urlopen(request, timeout=timeout) as r:
                return r.status, r.read(), {k.lower(): v for k, v in r.headers.items()}
        except urllib.error.HTTPError as e:
            return e.code, e.read(), {k.lower(): v for k, v in e.headers.items()}

    def jpost(self, c, op, data=None, timeout=10):
        status, body = self.post(c, op, data, timeout)
        try:
            return status, json.loads(body)
        except ValueError:
            return status, {}

    # ---- processes ----------------------------------------------------------------------------------------------
    def hosts(self):
        """Live library-host processes of this binary that this fixture can attribute to itself."""
        mine = {b.p.pid for b in self.bridges}
        found = []
        for pid, ppid, stat, args in ps():
            if stat.startswith('Z') or not args.startswith(self.binary + ' --library-host'):
                continue
            if ppid in mine or pid in self.seen:
                found.append(pid)
        return found

    def children_of(self, pid):
        return [(p, a) for p, pp, st, a in ps() if pp == pid and not st.startswith('Z')]

    def is_own_host(self, pid):
        return any(p == pid and a.startswith(self.binary + ' --library-host') for p, _, _, a in ps())

    def wait_gone(self, pid, timeout=10):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if not alive(pid):
                return True
            time.sleep(0.1)
        return False

    # ---- cleanup ------------------------------------------------------------------------------------------------
    def stop_hosts(self):
        """Normal Quit first (only possible with no active take); signals only for verified own hosts."""
        for pid in list(self.seen):
            if self.is_own_host(pid):
                try:
                    os.kill(pid, signal.SIGCONT)
                except OSError:
                    pass
        c = self.descriptor()
        if c and c.get('pid') in self.seen and alive(c['pid']):
            try:
                self.post(c, 'quit', timeout=5)
            except OSError:
                pass
        for pid in list(self.seen):
            if alive(pid) and not self.wait_gone(pid, 8) and self.is_own_host(pid):
                os.kill(pid, signal.SIGTERM)
                if not self.wait_gone(pid, 5) and self.is_own_host(pid):
                    os.kill(pid, signal.SIGKILL)

    def cleanup(self):
        try:
            self.stop_hosts()
        finally:
            for b in self.bridges:
                try:
                    b.close(5)
                except Exception:
                    pass
                b.log.close()
            shutil.rmtree(self.root, ignore_errors=True)

    # ---- fixture data (metadata only; never real media) -----------------------------------------------------------
    def fake_take(self, c, title, finish=True):
        status, take = self.jpost(c, 'native-begin', {'title': title, 'root': str(self.root / 'media'), 'ownerPid': os.getpid(),
                                                      'preferences': {'camera': False, 'keepOriginals': False}})
        assert status == 200, take
        (pathlib.Path(take['directory']) / 'recording.mp4').write_bytes(b'not media: synthetic fixture bytes')
        if finish:
            status, done = self.jpost(c, 'native-finish', {'id': take['id'], 'duration': 1})
            assert status == 200 and done['status'] == 'ready', done
        return take
