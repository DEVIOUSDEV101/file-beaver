# Shift-Drag File Tool for Windows: Full Build Plan

Working name: **ShiftDrop** (placeholder; pick a real name before release).
Goal: match every function of the macOS tool in the reference video on Windows 10/11, then redesign the look, then release it free.

---

## 0. What the reference app does (from the video)

Evidence: 13 frames taken from the 25s clip. Anything marked *(unverified)* was not clearly visible and must be confirmed against the real app before we copy it.

| # | Behaviour seen | Detail |
|---|---|---|
| 1 | **Trigger** | User drags a file on the desktop and holds **Shift**. A radial menu appears under the cursor. Without Shift it's a normal drag. |
| 2 | **Radial menu** | Round, frosted-glass, warm orange tint, about 8 wedges with icon + label: **Compress, Metadata, Edit, Annotate, Add BG, Crop, Redact**, plus a **convert** target (top/centre). The file icon "rides" the cursor while hovering wedges; the hovered wedge highlights. |
| 3 | **Drop = action** | Releasing the file on a wedge runs that action. |
| 4 | **Convert** | PDF dropped → small floating card "Converting to JPG… 1 of 1 file(s)" with progress bar and a close (x). Output `Page 001.jpg` appears **next to the original**; original untouched. |
| 5 | **Crop** | Opens a "Crop Image" window: preview with draggable crop box, aspect-ratio chips (Free, 1:1, 4:3, 16:9 etc. *(exact list unverified)*), W/H number fields, Cancel / Save. Output `… Cropped.jpg`. |
| 6 | **Add BG** | Opens an "Add BG" window: image placed on a gradient/colour background, swatch grid (solid colours + gradients), sliders (padding, corner radius, shadow, *(4th unverified)*), Cancel / "Save with Background". Output `… Cropped With BG.jpg`. |
| 7 | **Chaining** | Outputs are normal files, so you can Shift-drag the result again (video does PDF → JPG → Crop → Add BG). |
| 8 | **Compress, Metadata, Edit, Annotate, Redact** | Wedges visible, not demonstrated. Behaviour below is our best inference *(unverified)*. |

**Step 0 before coding:** find the real app's name (the creator is @metav3rse on Instagram) and read its feature list/site, so "exact" means exact. One hour, saves rework.

---

## 1. Full feature spec (what we will build)

### 1.1 Trigger and menu
- Works on files dragged from **Explorer, Desktop, browser downloads bar, any app that drags real files** (CF_HDROP).
- Shift held at any point during the drag → menu fades/scales in centred on the cursor. Shift released before dropping → menu hides, the drag goes back to normal.
- Escape or dropping outside the ring → cancel, nothing happens.
- Wedges are **filtered by file type** (e.g. Crop/Add BG greyed for a .docx). Multi-file drags supported where the action makes sense (batch convert/compress).
- Never moves or deletes the source. Always writes a new file beside it.

### 1.2 Actions

| Action | Inputs | What it does | Output |
|---|---|---|---|
| **Convert** | PDF, PNG, JPG, WEBP, HEIC, AVIF, BMP, TIFF, GIF, SVG | Hover shows a second ring/strip of target formats (JPG, PNG, WEBP, PDF, …). PDF → one image per page (`Page 001.jpg`…). Images → PDF (multi-file → one PDF). | `<name>.<ext>` beside source |
| **Compress** | Images, PDF, (video *(unverified)*) | Quick presets (Low / Medium / High) on hover; default Medium. Images: re-encode with mozjpeg/oxipng/webp quality. PDF: re-compress embedded images, strip unused objects. Toast shows "2.4 MB → 610 KB". | `<name> Compressed.<ext>` |
| **Metadata** | Images, PDF | Panel showing EXIF/GPS/camera/author/dates; buttons **Strip all** and **Strip location only**; editable title/author for PDF. | `<name> Clean.<ext>` or in place on confirm |
| **Edit** | Images | Basic adjust window: rotate, flip, resize (px/%), brightness/contrast/saturation. | `<name> Edited.<ext>` |
| **Annotate** | Images, PDF pages | Draw: arrow, rectangle, ellipse, freehand, text, highlighter, numbered step badges; colour + stroke width; undo/redo. | `<name> Annotated.<ext>` |
| **Add BG** | Images | Background (solid, gradient, image presets), padding, corner radius, shadow, aspect (auto/1:1/16:9/4:5). The "screenshot beautifier" effect. | `<name> With BG.png/jpg` |
| **Crop** | Images, PDF page | Crop box, aspect chips, W/H fields, rotate 90°. | `<name> Cropped.<ext>` |
| **Redact** | Images, PDF | Drag boxes over areas → solid black (or blur/pixelate for images). **PDF redaction must be real**: the redacted page is rasterised, so text under the box cannot be copied out. | `<name> Redacted.<ext>` |

