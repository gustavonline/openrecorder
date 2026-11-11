"""Generated plugin manifests satisfy the actual loader rules found in the host diagnostics. Pure file checks.

Usage: python3 tests/package-manifests.py
Builds each platform ZIP into a temporary directory from the repository's runtime archives and inspects the two MCP
manifests. Nothing is installed or launched. Actual Codex/Claude loading still needs a separate host run.
"""
import json, pathlib, subprocess, sys, tempfile, zipfile

root = pathlib.Path(__file__).resolve().parents[1]
checks = []
with tempfile.TemporaryDirectory(prefix='recorder-package-check-') as t:
    for platform in ('darwin-arm64', 'linux-x64', 'win32-x64'):
        out = pathlib.Path(t) / f'{platform}.zip'
        subprocess.run([sys.executable, str(root / 'scripts/package-plugin.py'), platform, str(out)], check=True, capture_output=True)
        with zipfile.ZipFile(out) as z:
            read = lambda n: json.loads(z.read('openrecorder/' + n))
            names = z.namelist()
            portable, claude = read('mcp.json')['mcpServers']['openrecorder'], read('.mcp.json')['mcpServers']['openrecorder']
            exe = 'OpenRecorder.app/Contents/MacOS/OpenRecorder' if platform.startswith('darwin') else 'openrecorder-core' + ('.exe' if platform.startswith('win32') else '')
            # Codex 0.167.0 (typed Agent Plugins): bare executable name or a contained ./ path; no placeholders.
            cmd = portable['command']
            assert cmd == './.runtime/' + exe and '${' not in cmd and '..' not in cmd.split('/'), cmd
            assert portable['args'] == ['--stdio'] and 'cwd' not in portable and portable['type'] == 'stdio'
            assert 'openrecorder/.runtime/' + exe in names, 'command target missing from the package'
            # Claude Code keeps its own placeholder, in its own file.
            assert claude['command'] == '${CLAUDE_PLUGIN_ROOT}/.runtime/' + exe and claude['args'] == ['--stdio'] and 'cwd' not in claude
            assert 'openrecorder/.claude-plugin/plugin.json' in names and 'openrecorder/plugin.json' in names
            # The ZIP is also a local marketplace for both tools.
            assert 'openrecorder/.agents/plugins/marketplace.json' in names and 'openrecorder/.claude-plugin/marketplace.json' in names
            assert json.loads(z.read('openrecorder/.agents/plugins/marketplace.json'))['plugins'][0]['source'] == {'source': 'local', 'path': './'}
            blob = b''.join(z.read(n) for n in names if n.endswith(('.json', '.md')) and '.runtime' not in n)
            assert b'/Users/' not in blob and b'/private/tmp' not in blob, 'a private path in the package'
            assert b'node ' not in json.dumps([portable, claude]).encode()
            receipt = json.loads(out.with_suffix('.json').read_text())
            assert receipt['nodeRequired'] is False and receipt['browserEngineBundled'] is False
        checks.append(f'{platform}: contained ./ command, Claude placeholder separate, target present, no Node')
root_manifest = json.loads((root / 'mcp.json').read_text())['mcpServers']['openrecorder']
assert root_manifest['command'].startswith('./.runtime/') and 'cwd' not in root_manifest
checks.append('checkout mcp.json uses the same contained path')
print(json.dumps({'passed': True, 'checks': checks}, indent=2))
