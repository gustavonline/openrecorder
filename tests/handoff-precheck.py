"""The operator's idle precheck judges strictly: anything missing, malformed or not clearly idle means "not idle".

Usage: python3 tests/handoff-precheck.py      (pure: synthetic states, no process is started, nothing is contacted)
The real accepted-11 library sequence (default defer, malformed answers, the controlled quiet-window Quit) is tests/version-handoff.py.
"""
import importlib.util, pathlib

spec = importlib.util.spec_from_file_location('handoff_precheck', pathlib.Path(__file__).resolve().parents[1] / 'scripts/handoff-precheck.py')
pc = importlib.util.module_from_spec(spec); spec.loader.exec_module(pc)
TARGET = '0.6.0-preview.12'
idle = {'version': '0.6.0-preview.11', 'active': None, 'shares': [], 'capture': {'phase': 'unavailable', 'generation': 0, 'known': False}}
app = '/Applications/OpenRecorder.app/Contents/MacOS/OpenRecorder'
quiet = [(1, app, 'OpenRecorder --stdio'), (2, app, 'OpenRecorder --library-host'), (3, app, 'OpenRecorder --doctor'), (4, '/bin/zsh', 'zsh -c /x/OpenRecorder --desktop'),
         (5, '/Applications/ChatGPT.app/Helpers/handler', '--database=/Users/u/Library/ArcitAI OpenRecorder Test/Crashpad')]


def blocked(state, rows=quiet):
    return pc.blockers(state, rows, TARGET, ())


assert blocked(idle) == [], blocked(idle)
assert blocked(dict(idle, shares=[{'status': 'failed'}])) == [], 'a failed share holds no tunnel'
without = lambda key: {k: v for k, v in idle.items() if k != key}
for name, state in {
    'missing version': without('version'), 'garbage1': dict(idle, version='garbage1'), 'numeric': dict(idle, version=11), 'equal': dict(idle, version=TARGET), 'newer': dict(idle, version='0.7.0'),
    'missing active (not the same as null)': without('active'), 'take': dict(idle, active={'status': 'recording'}), 'empty take object': dict(idle, active={}),
    'missing shares': without('shares'), 'shares null': dict(idle, shares=None), 'shares object': dict(idle, shares={}), 'share ready': dict(idle, shares=[{'status': 'ready'}]),
    'share starting': dict(idle, shares=[{'status': 'starting'}]), 'share without status': dict(idle, shares=[{}]), 'one failed one ready': dict(idle, shares=[{'status': 'failed'}, {'status': 'ready'}]),
    'missing capture': without('capture'), 'capture known': dict(idle, capture={'phase': 'idle', 'known': True}), 'capture without known': dict(idle, capture={'phase': 'unavailable'}),
    'not an object': ['idle'], 'null': None,
}.items():
    assert blocked(state), f'{name} must not be idle'
for odd in ('x0.6.0-preview.9y', '0.6.0-preview.9-extra', ' 0.6.0-preview.9', '0.6.0-preview.9\n', 'v0.6.0-preview.9', '0.6.0-preview.', '0.6-preview.9', '0.6.0-preview.9.1', ''):
    assert blocked(dict(idle, version=odd)), f'a malformed version must not count as older: {odd!r}'
assert pc.version_key('0.6.0-preview.9') < pc.version_key('0.6.0-preview.12') < pc.version_key('0.6.0') and pc.version_key('garbage1') is None
for name, rows in {'unreadable process list': None, 'the app': [(9, app, 'OpenRecorder')], 'a toolbar': [(9, app, 'OpenRecorder --toolbar --config /x/rust-connection.json')], 'the media helper': [(9, '/Applications/OpenRecorder.app/Contents/Helpers/ffmpeg', 'ffmpeg -i x')],
                   'a capture mode': [(9, '/x/openrecorder-core', 'openrecorder-core --synthetic /o 5')], 'a row that does not parse': [('x', 'y')]}.items():
    assert blocked(idle, rows), f'{name} means not idle'
print('handoff precheck: ok')