### 1.3 Supporting behaviour
- **Progress card** near the cursor/bottom-right: title, "n of m files", bar, cancel. Auto-dismisses 1.5s after success; click → reveal file in Explorer.
- **Naming**: `Name Suffix.ext`; on clash `Name Suffix (2).ext`.
- **Tray icon**: enable/disable, settings, quit, check for updates.
- **Settings**: trigger key (Shift / Ctrl+Shift / Alt), default formats and quality, output folder (beside source / fixed folder), launch at startup, theme.
- **Undo safety**: originals are never touched, so nothing needs undoing.
- **Offline only**: no network except the optional update check. Matters for a free privacy tool.

---

## 2. How it works on Windows (the hard part)

### 2.1 Detecting "file being dragged + Shift"
Windows has no public "a drag started anywhere" event. The plan:

1. **Global low-level mouse + keyboard hooks** (`SetWindowsHookEx(WH_MOUSE_LL / WH_KEYBOARD_LL)`) in a small Rust background process. We track: left button down → movement past the drag threshold (`SM_CXDRAG`) → Shift down.
2. When that combination happens, we **show the overlay window under the cursor**. It is a borderless, topmost, layered window registered as an OLE drop target (`RegisterDragDrop`, `IDropTarget`).
3. Because a real OLE drag is in progress, Windows itself delivers `DragEnter / DragOver / Drop` to our window with the `IDataObject`. If it contains `CF_HDROP`, it's a file drag and we read the paths. If not (text, a window being moved), we hide immediately. **This is what makes it reliable**: we don't guess what is being dragged, Windows tells us.
4. `DragOver` gives the cursor position → we work out which wedge is hovered and animate it. We return `DROPEFFECT_COPY` so Explorer never moves or deletes the file (Shift normally means "move").
5. `Drop` → read paths → hand to the action engine.

Risks and fixes:
- Elevated (admin) Explorer windows can't drop onto a normal process (UIPI). Rare; document it.
- Some apps run their own drag loop. Fine, OLE still delivers to us.
- Antivirus may flag low-level hooks. Fix: **code-sign the installer** (see §6) and keep hooks minimal.
- Prototype this piece **first**. If it fails, the fallback is a hotkey that pops the ring and accepts a normal drop.

### 2.2 Overlay look
- Window: WS_EX_LAYERED | TOPMOST | TOOLWINDOW (no taskbar entry), per-monitor DPI aware, appears on whichever monitor the cursor is on.
- Glass: Windows 11 **Acrylic/Mica** backdrop via `DwmSetWindowAttribute`; Windows 10 fallback = semi-opaque blur-less tint.
- Drawn in the web UI (SVG wedges + CSS transitions) so the later redesign is a UI-only change.

---

## 3. Tech stack

| Layer | Choice | Why |
|---|---|---|
| App shell | **Tauri 2** (Rust + WebView2) | Small installer (~10 MB vs 100+ MB Electron), native Win32 access from Rust, web UI for fast design work. WebView2 ships with Win10/11. |
| UI | React + TypeScript + Tailwind; canvas editor via **Konva** | Crop/Annotate/Redact/Add BG editors are canvas work; Konva handles transforms, handles, undo. |
| Hooks / drop target | Rust `windows` crate (official Microsoft bindings) | Direct Win32/OLE/COM. |
| Images | Rust `image`, `fast_image_resize`, **mozjpeg** (JPG), **oxipng** (PNG), `webp`, `libheif` (HEIC/AVIF decode) | All permissive licences, fast, bundled, no external installs. |
| PDF render/edit | **PDFium** via `pdfium-render` (BSD/Apache) + `lopdf` for writing | Renders pages to images, builds PDFs from images, re-compresses. **Avoid Ghostscript and MuPDF**: they are AGPL, which would force open-sourcing under AGPL. |
| Metadata | `little_exif` / `kamadak-exif` for images; `lopdf` for PDF Info/XMP | Read and strip. |
| SVG | `resvg` | Render SVG → PNG. |
| Installer / updates | Tauri bundler (MSI + NSIS) + Tauri updater (signed manifests on GitHub Releases) | Free hosting, signed updates. |
| Startup | Registry `Run` key, toggle in settings | Standard. |

### 3a. What was actually built (updated after Phase 6)

The table above was the starting plan. Where the build differs, this is what ships:

