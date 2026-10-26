# File Beaver

A little beaver that lives on your Windows desktop. Drag a file onto him, pick what you want
done, and it's done: convert, compress, crop, add a background, annotate, redact, edit or
remove hidden metadata. No website, no upload, no account.

![Dragging a PDF onto the beaver and converting it to JPG](branding/demo.gif)

**[Download for Windows](https://github.com/DEVIOUSDEV101/file-beaver/releases/latest)**
· Free and open source · Windows 10 and 11 (64-bit)

---

## What it does

Drop an image or a PDF on the beaver and a ring of actions opens around him.

| Action | What it's for |
|---|---|
| **Convert** | Change a file's format: PDF to JPG (one image per page), PNG to JPG, images to PDF, and more. |
| **Compress** | Make photos and PDFs smaller so they fit in an email or upload faster. Shows the size before and after. |
| **Crop** | Cut an image down, freely or to a ratio (1:1, 4:3, 16:9, 9:16 and others). |
| **Add background** | Put a screenshot on a colour or gradient with padding, rounded corners and a shadow. |
| **Annotate** | Add text, arrows, boxes, circles, highlights and numbered steps. |
| **Redact** | Black out or pixelate names, emails, addresses. On PDFs the text underneath is really removed, not just covered. |
| **Edit** | Rotate, flip, resize, and adjust brightness, contrast and saturation. |
| **Metadata** | Remove hidden details such as the camera or phone model and the GPS location a photo was taken at. |

**File types it opens:** JPG, PNG, WEBP, HEIC, TIFF, AVIF, BMP, GIF and PDF.
**File types it saves:** JPG, PNG, WEBP, HEIC, TIFF, AVIF, BMP, PDF and DOCX.

Every result is saved as a new file next to the original (for example `Photo Cropped.jpg`).
Your original file is never changed.

## Install

1. Download `File.Beaver_x.y.z_x64-setup.exe` from the
   [latest release](https://github.com/DEVIOUSDEV101/file-beaver/releases/latest).
2. Open it. It installs in a few seconds, without asking for admin rights.
3. The beaver appears on your desktop. File Beaver starts with Windows from now on and
   keeps itself up to date.

> **If Windows says "Windows protected your PC":** click **More info**, then
> **Run anyway**. Windows shows this for new apps that haven't been downloaded many
> times yet and aren't code-signed. File Beaver will be signed once it's eligible for free
> signing; until then, everything it does is in this repository for anyone to check.

## How to use it

**With the beaver**

1. Drag a file from File Explorer, the desktop or a browser's downloads onto the beaver.
2. The ring opens around him. Slide onto an action and let go.
3. Prefer clicking? Let go right on the beaver: the ring stays open and you click an action.

The beaver sits on your desktop behind your windows, so he's never in the way. When you
start dragging a file, he comes to the front so you can reach him. Drag him to move him;
right-click him to hide him.

**With Shift, anywhere on screen**

1. Start dragging a file and tap <kbd>Shift</kbd>. The ring opens under your cursor.
2. Drop on an action.

**Tips**

- Images open on the tools (Crop, Add background…) and PDFs on Convert. Click the middle
  of the ring, or tap <kbd>Shift</kbd> again, to switch.
- To cancel, drop in the middle of the ring, drag away from it, or press <kbd>Esc</kbd>.
- Drag several files at once to convert or compress them together.
- The tray icon (bottom right, by the clock) has Settings: the trigger key, where results
  are saved, starting with Windows, and showing or hiding the beaver.

## Is it safe?

Yes, and you don't have to take our word for it. Everything below can be checked in this
repository.

- **Your files never leave your PC.** All the work happens on your computer. There is no
  server, no upload and no cloud.
- **No account, no tracking, no ads.** File Beaver doesn't collect any data about you or
  how you use it.
- **One connection, for updates only.** Once a day it checks this GitHub page for a new
  version. Updates are cryptographically signed, and File Beaver refuses any update that
  wasn't signed with the developer's private key.
- **Your originals are never touched.** Results are always written as new files.
- **No admin rights.** It installs for your Windows account only and can't change system
  settings.
- **Crash reports stay on your PC.** If it ever crashes, a report is saved locally and
  nothing is sent anywhere.
- **Clean uninstall.** Remove it any time from Windows **Settings > Apps > Installed apps**.
  It also removes itself from startup.
- **Open source (MIT).** The full source code is here, and each release is built from it.

## Use it with an AI assistant

### Getting help installing or using it

Paste this into ChatGPT, Claude, Gemini or any assistant that can read web pages:

```
I want to use File Beaver on my Windows PC: https://github.com/DEVIOUSDEV101/file-beaver
Read the README and walk me through installing it step by step, then show me how to
convert, compress and crop a file with it. Ask me what I see if anything goes wrong.
```

### Changing it with an AI coding agent

File Beaver is set up so AI coding agents can work on it. [AGENTS.md](AGENTS.md) tells
them how the project is laid out, how to build it, how to test it and which rules to keep.
Claude Code, OpenAI Codex, Cursor, GitHub Copilot and similar tools read it automatically.

1. Install [Git](https://git-scm.com/download/win), then clone the project:
   `git clone https://github.com/DEVIOUSDEV101/file-beaver`
2. Open the folder in your agent (for example run `claude` or `codex` inside it).
3. Ask for what you want, for example:
   - "Set up everything I need to build File Beaver on this PC, then build it."
   - "Add ICO as a Convert option and add a test for it."
   - "Make the beaver bigger and add a size option in Settings."

## FAQ

**Can it convert Word documents to PDF?** Not yet. It opens images and PDFs. (It can save
a PDF as DOCX, with each page as a picture rather than editable text.)

**Why is HEIC greyed out?** Windows needs Microsoft's free *HEIF Image Extensions* (to open
HEIC) and *HEVC Video Extensions* (to save HEIC) from the Microsoft Store. File Beaver
greys HEIC out until they're installed.

**Where do results go?** Next to the original file. You can pick one fixed folder instead
in Settings.

**Can I get rid of the beaver and just use Shift?** Yes. Right-click him and choose Hide
beaver, or untick "Show the beaver on screen" in Settings.

**Mac or Linux?** Windows only for now.

## Build from source

You need Rust (GNU toolchain), MinGW, nasm and Node.js 20+. [RELEASING.md](RELEASING.md)
lists the exact setup and how releases are made; [PLAN.md](PLAN.md) explains the design and
why each technical choice was made.

```
cd core && cargo test                                   # file-format and logic tests
cd app && npm install && npx tauri build --debug --no-bundle
```

## Credits

Inspired by a radial file tool for macOS. File Beaver is an independent project and isn't
affiliated with it. Third-party components and their licences are listed in
[THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).

## Licence

[MIT](LICENSE) © 2026 DEVIOUSDEV101
