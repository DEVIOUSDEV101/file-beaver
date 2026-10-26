# File Beaver

**Drag a file, tap Shift, get things done.** File Beaver adds a radial menu to file drags
on Windows. Drag any image or PDF, tap <kbd>Shift</kbd>, and drop it on what you want:

- **Convert** to JPG, PNG, WEBP, HEIC, TIFF, AVIF, BMP, PDF or DOCX
- **Compress** photos and PDFs
- **Remove metadata** (camera details, GPS location, PDF author info) without re-encoding
- **Edit**, **Crop**, **Annotate**, **Add a background**, or **Redact** in a quick editor

Results are saved next to the original. The original is never changed.

Free and open source (MIT). Works offline: File Beaver only goes online to check for updates.

## Install

Download the latest `File.Beaver_x.y.z_x64-setup.exe` from
[Releases](https://github.com/DEVIOUSDEV101/file-beaver/releases) and run it.
It installs for your user account only (no admin prompt) and updates itself.

Requires Windows 10 or 11 (64-bit). HEIC needs Microsoft's HEIF and HEVC extensions
from the Microsoft Store; File Beaver greys HEIC out if they're missing.

## How to use

1. Start dragging any file (from Explorer, the desktop, a browser download).
2. Tap <kbd>Shift</kbd>: the ring opens with **Convert** options.
3. Tap <kbd>Shift</kbd> again to switch to **Tools**.
4. Drop on a petal. Drop in the centre, drag away, or press <kbd>Esc</kbd> to cancel.

The trigger key (Shift, Ctrl+Shift or Alt) and where results are saved can be changed in
Settings (click the tray icon).

## Privacy

No accounts, no tracking, no uploads. Files are processed on your PC. If File Beaver
crashes, a report is saved locally in `%LOCALAPPDATA%\app.filebeaver.desktop\crashes`;
nothing is sent anywhere.

## Build from source

See [RELEASING.md](RELEASING.md) for the toolchain and release steps, and
[PLAN.md](PLAN.md) for the design and the reasoning behind technical choices.

```
cd core && cargo test          # logic and file-format tests
cd app && npm install && npx tauri build --debug --no-bundle
```

## Credits

Inspired by a radial file tool for macOS. File Beaver is an independent project and is
not affiliated with it. Third-party components and their licences are listed in
[THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).

## Licence

[MIT](LICENSE) © 2026 DEVIOUSDEV101