| Layer | Built with | Why it changed |
|---|---|---|
| Toolchain | Rust GNU target + MinGW (Scoop) | No Visual Studio needed. Revisit MSVC for release (see mozjpeg below). |
| UI | React + plain `<canvas>` (no Konva) | One shared renderer (`editor/render.ts`) draws previews and full-res saves identically. |
| JPEG encode | `jpeg-encoder` (pure Rust, progressive + optimised Huffman) | **mozjpeg rejected**: built with MinGW it fails on noisy photos ("Missing Huffman code table entry") and crashes without SIMD. Measured: 13% smaller than baseline at q72, same fidelity. |
| WebP encode | `libwebp` (via `webp`), effort 2 + threads | 2.1x faster than default effort 4, -0.56 dB at ~40 dB. **SSE4.1/AVX2 builds left off**: they compile the whole library for those CPUs and would crash older ones. |
| AVIF encode | `image` crate (rav1e) | Pure Rust. |
| HEIC / AVIF decode, HEIC encode | Windows Imaging Component + Microsoft Store extensions | No GPL codecs. The app probes at startup (HEIC with a real test encode) and greys out what this PC can't do. |
| PDF render | Windows' built-in `Windows.Data.Pdf` (BMP output) | No DLL to ship. Needs a scale correction (it multiplies sizes by display scaling) and a byte-stream path for >260-char paths. |
| PDF write / edit | Own image-to-PDF writer + `lopdf` | Crop = CropBox (lossless), Annotate = vector overlay, Redact = flatten every page. |
| Parallelism | `rayon` pool, half the cores, 2 to 6 workers | Memory cap: each worker can hold a decoded photo. |
| Crash safety | Panic hook + unhandled-exception minidump, all local | `panic = "unwind"` in release so one job can fail without closing the app. |

---

## 4. Architecture

```
shiftdrop.exe (one process)
├─ hook thread (Rust)       LL mouse/keyboard hooks → "show ring at x,y"
├─ overlay window           IDropTarget → hover wedge / drop(paths, action)
├─ action engine (Rust)     job queue, worker pool, one module per action
│   convert · compress · metadata · edit · annotate · addbg · crop · redact
├─ editor windows (WebView) Crop / Add BG / Annotate / Redact / Edit / Metadata
│   UI sends parameters → engine renders the full-resolution output
├─ progress cards (WebView) small topmost toasts
└─ tray + settings          settings.json in %APPDATA%\ShiftDrop
```

Editors show a **preview** in the browser canvas but the **final file is rendered in Rust** at full resolution, so output quality does not depend on the WebView.

---

## 5. Build phases

| Phase | Work | Done when | Estimate |
|---|---|---|---|
| **0. Verify** | Identify the original app, confirm the full feature list, fill in the *(unverified)* rows | Spec frozen | 0.5 day |
| **1. Trigger spike** | Hooks + overlay drop target + read paths + DROPEFFECT_COPY | Shift-drag from Explorer, Desktop, Chrome shows the ring and logs the paths, no file ever moved | 2–3 days |
| **2. Shell** | Tauri app, tray, settings, autostart, single instance, logging | Runs from tray, survives reboot | 1–2 days |
| **3. Radial menu UI** | Wedges, hover, file-type filtering, convert sub-ring, glass, DPI/multi-monitor | Feels like the video | 2–3 days |
| **4. Quick actions** | Convert, Compress, Metadata strip + progress card + naming rules | PDF→JPG flow in the video works end to end | 3–4 days |
| **5. Editors** | Crop, Add BG, Edit, Annotate, Redact (incl. real PDF redaction), Metadata panel | Full video flow (PDF→JPG→Crop→Add BG) works | 6–8 days |
| **6. Hardening** | Big files (200 MB PDFs, 50 MP images), batch of 100, odd formats, admin windows, AV false-positive check, crash reporting (opt-in) | Test matrix green | 3–4 days |
| **7. Release** | Code signing, MSI/NSIS installer, auto-update, landing page, winget listing | Installs clean on fresh Win10 + Win11 VMs | 2–3 days |
| **8. Redesign** | Your new design system applied to ring, cards, editors | Looks like yours, not theirs | separate |

**Total for full feature parity: about 4–5 weeks** of focused work, me building and you testing on your PC each phase.

---

## 6. Release and legal checklist (because it ships free to the public)
- **Copy the features, not the identity.** Features and ideas are fair game. The name, icon set and exact visual design are theirs, so **phase 8 must happen before public release**, not after. Use our own icons (e.g. Lucide, MIT).
- Licences: only MIT/BSD/Apache/zlib dependencies (checked with `cargo-deny`). No AGPL (Ghostscript, MuPDF). PDFium is fine.
- **Code-signing certificate** (~£70–£250/yr, or free via SignPath for open-source projects). Without it SmartScreen warns every user and AV tools may flag the hooks.
- Privacy: fully offline; say so on the site. That is a selling point.
- Decide: open-source on GitHub (free signing via SignPath, community trust) or closed-free. Recommendation: **open source**.

---

## 7. Test matrix
- OS: Windows 10 22H2, Windows 11 23H2/24H2; 100% / 150% / 200% scaling; dual monitor with mixed DPI.
- Sources: Explorer, Desktop, Chrome/Edge downloads, Outlook attachment, VS Code explorer.
- Files: 1-page and 300-page PDF, encrypted PDF (clear error), HEIC from iPhone, animated GIF, transparent PNG, 0-byte file, read-only folder (fallback to Downloads), long paths > 260 chars, non-ASCII names.
- Safety: source file hash unchanged after every action; redacted PDF text not extractable (`pdftotext` check).

---

## 8. Open questions for you
1. Name for the app (needed before release, not before building).
2. Open source or closed-but-free?
3. Willing to buy a code-signing cert, or go open-source + SignPath?
4. Windows 10 support, or Windows 11 only (simpler glass effects)?
