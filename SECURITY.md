# Security

OpenRecorder records your screen, camera and microphone and can expose one recording through a temporary link, so a boundary bug matters.

## Scope

In scope: anything that lets an agent or another local/remote party start a recording or open a device without the person's press; reading or exposing the owner key or the app-only session key; reaching the library, Settings, other recordings or original files through a shared link; bypassing the access code or its rate limit; a media or page route that leaks files outside a take's folder.
Out of scope: an attacker who already runs code as you with shell or computer-use access (the guards are for tool calls, not a sandbox, and the README says so); the macOS Gatekeeper warning for a non-notarized app (documented); findings that need a modified build.

## Reporting

Please use GitHub's **private vulnerability reporting** on this repository (Security tab → Report a vulnerability) so details stay private until a fix exists. If it is not offered, open an issue that says only that you have a security report and ask for a private channel; do not put details in the issue. This is a one-person project: expect an acknowledgement, not a service level.

## Supported versions

Only the latest preview release is supported.
