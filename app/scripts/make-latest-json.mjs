// Writes latest.json, the update feed the app checks (tauri.conf.json > plugins.updater).
// Usage (after `npx tauri build`):
//   node scripts/make-latest-json.mjs <download-base-url> [notes]
// e.g. node scripts/make-latest-json.mjs https://github.com/DEVIOUSDEV101/file-beaver/releases/download/v0.1.1 "Bug fixes"
// Upload latest.json together with the installer and its .sig to the same release.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [base, notes = ""] = process.argv.slice(2);
if (!base) throw new Error("usage: make-latest-json.mjs <download-base-url> [notes]");

const app = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const conf = JSON.parse(readFileSync(join(app, "src-tauri", "tauri.conf.json"), "utf8"));
const dir = join(app, "src-tauri", "target", "release", "bundle", "nsis");
const installer = readdirSync(dir).find((f) => f.endsWith("-setup.exe") && f.includes(conf.version));
if (!installer) throw new Error(`no ${conf.version} installer in ${dir}; run npx tauri build first`);
const signature = readFileSync(join(dir, `${installer}.sig`), "utf8").trim();
const url = `${base.replace(/\/$/, "")}/${encodeURIComponent(installer)}`;

const feed = {
  version: conf.version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": { signature, url },
    "windows-x86_64-nsis": { signature, url },
  },
};
writeFileSync(join(dir, "latest.json"), JSON.stringify(feed, null, 2));
console.log(`wrote ${join(dir, "latest.json")} for ${installer}`);
