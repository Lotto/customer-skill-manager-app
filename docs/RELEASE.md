# Release runbook

Releases are built and published by GitHub Actions. You only tag; CI does the
rest (build → sign → publish for Windows/macOS/Linux, plus the updater
manifest).

## One-time setup: updater signing secrets

The auto-updater refuses any package not signed with the private key that
matches the public key embedded in `src-tauri/tauri.conf.json`
(`plugins.updater.pubkey`). Add the private key as repository secrets:

1. In the repo: **Settings → Secrets and variables → Actions → New repository secret**.
2. Add:
   - `TAURI_SIGNING_PRIVATE_KEY` — the full contents of the `.key` private key file.
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the key password (empty string if the key has none).

The keypair was generated with:

```bash
npx tauri signer generate -w csm-updater.key
```

> Keep the private key out of the repo. If it is lost, existing installs can no
> longer be updated (you'd have to ship a new public key and re-install).

### Optional: code-signing certificates

Without OS code-signing, Windows SmartScreen warns on first download and macOS
Gatekeeper blocks the app until the user allows it. To remove those:

- **Windows**: an Authenticode certificate (`WINDOWS_CERTIFICATE` /
  `WINDOWS_CERTIFICATE_PASSWORD`) wired into the Windows job.
- **macOS**: an Apple Developer ID certificate + notarization
  (`APPLE_CERTIFICATE`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`).

These are not required for the updater to work — only for a warning-free install.

## Cutting a release

1. Bump the version in **both** `src-tauri/tauri.conf.json` and
   `src-tauri/Cargo.toml` (keep them equal), commit.
2. Tag and push:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. The **Release** workflow builds every platform and creates a **draft**
   GitHub release with the installers + `latest.json` attached.
4. Review the draft, then **publish** it. Publishing marks it the *latest*
   release, which is what the updater endpoint
   (`releases/latest/download/latest.json`) resolves to.

## How updates reach clients

Installed apps check `latest.json` on their schedule. If it advertises a newer,
correctly-signed version, the app downloads it in the background and offers a
tray entry **"Restart to install …"** — it never restarts on its own (a restart
mid-sync could interrupt a skill write).
