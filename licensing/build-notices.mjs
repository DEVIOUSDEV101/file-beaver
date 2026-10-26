// Builds THIRD_PARTY_NOTICES.txt (shipped in the installer) from:
//   1. rust-notices.txt  - every Rust crate, from `cargo about` (licence allow-list in about.toml)
//   2. vendored C code whose licence the wrapping crate doesn't carry (libwebp)
//   3. the npm packages compiled into the UI (production dependencies only)
// Run from this folder:  node build-notices.mjs
// Re-run after changing dependencies (see RELEASING.md).

import { execSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const here = new URL(".", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const app = join(here, "..", "app");
const out = join(here, "..", "THIRD_PARTY_NOTICES.txt");
const rule = "-".repeat(88);
const parts = [];

// 1. Rust crates
parts.push(readFileSync(join(here, "rust-notices.txt"), "utf8").trimEnd());

// 2. libwebp: the libwebp-sys crate is MIT, but the C library it builds is BSD-3-Clause
//    with Google's additional patent grant. Both texts ship as-is.
const registry = join(homedir(), ".cargo", "registry", "src");
const webpDir = readdirSync(registry)
  .map((r) => join(registry, r))
  .flatMap((r) => readdirSync(r).filter((d) => d.startsWith("libwebp-sys-")).map((d) => join(r, d, "vendor")))
  .find((d) => existsSync(join(d, "COPYING")));
if (!webpDir) throw new Error("libwebp sources not found; run a cargo build first");
for (const f of ["COPYING", "PATENTS"]) {
  parts.push(`${rule}\nlibwebp (bundled C library) - ${f}\n${rule}\n\n${readFileSync(join(webpDir, f), "utf8").trimEnd()}`);
}

// 3. npm packages in the shipped UI bundle
const tree = JSON.parse(execSync("npm ls --omit=dev --all --json", { cwd: app, encoding: "utf8" }));
const seen = new Map();
const walk = (deps) => {
  for (const [name, info] of Object.entries(deps ?? {})) {
    const key = `${name}@${info.version}`;
    if (seen.has(key)) continue;
    seen.set(key, { name, version: info.version });
    walk(info.dependencies);
  }
};
walk(tree.dependencies);
for (const { name, version } of [...seen.values()].sort((a, b) => a.name.localeCompare(b.name))) {
  const dir = join(app, "node_modules", name);
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  const licenceFile = readdirSync(dir).find((f) => /^(licen[cs]e|copying)/i.test(f));
  if (!licenceFile) throw new Error(`no licence file in ${name}`);
  const id = typeof pkg.license === "string" ? pkg.license : JSON.stringify(pkg.license);
  if (!/^(MIT|ISC|Apache-2\.0|BSD-2-Clause|BSD-3-Clause|0BSD|Apache-2\.0 OR MIT|MIT OR Apache-2\.0)$/.test(id)) {
    throw new Error(`${name}: licence ${id} is not on the allow-list`);
  }
  parts.push(`${rule}\n${name} ${version} (npm, ${id})\n${rule}\n\n${readFileSync(join(dir, licenceFile), "utf8").trimEnd()}`);
}

writeFileSync(out, parts.join("\n\n") + "\n");
console.log(`wrote ${out}: ${seen.size} npm packages + libwebp + Rust crates`);
