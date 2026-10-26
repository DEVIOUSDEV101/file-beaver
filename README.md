<p align="center">
  <img src="branding/github/logo.png" width="120" height="120" alt="File Beaver: a glass beaver holding a lime">
</p>

<h1 align="center">File Beaver</h1>

<p align="center">
  <b>Drag a file onto the beaver. Get it done.</b><br>
  Convert, compress, crop, annotate, redact or clean any image or PDF,<br>
  right on your Windows desktop. Free, offline and open source.
</p>

<p align="center">
  <a href="https://github.com/DEVIOUSDEV101/file-beaver/releases/latest"><img src="branding/github/download-button.png" width="260" height="56" alt="Download for Windows"></a>
</p>

<p align="center">
  <a href="https://github.com/DEVIOUSDEV101/file-beaver/releases/latest"><img src="https://img.shields.io/github/v/release/DEVIOUSDEV101/file-beaver?style=flat-square&label=latest&labelColor=0e3b24&color=a6e43a" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-a6e43a?style=flat-square&labelColor=0e3b24" alt="Windows 10 and 11">
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-MIT-a6e43a?style=flat-square&labelColor=0e3b24" alt="MIT licence"></a>
</p>

<p align="center">
  <a href="#what-it-does">What it does</a> ·
  <a href="#install">Install</a> ·
  <a href="#how-to-use-it">How to use it</a> ·
  <a href="#is-it-safe">Is it safe?</a> ·
  <a href="#faq">FAQ</a> ·
  <a href="https://deviousdev101.github.io/file-beaver/">Website</a>
</p>

<p align="center">
  <img src="branding/github/demo.gif" width="880" alt="A PDF is dragged onto the beaver, the ring of formats opens, it's dropped on JPG and a 'Converted to JPG' card appears">
</p>

## What it does

Drop an image or a PDF on the beaver and a ring of actions opens around him. Every result
is saved as a new file next to the original, so the original is never changed.

| | |
|:---:|---|
| <img src="branding/github/icons/convert.svg" width="36" height="36" alt=""> | **Convert**: change the format. PDF to JPG (one image per page), PNG to JPG, photos to PDF, and more. |
| <img src="branding/github/icons/compress.svg" width="36" height="36" alt=""> | **Compress**: make photos and PDFs smaller for email or upload, and see the size before and after. |
| <img src="branding/github/icons/crop.svg" width="36" height="36" alt=""> | **Crop**: cut an image down freely or to a ratio (1:1, 4:3, 16:9, 9:16 and more). |
| <img src="branding/github/icons/add-bg.svg" width="36" height="36" alt=""> | **Add background**: put a screenshot on a colour or gradient, with padding, rounded corners and a shadow. |
| <img src="branding/github/icons/annotate.svg" width="36" height="36" alt=""> | **Annotate**: add text, arrows, boxes, circles, highlights and numbered steps. |
| <img src="branding/github/icons/redact.svg" width="36" height="36" alt=""> | **Redact**: black out or pixelate names, emails and addresses. In PDFs the text underneath is removed, not just covered. |
| <img src="branding/github/icons/edit.svg" width="36" height="36" alt=""> | **Edit**: rotate, flip, resize, and adjust brightness, contrast and saturation. |
| <img src="branding/github/icons/metadata.svg" width="36" height="36" alt=""> | **Metadata**: remove hidden details such as the phone or camera model and the GPS location a photo was taken at. |

**Opens** JPG, PNG, WEBP, HEIC, TIFF, AVIF, BMP, GIF and PDF.
**Saves** JPG, PNG, WEBP, HEIC, TIFF, AVIF, BMP, PDF and DOCX.

## Install

