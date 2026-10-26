// Writes the README feature icons (branding/github/icons/*.svg): the same Lucide icons the
// app's ring uses, lime on a pond-green tile so they read in GitHub's light and dark themes.
// usage: node branding/make_icons.mjs   (from the repo root, after npm install in app/)
import { readFileSync, writeFileSync } from "node:fs";

const icons = {
  convert: "arrow-left-right",
  compress: "shrink",
  crop: "crop",
  "add-bg": "image",
  annotate: "pen-line",
  redact: "eye-off",
  edit: "sliders-horizontal",
  metadata: "tag",
};

for (const [name, lucide] of Object.entries(icons)) {
  const src = readFileSync(`app/node_modules/lucide-react/dist/esm/icons/${lucide}.js`, "utf8");
  const nodes = [...src.matchAll(/\["(\w+)", (\{[^}]*\})\]/g)].map(([, tag, attrs]) => {
    const obj = Function(`return ${attrs}`)();
    delete obj.key;
    return `<${tag} ${Object.entries(obj).map(([k, v]) => `${k}="${v}"`).join(" ")}/>`;
  });
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="0 0 40 40">
  <rect width="40" height="40" rx="10" fill="#0E3B24"/>
  <g transform="translate(8 8)" fill="none" stroke="#A6E43A" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    ${nodes.join("\n    ")}
  </g>
</svg>
`;
  writeFileSync(`branding/github/icons/${name}.svg`, svg);
}
console.log(`wrote ${Object.keys(icons).length} icons`);
