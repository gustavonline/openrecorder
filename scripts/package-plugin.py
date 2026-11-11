#!/usr/bin/env python3
"""Create a native-only, platform-specific local plugin ZIP from a runtime archive.

Usage: package-plugin.py PLATFORM OUTPUT.zip [RUNTIME.tar.gz]   (PLATFORM: darwin-arm64, linux-x64 or win32-x64)
The runtime archive (default engine/runtime/PLATFORM.tar.gz) comes from scripts/build-native.py (or scripts/package-windows.py). The ZIP is a
complete local plugin and, because it carries the Codex and Claude Code marketplace files, also a local marketplace.
"""
import pathlib,sys,tempfile,tarfile,zipfile,shutil,json,hashlib,stat
root=pathlib.Path(__file__).resolve().parents[1];platform=sys.argv[1]
assert platform in ['darwin-arm64','linux-x64','win32-x64']
runtime=pathlib.Path(sys.argv[3]).resolve() if len(sys.argv)>3 else root/'engine/runtime'/(platform+'.tar.gz');assert runtime.is_file(),'runtime archive not found: '+str(runtime)
out=pathlib.Path(sys.argv[2]).resolve();assert not out.exists(), 'Choose a new output file'
with tempfile.TemporaryDirectory(prefix='recorder-plugin-')as t:
 stage=pathlib.Path(t)/'openrecorder';stage.mkdir()
 for name in ['plugin.json','README.md','LICENSE','THIRD_PARTY.md','CHANGELOG.md']:
  shutil.copyfile(root/name,stage/name)
 for name in ['skills','licenses','docs','.claude-plugin','.agents']:
  shutil.copytree(root/name,stage/name)
 (stage/'assets').mkdir()
 for f in (root/'assets').glob('icon*.svg'):shutil.copyfile(f,stage/'assets'/f.name)
 target=stage/'.runtime';target.mkdir()
 with tarfile.open(runtime)as tar:tar.extractall(target,filter='data')
 executable='OpenRecorder.app/Contents/MacOS/OpenRecorder'if platform.startswith('darwin')else'openrecorder-core'+('.exe'if platform.startswith('win32')else'')
 # Codex's typed Agent Plugins loader accepts only a bare executable name or a contained ./ path (a ${PLUGIN_ROOT}
 # command was rejected by Codex 0.167.0), so mcp.json is relative to the plugin root. Claude Code expands
 # ${CLAUDE_PLUGIN_ROOT} in command/args, so its separate .mcp.json keeps that. No cwd: nothing here needs one.
 portable={'$schema':'https://agent-plugins.org/schemas/1.0.0/mcp.schema.json','mcpServers':{'openrecorder':{'type':'stdio','command':'./.runtime/'+executable,'args':['--stdio']}}}
 claude={'mcpServers':{'openrecorder':{'type':'stdio','command':'${CLAUDE_PLUGIN_ROOT}/.runtime/'+executable,'args':['--stdio']}}}
 (stage/'mcp.json').write_text(json.dumps(portable,indent=2)+'\n');(stage/'.mcp.json').write_text(json.dumps(claude,indent=2)+'\n')
 # One fixed timestamp for every member (SOURCE_DATE_EPOCH if set): a reproducible ZIP, and no pre-1980 dates from an archive.
 import os
 stamp=int(os.environ.get('SOURCE_DATE_EPOCH','1767225600'))
 for item in sorted(stage.rglob('*')):os.utime(item,(stamp,stamp))
 with zipfile.ZipFile(out,'w',zipfile.ZIP_DEFLATED,compresslevel=9)as z:
  for p in sorted(stage.rglob('*')):
   if p.is_file():z.write(p,p.relative_to(stage.parent))
 receipt={'platform':platform,'version':json.loads((root/'plugin.json').read_text())['version'],'bytes':out.stat().st_size,'sha256':hashlib.sha256(out.read_bytes()).hexdigest(),'runtimeArchiveSha256':hashlib.sha256(runtime.read_bytes()).hexdigest(),'nodeRequired':False,'browserEngineBundled':False}
 out.with_suffix('.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt))
