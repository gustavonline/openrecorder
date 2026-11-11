"""Records the capture-report responses of the actual Rust host into tests/fixtures/actual-backend-contracts.json.

Usage: python3 tests/capture-fixtures.py /path/to/openrecorder-core
Runs against a throw-away HOME/OPENRECORDER_HOME with the tunnel disabled, like the other host tests: no GUI, no
device, no browser. The two recorded cases are what the real serializer answers to `state` before the native app has
reported anything, and after it has reported a failed take with devices, sources and a 60 fps capability. The seven
earlier cases in the file are left exactly as recorded; only these two are replaced.
"""
import hashlib, json, os, pathlib, sys
from hostfixture import Bridge, Fixture

binary = sys.argv[1]
fx = Fixture(binary, 'recorder-capture-fixture-')
target = pathlib.Path(__file__).resolve().parent / 'fixtures' / 'actual-backend-contracts.json'
try:
    b = Bridge(fx)
    assert 'result' in b.initialize()
    b.tool('openrecorder_home')
    c = fx.descriptor()
    status, before = fx.jpost(c, 'state')
    assert status == 200 and before['capture']['phase'] == 'unavailable'
    assert fx.jpost(c, 'native-window-ready', {'pid': os.getpid()})[0] == 200
    report = {
        'phase': 'idle', 'generation': 6, 'launch': 'fixture-launch-0001', 'fpsSet': 60, 'trayAvailable': True, 'trayNote': None, 'screenAllowed': True, 'screenState': 'allowed',
        'inputs': {'camera': {'mark': 'armed', 'name': 'FaceTime HD Camera'}, 'microphone': {'mark': 'off'}},
        'source': {'kind': 'display', 'name': 'Built-in Display'},
        'appearance': {'shape': 'rounded', 'size': 168, 'mirror': True, 'showCamera': True},
        'devices': {'camera': [{'uid': 'FIXTURE-CAM-A', 'name': 'FaceTime HD Camera', 'selected': True}, {'uid': 'FIXTURE-CAM-B', 'name': 'Studio Camera', 'selected': False}],
                    'microphone': [{'uid': 'MacBook Pro Microphone', 'name': 'MacBook Pro Microphone', 'selected': True}]},
        'sources': [{'id': 1, 'kind': 'display', 'name': 'Built-in Display', 'selected': True}, {'id': 2, 'kind': 'display', 'name': 'LG UltraFine', 'selected': False}],
        'notice': {'kind': 'failed', 'text': 'Couldn’t keep up at 60 fps, 2560 × 1440. Originals are kept.',
                   'details': 'Recording cannot keep up. Choose a lower resolution or frame rate.', 'generation': 6, 'kept': True},
        'capabilities': {'fps60': {'status': 'failed-last', 'basis': 'The last 60 fps recording of this setup could not keep up', 'measuredFps': None, 'encoder': {'status': 'qualified', 'measuredFps': 60.0}}},
    }
    report = {k: v for k, v in report.items() if v is not None}
    assert fx.jpost(c, 'native-progress', report)[0] == 200
    status, after = fx.jpost(c, 'state')
    assert status == 200 and after['capture']['phase'] == 'idle' and after['capture']['notice']['kind'] == 'failed'
    recording = {'phase': 'recording', 'generation': 7, 'launch': 'fixture-launch-0001', 'elapsedMs': 46000, 'fpsSet': 60, 'fpsDelivered': 59.8, 'sourceFps': 30.0,
                 'inputs': {'camera': {'mark': 'live', 'name': 'FaceTime HD Camera'}, 'microphone': {'mark': 'live'}}, 'source': {'kind': 'display', 'name': 'Built-in Display'},
                 'appearance': {'shape': 'wide', 'size': 224, 'mirror': False, 'showCamera': False}}
    assert fx.jpost(c, 'native-progress', recording)[0] == 200
    status, running = fx.jpost(c, 'state')
    assert running['capture']['phase'] == 'recording' and running['capture']['elapsedMs'] == 46000
    document = json.loads(target.read_text())
    cases = [k for k in document['cases'] if not k['name'].startswith('capture-')]
    cases += [{'name': 'capture-unreported', 'payload': before}, {'name': 'capture-failed-take-reported', 'payload': after}, {'name': 'capture-recording-reported', 'payload': running}]
    document['cases'] = cases
    document['captureCasesCoreSha256'] = hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest()
    target.write_text(json.dumps(document, indent=2) + '\n')
    print(json.dumps({'recorded': [k['name'] for k in cases if k['name'].startswith('capture-')]}))
finally:
    fx.cleanup() if hasattr(fx, 'cleanup') else None
