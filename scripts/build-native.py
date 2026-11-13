#!/usr/bin/env python3
"""Build and package a platform runtime with an explicitly supplied media helper.
Usage: python3 scripts/build-native.py /absolute/path/to/ffmpeg
Requires Rust and platform development libraries on the build machine only.
"""
import hashlib,json,os,pathlib,platform,plistlib,re,shutil,subprocess,sys,tarfile,tempfile
root=pathlib.Path(__file__).resolve().parents[1]
helper=pathlib.Path(sys.argv[1]).resolve();assert helper.is_file(),"Supply a compiled FFmpeg helper"
def minos(path):
 # The deployment target a Mach-O file really carries (LC_BUILD_VERSION minos), read with otool; never typed by hand.
 out=subprocess.run(["/usr/bin/otool","-l",str(path)],capture_output=True,text=True,check=True).stdout;return re.search(r"minos (\S+)",out.split("LC_BUILD_VERSION",1)[1]).group(1)
subprocess.run(["cargo","build","--release","--locked","--features","desktop-ui","--manifest-path",str(root/"engine/Cargo.toml")],check=True)
name={"Darwin":"darwin","Linux":"linux","Windows":"win32"}[platform.system()]
arch={"arm64":"arm64","aarch64":"arm64","x86_64":"x64","AMD64":"x64"}[platform.machine()]
with tempfile.TemporaryDirectory(prefix="openrecorder-package-")as tmp:
 stage=pathlib.Path(tmp);exe=".exe"if name=="win32"else"";binary=pathlib.Path(os.environ.get("CARGO_TARGET_DIR",str(root/"engine/target")))/("release/openrecorder-core"+exe)
 if name=="darwin":
  app=stage/"OpenRecorder.app";contents=app/"Contents";mac=contents/"MacOS";helpers=contents/"Helpers";resources=contents/"Resources";mac.mkdir(parents=True);helpers.mkdir();resources.mkdir()
  shutil.copyfile(binary,mac/"OpenRecorder");shutil.copyfile(helper,helpers/"ffmpeg");os.chmod(mac/"OpenRecorder",0o755);os.chmod(helpers/"ffmpeg",0o755)
  # The app icon is the project's own black disc, drawn here from its geometry (scripts/render-app-icon.py), not third-party art.
  subprocess.run([sys.executable,str(root/"scripts/render-app-icon.py"),str(resources/"OpenRecorder.icns")],check=True)
  # The system the app needs is the highest of what it ships (the helper is built for a newer macOS than the core), read from the files.
  needed=max(minos(mac/"OpenRecorder"),minos(helpers/"ffmpeg"),key=lambda v:tuple(int(n)for n in v.split(".")))
  identity=os.environ.get("OPENRECORDER_SIGN_IDENTITY","-");keychain=os.environ.get("OPENRECORDER_SIGN_KEYCHAIN");sign_args=["--sign",identity]+(["--keychain",keychain]if keychain else[])
  info={"CFBundleIdentifier":"online.openrecorder.desktop","CFBundleName":"OpenRecorder","CFBundleDisplayName":"OpenRecorder","CFBundleExecutable":"OpenRecorder","CFBundleIconFile":"OpenRecorder","CFBundlePackageType":"APPL","CFBundleVersion":"17","CFBundleShortVersionString":"0.6.0","OpenRecorderBuild":hashlib.sha256(binary.read_bytes()+helper.read_bytes()).hexdigest(),"LSUIElement":True,"LSMinimumSystemVersion":needed,"NSScreenCaptureUsageDescription":"Record the screen or window you choose and save the video locally on your computer.","NSHighResolutionCapable":True,"NSCameraUsageDescription":"Show your camera bubble and save its separate local recording when you enable Camera.","NSMicrophoneUsageDescription":"Record your microphone locally when you enable Mic."}
  (contents/"Info.plist").write_bytes(plistlib.dumps(info));subprocess.run(["codesign","--force",*sign_args,str(helpers/"ffmpeg")],check=True);subprocess.run(["codesign","--force","--deep",*sign_args,"--identifier","online.openrecorder.desktop",str(app)],check=True);subprocess.run(["codesign","--verify","--deep","--strict",str(app)],check=True)
 else:
  for src,dst in [(binary,stage/("openrecorder-core"+exe)),(helper,stage/("ffmpeg"+exe))]:shutil.copyfile(src,dst);os.chmod(dst,0o755)
 output=root/"engine/runtime";output.mkdir(exist_ok=True);archive=output/(name+"-"+arch+".tar.gz")
 with tarfile.open(archive,"w:gz")as tar:
  for item in stage.iterdir():tar.add(item,arcname=item.name)
 receipt={"platform":name,"architecture":arch,"archiveBytes":archive.stat().st_size,"sha256":hashlib.sha256(archive.read_bytes()).hexdigest(),"coreBytes":binary.stat().st_size,"mediaHelperBytes":helper.stat().st_size}
 (output/(name+"-"+arch+".json")).write_text(json.dumps(receipt,indent=2));print(json.dumps(receipt))
