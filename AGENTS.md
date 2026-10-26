# Working on File Beaver (for AI coding agents)

File Beaver is a Windows desktop app: a floating beaver (and a Shift-while-dragging trigger)
opens a radial menu of file actions. Read this before changing anything. Humans are welcome
to read it too.

## Layout

| Path | What's there |
|---|---|
| `core/` | All file logic in plain Rust: formats, decode/encode, convert, compress, metadata, PDF read/write/edit, output naming, ring geometry, menu building. **Every test lives here.** |
| `app/src-tauri/` | The Tauri 2 shell. `trigger.rs` (mouse/keyboard hooks, the drop target, ring placement), `beaver.rs` (the floating widget), `jobs.rs` (job queue + progress card), `editor.rs`, `updates.rs`, `crash.rs`, `settings.rs`. |
| `app/src/` | React + TypeScript UI: `Ring.tsx` (the menu), `Toast.tsx` (progress card), `App.tsx` (settings), `editor/` (Crop, Add background, Edit, Annotate, Redact). |
| `spike/` | End-to-end harnesses that drive the real mouse and keyboard (`ring_test`, `editor_test`, `beaver_test`, `demo`). |
| `licensing/` | Third-party licence allow-list and notices builder. |
| `PLAN.md` | The spec and, in section 3a, what was actually built and why. Read 3a before touching codecs. |
| `RELEASING.md` | Toolchain setup and the release runbook. |

## Toolchain

- Rust **GNU** target with MinGW and nasm (`scoop install mingw nasm`). No Visual Studio.
- Node.js 20+ (`npm install` in `app/`).
- Build a runnable app with `npx tauri build --debug --no-bundle` in `app/`.
  Plain `cargo build` in `app/src-tauri` produces an exe that expects the Vite dev server.
- Release builds: `npx tauri build` (see RELEASING.md for the updater signing key variables).

## Tests

- `cd core && cargo test` must pass before any change is done.
- The Tauri crate's unit tests can't run under MinGW (manifest issue), so put testable logic
  in `core/` and keep `app/src-tauri` thin.
- UI changes: run the app and check them. The `spike/` harnesses take over the real mouse,
  so warn the person before running them.

## Rules to keep

1. **Never modify or delete a user's original file.** Results are new files named by
   `core/src/naming.rs` ("Name Suffix.ext", then "Name Suffix (2).ext").
2. **No network access** except the updater (`updates.rs`, GitHub releases). No telemetry,
   no uploads, no remote content in the webviews.
3. **Ring geometry lives in three places that must match:** `app/src/Ring.tsx`
   (WIN, R, HUB, petal radii), `app/src-tauri/src/trigger.rs` (WIN_LOGICAL,
   RING_R_LOGICAL) and `core/src/geometry.rs` (hit-testing). Change all three together.
4. **Windows callbacks must never panic into Windows.** Wrap hook procs, window procs, COM
   callbacks and job bodies in `crash::guarded`.
5. **The hooks thread must stay responsive.** Low-level hooks only record state and post
   messages; slow work (window placement, file I/O) happens elsewhere, or Windows starts
   dropping key events.
6. **Physical pixels everywhere** (per-monitor DPI aware v2). Convert to logical only at the
   web UI boundary.
7. **Known dead ends** (measured; don't retry without new evidence): mozjpeg under MinGW
   (corrupt output on noisy photos), libwebp's SSE4.1/AVX2 features (crashes older CPUs).
8. **Dependencies:** a new crate or npm package must have a licence on the allow-list in
   `licensing/about.toml`; regenerate the notices as described in RELEASING.md.
9. Keep user-facing text plain and short, in sentence case.

## Where to start for common changes

- **New convert format:** `core/src/formats.rs` (detect, read/write capability),
  `core/src/encode.rs` / `decode.rs`, the target list in `core/src/menu.rs`, plus a test.
- **New tool:** the `TOOLS` list in `core/src/menu.rs`, an action in `core/src/actions.rs`
  (quick action) or an editor in `app/src/editor/` wired up in `app/src-tauri/src/editor.rs`.
- **Beaver behaviour:** `app/src-tauri/src/beaver.rs` and the beaver parts of `trigger.rs`.
- **Settings:** `app/src-tauri/src/settings.rs` and `app/src/App.tsx`.
