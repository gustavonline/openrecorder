# Update, uninstall and recovery

## Where things are

| What | Where (default) |
|---|---|
| Library index, settings, journal | macOS `~/Library/Application Support/OpenRecorder/Native` · Windows `%LOCALAPPDATA%\OpenRecorder\Native` · Linux `$XDG_DATA_HOME/openrecorder/native` (or `~/.local/share/openrecorder/native`) |
| Recordings | `~/Movies/OpenRecorder` (macOS) or `~/Videos/OpenRecorder`, or the folder you chose in Settings; one folder per take |
| The app (macOS) | `~/Applications/OpenRecorder.app` |
| The plugin | the unzipped plugin folder you added to Codex or Claude Code |

None of these is inside a plugin cache, so replacing or removing a plugin never touches your data. `OPENRECORDER_HOME` can point the library elsewhere.

## Update

Download the new release, verify its checksum, quit OpenRecorder, and replace the app (and the plugin folder, if you use it) with the new one.

- Recordings, folders, favorites, archive state and settings are kept; a setting an older build stored (for example 24 fps or a stored camera shape) is kept exactly as you chose it.
- OpenRecorder never resets permissions. Whether macOS keeps a grant across an update depends on the app's signing identity staying the same; a build signed with a different identity can be treated as a different app and asked about afresh.
- The plugin never replaces an installed app while any OpenRecorder process may be recording; it tells you to stop and quit first.
- Active shares end when OpenRecorder quits or updates. Links and codes are never written to disk.

## Uninstall

1. Stop any recording and choose **Quit OpenRecorder** from the menu-bar icon (it is refused while a take is active).
2. Remove the plugin in Codex / Claude Code, then delete its folder.
3. Delete `OpenRecorder.app`.
4. Optional: delete the library folder above. **Your recordings are separate files; delete them yourself, deliberately, from the recordings folder.** The app never deletes a recording on its own.
5. To revoke access, use System Settings → Privacy & Security (Screen & System Audio Recording, Camera, Microphone). OpenRecorder never resets or removes it.

## Recovery

- **A take stopped early** (the app quit, the machine slept, the encoder could not keep up): the files already written are kept and Home says "Recording stopped early. Available files were kept.". Open the recording; **Technical details** shows the reason. A take with no proof it was finished has no player and no duration, but its files are in the take's folder.
- **Home shows no recordings or is unreachable**: choose **Quit OpenRecorder**, then open it again. One library host owns the library at a time, enforced by an operating-system lock, so a second one never starts over it, and a crashed host releases the lock by itself.
- **No tray (Linux)**: OpenRecorder opens Home once and Settings → About says "Controls: Home".
- **Check the install**: `OpenRecorder --doctor` (macOS: `OpenRecorder.app/Contents/MacOS/OpenRecorder --doctor`) prints one JSON object about the bundled media helper without touching any device.
- **Journal**: `native-events.jsonl` in the library folder records app events; read it before attaching any of it to a report, and remove anything you do not want to share.
- **Never** keep the only copy of something important in an unreviewed preview build. Copy the recording folder to a backup you trust.
