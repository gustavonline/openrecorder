"""Collector-validity contract for scripts/measure-native.py. Pure functions plus trivial local child processes.

Usage: python3 tests/measure-native-contract.py
Does not measure the application and creates no profile data: the children are `python3 -c ...` and `sleep`.
"""
import contextlib, importlib.util, io, json, os, pathlib, sys, tempfile, textwrap

root = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('measure_native', root / 'scripts/measure-native.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
checks = []
W = m.WARMUP_SECONDS


def sample(t, phase='run'):
    return {'phase': phase, 'elapsedMonotonicSeconds': t}


def report(samples, run, errors=0, **extra):
    collection = m.assess_collection(samples, run, errors)
    steady = [s for s in samples if s['elapsedMonotonicSeconds'] >= W and s['phase'] == 'run']
    return {'sampleCount': len(samples), 'steadySampleCount': len(steady), 'lastSampleSeconds': samples[-1]['elapsedMonotonicSeconds'] if samples else None,
            'collection': collection, 'exit': 0, 'postExitSurvivors': [], 'survivorAccounting': 'complete', 'expectedOutputs': [], **extra}


# Review finding 5: one steady sample, then nothing for the rest of the run, must not be able to PASS.
lost = report([sample(1), sample(5.3)], 30, errors=95)
reasons = m.failure_reasons(lost)
assert any('incomplete collection' in r for r in reasons), reasons
assert not lost['collection']['valid'] and lost['collection']['tailGapSeconds'] > 24
dense = [sample(round(0.4 * i, 2)) for i in range(1, 76)]
good = report(dense, 30.1)
assert good['collection']['valid'] and m.failure_reasons(good) == [], (good['collection'], m.failure_reasons(good))
gap = [s for s in dense if not 12 < s['elapsedMonotonicSeconds'] < 15.5]
assert not report(gap, 30.1)['collection']['valid'], 'a 3.5 s hole in the steady window is incomplete'
assert not report([s for s in dense if s['elapsedMonotonicSeconds'] > 8], 30.1)['collection']['valid'], 'late start of steady sampling'
assert not report([s for s in dense if s['elapsedMonotonicSeconds'] < 24], 30.1)['collection']['valid'], 'ended early'
short = report([sample(0.4 * i) for i in range(1, 22)], 8.4)
assert any('shorter than the required' in r for r in short['collection']['problems']), short['collection']
assert m.failure_reasons(short), 'a short run cannot PASS'
# Post-exit samples never count toward the steady window.
assert not report([sample(1)] + [sample(t, 'post-exit') for t in (6, 7, 8, 9)], 30)['collection']['valid']
checks.append('inadequate coverage, gaps, early end, short windows and lost sampling cannot yield PASS')

# Survivors and outputs.
service = {'pid': 9, 'name': 'OpenRecorder', 'expectedService': True}
encoder = {'pid': 10, 'name': 'ffmpeg', 'expectedService': False}
assert m.failure_reasons(report(dense, 30.1, postExitSurvivors=[service])) == []
undeclared = m.failure_reasons(report(dense, 30.1, postExitSurvivors=[service, encoder], postExitGraceSeconds=15))
assert len(undeclared) == 1 and 'ffmpeg[10]' in undeclared[0] and 'OpenRecorder' not in undeclared[0]
for output, text in (({'path': 'a', 'exists': False}, 'missing'), ({'path': 'a', 'exists': True, 'bytes': 0, 'stable': True}, 'empty'),
                     ({'path': 'a', 'exists': True, 'bytes': 5, 'stable': False}, 'growing')):
    assert any(text in r for r in m.failure_reasons(report(dense, 30.1, expectedOutputs=[output]))), text
assert m.failure_reasons(report(dense, 30.1, expectedOutputs=[{'path': 'a', 'exists': True, 'bytes': 5, 'stable': True}])) == []
real_args_for = m.args_for
m.args_for = lambda pids: {10: '/x/OpenRecorder --library-host', 11: '/x/ffmpeg -i in out.mp4'}
rows = {10: {'ppid': 1, 'comm': '/x/OpenRecorder'}, 11: {'ppid': 10, 'comm': '/x/ffmpeg'}, 5: {'ppid': 1, 'comm': '/x/root'}}
found = m.classify_survivors(rows, 5, ['--library-host'])
m.args_for = real_args_for
assert [(x['pid'], x['expectedService']) for x in found] == [(10, True), (11, False)], found
# Review finding 4: valid 30 s coverage with *no* successful post-exit accounting must not PASS.
for accounting in ('unavailable', 'not-attempted', None):
    blind = report(dense, 30.1, survivorAccounting=accounting) if accounting else {k: v for k, v in report(dense, 30.1).items() if k != 'survivorAccounting'}
    assert blind['collection']['valid'] and blind['postExitSurvivors'] == [], 'the steady window itself is fine'
    reasons = m.failure_reasons(blind)
    assert len(reasons) == 1 and 'post-exit process accounting' in reasons[0] and 'unknown whether an encoder had finished' in reasons[0], (accounting, reasons)
assert m.failure_reasons(report(dense, 30.1, survivorAccounting='complete')) == []
# An interrupted measurement has no exit code and is reported as interrupted, not as missing accounting.
interrupted = report(dense, 30.1, survivorAccounting='not-attempted', interrupted=True, exit=None)
assert m.failure_reasons(interrupted) == ['measurement interrupted before child exit; child was not killed']
checks.append('missing post-exit accounting cannot yield PASS (unavailable, not attempted, absent); complete accounting still passes')
checks.append('undeclared survivors (encoder) fail; declared services and the root are handled; outputs must be final')


def run(*argv):
    out = pathlib.Path(tempfile.mkdtemp(prefix='measure-contract-')) / 'report.json'
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        code = m.main([*argv[0], str(out), *argv[1]])
    return code, json.loads(out.read_text())


# Review finding 7: a missing executable returns 127 with a uniform report and no traceback.
code, r = run([], ['/nonexistent/openrecorder-core', '--desktop'])
assert code == 127 and r['status'] == 'FAIL' and r['launchError'].startswith('FileNotFoundError'), (code, r.get('launchError'))
for key in ('stderr', 'result', 'samples', 'binaries', 'postExitSurvivors', 'endCondition'):
    assert key in r, key
assert r['endCondition'] == 'launch-failed' and any('launch failed' in x for x in r['failureReasons'])
plain = pathlib.Path(tempfile.mkdtemp()) / 'not-executable'
plain.write_text('x')
code, r = run([], [str(plain)])
assert code == 126 and r['launchError'].startswith('PermissionError'), (code, r.get('launchError'))
checks.append('missing executable -> 127 (not executable -> 126), complete FAIL report, no KeyError')

# Review finding 6: root exit with a live descendant is observed for a bounded window and reported honestly.
spawner = textwrap.dedent('''
    import subprocess, time
    subprocess.Popen(["sleep", "4"], start_new_session=True)
    time.sleep(1.0)
''')
code, r = run(['--post-exit-grace', '8'], [sys.executable, '-c', spawner])
assert r['rootExitedWithLiveDescendants'] is True and r['endCondition'] == 'root-exit+drained', (r['endCondition'], r['postExitSurvivors'])
assert r['postExitSurvivors'] == [] and r['postExitSampleCount'] >= 1 and r['survivorAccounting'] == 'complete'
assert r['status'] == 'FAIL' and any('no steady samples' in x for x in r['failureReasons']), 'a 1 s run must not pass'
assert 'not an exhaustive process lineage' in r['lineage']
code, r = run(['--post-exit-grace', '1'], [sys.executable, '-c', spawner])
assert r['endCondition'] == 'root-exit+grace-expired', r['endCondition']
assert [x['name'] for x in r['postExitSurvivors']] == ['sleep'] and not r['postExitSurvivors'][0]['expectedService']
assert any('still running after' in x for x in r['failureReasons']) and r['runSeconds'] < 4
code, r = run(['--post-exit-grace', '8', '--expect-survivor', 'sleep'], [sys.executable, '-c', spawner])
assert r['endCondition'] == 'root-exit+expected-services-remain' and r['postExitSurvivors'][0]['expectedService'] is True, r['endCondition']
assert not any('still running after' in x for x in r['failureReasons'])
checks.append('root exit + descendant: bounded post-exit window, drained/expired/declared-service end conditions, no endless wait')

# Fault injection, real run: every process snapshot after the root has exited fails.
marker = pathlib.Path(tempfile.mkdtemp(prefix='measure-fault-')) / 'root-exiting'
child = f'import pathlib, time\npathlib.Path({str(marker)!r}).write_text("x")\ntime.sleep(0.3)\n'
real_snapshot = m.snapshot
def blind_after_exit():
    if marker.exists():
        raise OSError('ps unavailable (injected)')
    return real_snapshot()
m.snapshot = blind_after_exit
try:
    code, r = run(['--post-exit-grace', '1'], [sys.executable, '-c', child])
finally:
    m.snapshot = real_snapshot
assert r['survivorAccounting'] == 'unavailable' and r['endCondition'] == 'root-exit+accounting-unavailable', (r['survivorAccounting'], r['endCondition'])
assert r['postExitSurvivors'] == [] and r['status'] == 'FAIL' and code == 3, (r['status'], code)
assert any('post-exit process accounting was unavailable' in x for x in r['failureReasons']), r['failureReasons']
assert r['samplingErrorCount'] >= 1 and any(e.get('phase') == 'post-exit' for e in r['samplingErrors'])
checks.append('real run with every post-exit ps failing: accounting unavailable, FAIL with a specific reason, exit 3')
print(json.dumps({'passed': True, 'checks': checks}, indent=2))
