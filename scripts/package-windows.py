#!/usr/bin/env python3
"""Package a previously cross-built Windows core and its matching media helper.
Usage: package-windows.py CORE.exe FFMPEG.exe LIBWINPTHREAD.dll
"""
import pathlib,sys,tempfile,shutil,tarfile,json,hashlib
root=pathlib.Path(__file__).resolve().parents[1];output=root/'engine/runtime';output.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='openrecorder-windows-')as tmp:
 stage=pathlib.Path(tmp)
 for source,name in zip(sys.argv[1:],['openrecorder-core.exe','ffmpeg.exe','libwinpthread-1.dll']):shutil.copyfile(source,stage/name)
 assert (stage/'openrecorder-core.exe').is_file()and(stage/'ffmpeg.exe').is_file()and(stage/'libwinpthread-1.dll').is_file()
 archive=output/'win32-x64.tar.gz'
 with tarfile.open(archive,'w:gz')as tar:
  for item in stage.iterdir():tar.add(item,arcname=item.name)
 receipt={'platform':'win32','architecture':'x64','archiveBytes':archive.stat().st_size,'sha256':hashlib.sha256(archive.read_bytes()).hexdigest(),'coreBytes':(stage/'openrecorder-core.exe').stat().st_size,'mediaHelperBytes':(stage/'ffmpeg.exe').stat().st_size,'hardwareTested':False}
 (output/'win32-x64.json').write_text(json.dumps(receipt,indent=2));print(json.dumps(receipt))
