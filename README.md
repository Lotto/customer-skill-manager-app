# Customer Skill Manager

A silent, tray-resident desktop agent (Tauri 2 + Rust) that keeps a customer's
**licensed Claude Code skills** in sync. It polls the CSM backend on a schedule,
materializes each entitled skill into `~/.claude/skills/<slug>/SKILL.md`, and
updates itself automatically.

Philosophy: **silent + logged**. In normal operation there is no UI — just a
tray icon, a status line, and rotating log files. The only time the app shows
itself unprompted is first launch, to enter a license.

## Architecture

```
crates/csm-core   Pure, GUI-free logic (config, state, manifest parsing,
                  diff, sync engine, HTTP client). Fully unit-tested; runs
                  offline with `cargo test -p csm-core`.
src-tauri         The Tauri shell: tray, scheduler, updater, config UI,
                  OS integration (single-instance, autostart, logging).
ui/               Static configuration/activation page (no bundler).
```

Key invariants:

- **Safe removal** — a skill directory is deleted only if the app installed it
  (recorded in state *and* carrying a `.csm-managed` marker). Hand-made skills
  in `~/.claude/skills` are never touched.
- **Atomic install** — skills are written to a staging dir and swapped into
  place, so a half-written `SKILL.md` is never observed.
- **Version-driven** — the backend advertises versions, not hashes; a skill is
  re-fetched when its version changes.

## Backend contract

`GET <endpoint>?resource=__list` returns a markdown list of skills; `GET
<endpoint>?slug=<slug>&resource=instructions` returns the skill body. Auth is a
`X-License-Key` header. HTTP 402/403 mean the license is inactive/invalid and
pause syncing; 429/5xx are retried with backoff.

## Development

```bash
# Core logic — fast, no system dependencies:
cargo test  -p csm-core
cargo clippy -p csm-core --all-targets -- -D warnings

# Live backend check (needs network + a real key; ignored by default):
CSM_ENDPOINT=... CSM_LICENSE_KEY=... \
  cargo test -p csm-core --features net --test live_api -- --ignored --nocapture
```

### Building the desktop app

The app requires the **MSVC C++ toolchain with the Windows SDK headers**
(Tauri compiles native C/C++). On Windows, use the helper which loads the VS
Developer environment first:

```powershell
./scripts/build-local.ps1 "npx tauri dev"     # run locally
./scripts/build-local.ps1 "npx tauri build"   # produce installers
```

(The Tauri CLI is the npm dev-dependency, so use `npx tauri …`. `cargo build
-p customer-skill-manager` also works for a plain compile check.)

If you hit `fatal error C1083: Cannot open include file 'vcruntime.h'`, your VS
install is missing the C++ headers — install *"Desktop development with C++"*
(or the *MSVC v143 build tools* + *Windows 11 SDK* components) via the Visual
Studio Installer.

Cross-platform installers are produced by CI — see below.

## Releases

Releases are built by GitHub Actions (`.github/workflows/release.yml`) on any
`vX.Y.Z` tag, for Windows (NSIS), macOS (universal `.app`/`.dmg`) and Linux
(AppImage), including the signed updater manifest. See
[docs/RELEASE.md](docs/RELEASE.md) for the one-time secret setup and the
tag-to-publish steps.

## License

MIT
