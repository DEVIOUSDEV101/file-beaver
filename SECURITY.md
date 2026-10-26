# Security

## Reporting a problem

If you find a security problem in File Beaver, please report it privately through
[GitHub's private vulnerability reporting](https://github.com/DEVIOUSDEV101/file-beaver/security/advisories/new)
rather than in a public issue. Include what you found, how to reproduce it and which
version you were using (Settings shows it). You'll get a reply within a few days.

Fixes ship as a new release, and installed copies update themselves.

## How File Beaver protects you

- **Files stay on your PC.** Files are processed locally. Nothing is uploaded and there is
  no server.
- **One network connection.** The only thing File Beaver fetches is the update feed
  (`latest.json` from this repository's releases), once a day.
- **Signed updates.** Every update is signed with a private key that only the developer
  holds. The app checks the signature against the public key built into it and refuses
  anything that doesn't match, so a tampered or substituted download can't be installed.
- **No admin rights.** It installs for the current Windows user and runs without
  elevation.
- **Originals are read, never written.** Every result is a new file.
- **Crash reports stay local.** They're saved in
  `%LOCALAPPDATA%\app.filebeaver.desktop\crashes` and never sent.

## Supported versions

Only the latest release gets fixes. File Beaver updates itself, so this is normally the
version you already have.
