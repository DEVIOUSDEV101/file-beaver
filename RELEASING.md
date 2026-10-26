# Releasing File Beaver

How to cut a release. Everything here runs on Windows from the repo root unless noted.

## One-time setup

| What | Where | Notes |
|---|---|---|
| Rust (GNU), MinGW, nasm | `rustup`, `scoop install mingw nasm` | See PLAN.md 3a. |
| Node 20+ | already installed | `npm install` in `app/`. |
| `cargo-about` | `cargo install cargo-about --locked --features cli` | Builds the licence notices. |
| **Updater key** | `%USERPROFILE%\.shiftdrop-keys\updater.key` (+ `.pub`) | **Back it up** (password manager / offline). If it's lost, existing installs can never update again. The public key is in `tauri.conf.json > plugins.updater.pubkey`. |
| Update feed URL | `tauri.conf.json > plugins.updater.endpoints` | Set to `DEVIOUSDEV101/file-beaver`. |
| Code signing | `app/src-tauri/sign.ps1` | Optional but recommended; see below. |

## Cutting a release

1. **Version:** set the same version in `app/package.json`, `app/src-tauri/Cargo.toml`
   and `app/src-tauri/tauri.conf.json`.
2. **Tests:** `cargo test` in `core/`; run the e2e drags (`spike/` harnesses) if anything
   user-facing changed.
3. **Licence notices** (after any dependency change):
   ```
   cd licensing
   cargo about generate --config about.toml --manifest-path ../app/src-tauri/Cargo.toml about.hbs -o rust-notices.txt
   node build-notices.mjs
   ```
   A licence not on the allow-list (`about.toml`) fails here, on purpose.
4. **Build** (in `app/`, PowerShell):
   ```
   $env:TAURI_SIGNING_PRIVATE_KEY = "$env:USERPROFILE\.shiftdrop-keys\updater.key"
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
   $env:SHIFTDROP_SIGN = "cert"   # or "azure", or leave unset to skip code signing
   npx tauri build
   ```
   Output: `app/src-tauri/target/release/bundle/nsis/File Beaver_<version>_x64-setup.exe` and `.sig`.
5. **Update feed:**
   ```
   node scripts/make-latest-json.mjs https://github.com/DEVIOUSDEV101/file-beaver/releases/download/v<version> "What changed"
   ```
6. **Publish** a GitHub release tagged `v<version>` with three files: the `-setup.exe`,
   its `.sig`, and `latest.json`. Installed copies find it within a day (or via Settings >
   Check for updates).

## Code signing options (checked October 2026)

None of these removes the SmartScreen warning on day one: since 2024 every certificate
(OV and EV alike) builds reputation as people download the app. Signing still replaces
"Unknown publisher" with a name and is what that reputation attaches to.

| Option | Cost | Who can use it | `sign.ps1` mode |
|---|---|---|---|
| Azure Artifact Signing | ~$9.99/month | Organisations in US/CA/EU/UK; **individuals only in US/Canada** | `azure` |
| OV certificate (Sectigo, DigiCert, ...) | ~$220-440/year | Individuals or companies; key must be on a USB token or cloud HSM; max ~15-month validity | `cert` |
| SignPath Foundation | Free | Fully open-source projects (OSI licence) with automated public builds; publisher shown is "SignPath Foundation" | signs in CI, not here |

**`cert` mode:** install the certificate (token driver / cloud HSM client) so it appears in
the Windows certificate store, then set `SHIFTDROP_CERT_THUMBPRINT`. Needs `signtool.exe`
from the Windows SDK.

**`azure` mode:** install the Azure Artifact Signing dlib (`Microsoft.ArtifactSigning.Client`
/ "Trusted Signing Client Tools"), create a `metadata.json` with your endpoint, account and
certificate profile, sign in with `az login`, then set `SHIFTDROP_AZURE_DLIB` (path to the
dlib DLL) and `SHIFTDROP_AZURE_METADATA` (path to metadata.json).

## What the installer does

- Per-user install (`%LOCALAPPDATA%\File Beaver`), no admin prompt; installs WebView2 if
  missing (silent bootstrapper).
- Ships `THIRD_PARTY_NOTICES.txt` next to the app.
- Uninstall removes the "launch at startup" entry (`installer-hooks.nsh`). Logs, settings
  and crash reports in `%LOCALAPPDATA%\app.filebeaver.desktop` and `%APPDATA%\app.filebeaver.desktop`
  are kept unless the user ticks "delete app data".