1. Download `File.Beaver_x.y.z_x64-setup.exe` from the
   [latest release](https://github.com/DEVIOUSDEV101/file-beaver/releases/latest).
2. Open it. It installs in a few seconds, without asking for admin rights.
3. The beaver appears on your desktop. From now on File Beaver starts with Windows and
   keeps itself up to date.

> [!NOTE]
> **Seeing "Windows protected your PC"?** Click **More info**, then **Run anyway**.
> Windows shows this for new apps that aren't code-signed yet and haven't been downloaded
> many times. Everything File Beaver does is in this repository for anyone to check.

## How to use it

**With the beaver**

1. Drag a file from File Explorer, the desktop or a browser's downloads onto the beaver.
2. The ring opens around him. Slide onto an action and let go.
3. Prefer to click? Let go right on the beaver, and the ring stays open until you click an
   action.

He sits on your desktop behind your windows, so he's never in the way, and comes to the
front as soon as you start dragging a file. Drag him to move him; right-click him to hide
him.

**With Shift, anywhere on screen**

Start dragging a file and tap <kbd>Shift</kbd>. The ring opens under your cursor; drop on an
action.

**Good to know**

- Photos open on the tools and PDFs on Convert. Click the middle of the ring, or tap
  <kbd>Shift</kbd> again, to switch.
- To cancel, drop in the middle, drag away from the ring, or press <kbd>Esc</kbd>.
- Drag several files at once to convert or compress them together.
- The tray icon (by the clock) opens Settings: the trigger key, where results go, starting
  with Windows, and showing or hiding the beaver.

## Is it safe?

Yes, and every point below can be checked in this repository.

| | |
|---|---|
| **Your files stay on your PC** | All the work happens on your computer. No server, no upload, no cloud. |
| **No account, no tracking** | File Beaver collects nothing about you or how you use it. |
| **One connection, for updates** | Once a day it checks this repository for a new version. Updates are signed, and File Beaver refuses any update that wasn't signed with the developer's key. |
| **Originals are never touched** | Results are always written as new files. |
| **No admin rights** | It installs for your Windows account only and can't change system settings. |
| **Crash reports stay local** | If it ever crashes, a report is saved on your PC and nothing is sent. |
| **Clean uninstall** | Remove it from **Settings › Apps › Installed apps**; it also takes itself out of startup. |
| **Open source** | The full source is here under the MIT licence. See [SECURITY.md](SECURITY.md) for details and how to report a problem. |

## Use it with an AI assistant

**Help installing or using it.** Paste this into ChatGPT, Claude, Gemini or any assistant
that can read web pages:

```text
I want to use File Beaver on my Windows PC: https://github.com/DEVIOUSDEV101/file-beaver
Read the README, then walk me through installing it step by step and show me how to
convert, compress and crop a file with it. Ask me what I see if anything goes wrong.
```

**Change it with an AI coding agent.** The project is set up for Claude Code, OpenAI Codex,
Cursor, GitHub Copilot and similar tools: [AGENTS.md](AGENTS.md) tells them how the code is
laid out, how to build and test it, and which rules to keep. Clone the repository, open the
folder in your agent, and ask for what you want:

```text
Set up everything I need to build File Beaver on this PC, then build it.
Add ICO as a Convert option, with a test.
Add a setting to make the beaver bigger.
```

## FAQ

<details>
<summary><b>Can it convert Word documents to PDF?</b></summary>
<br>
Not yet; it's being built now. Today it opens images and PDFs. It can save a PDF as DOCX,
with each page as a picture rather than editable text.
</details>

<details>
<summary><b>Why is HEIC greyed out?</b></summary>
<br>
Windows needs Microsoft's free <i>HEIF Image Extensions</i> to open HEIC files and the
<i>HEVC Video Extensions</i> to save them, both from the Microsoft Store. File Beaver greys
HEIC out until they're installed.
</details>

<details>
<summary><b>Where do the results go?</b></summary>
<br>
Next to the original file, named after it (for example <code>Photo Cropped.jpg</code>). You
can choose one fixed folder instead in Settings.
</details>

<details>
<summary><b>Can I hide the beaver and just use Shift?</b></summary>
<br>
Yes. Right-click him and choose <b>Hide beaver</b>, or untick "Show the beaver on screen"
in Settings. Shift keeps working.
</details>

<details>
<summary><b>Does it work on Mac or Linux?</b></summary>
<br>
Windows only, for now.
</details>

## Build from source

You'll need Rust (GNU toolchain) with MinGW and nasm, and Node.js 20 or later.
[RELEASING.md](RELEASING.md) has the exact setup and release steps, and [PLAN.md](PLAN.md)
explains the design and the reasoning behind each technical choice.

```sh
cd core && cargo test                                     # file formats and logic
cd app && npm install && npx tauri build --debug --no-bundle
```

Changes are listed in [CHANGELOG.md](CHANGELOG.md).

## Credits

Inspired by a radial file tool for macOS; File Beaver is an independent project and isn't
affiliated with it. Icons by [Lucide](https://lucide.dev). Third-party components and their
licences are listed in [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).

## Licence

[MIT](LICENSE) © 2026 DEVIOUSDEV101
