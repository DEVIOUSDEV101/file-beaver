import { addBgLayout, AddBgState, Background, BgAspect, renderAddBg } from "./render";
import { Chips, Paint, Section, Slider, Stage } from "./ui";

const SOLIDS = ["#ffffff", "#111111", "#f4ede4", "#ff6b35", "#ffd166", "#06d6a0", "#118ab2", "#ef476f", "#8338ec"];
const GRADIENTS: string[][] = [
  // First = default: File Beaver's lime fading into pond green.
  ["#c8f272", "#2f6b3f", "#0e3b24"],
  ["#ff9a8b", "#ff6a88", "#ff99ac"],
  ["#f6d365", "#fda085"],
  ["#fbc2eb", "#a6c1ee"],
  ["#84fab0", "#8fd3f4"],
  ["#a18cd1", "#fbc2eb"],
  ["#ff7e5f", "#feb47b"],
  ["#43cea2", "#185a9d"],
  ["#30cfd0", "#330867"],
  ["#0f2027", "#2c5364"],
];

const ASPECTS: { value: BgAspect; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "1:1", label: "1:1" },
  { value: "4:3", label: "4:3" },
  { value: "16:9", label: "16:9" },
  { value: "4:5", label: "4:5" },
  { value: "9:16", label: "9:16" },
];

export const ADD_BG_DEFAULTS: AddBgState = {
  bg: { kind: "gradient", colors: GRADIENTS[0], angle: 135 },
  padding: 0.08,
  radius: 0.03,
  shadow: 0.5,
  aspect: "auto",
};

const same = (a: Background, b: Background) => a.kind === b.kind && a.colors.join() === b.colors.join();

function swatchStyle(bg: Background) {
  return { background: bg.kind === "solid" ? bg.colors[0] : `linear-gradient(${bg.angle}deg, ${bg.colors.join(", ")})` };
}

export default function AddBgTool({ img, s, set }: { img: ImageBitmap; s: AddBgState; set: (s: AddBgState) => void }) {
  const { W, H } = addBgLayout(img.width, img.height, s);
  const swatches: Background[] = [
    ...GRADIENTS.map((colors) => ({ kind: "gradient" as const, colors, angle: 135 })),
    ...SOLIDS.map((c) => ({ kind: "solid" as const, colors: [c], angle: 0 })),
  ];
  const pct = (v: number) => `${Math.round(v * 100)}%`;
  return (
    <>
      <Stage w={W} h={H}>
        {(dw, dh) => <Paint dw={dw} dh={dh} draw={(c, cw) => c.drawImage(renderAddBg(img, s, cw / W), 0, 0)} />}
      </Stage>
      <aside className="panel">
        <Section title="Background">
          <div className="swatches">
            {swatches.map((bg, i) => (
              <button
                key={i}
                className={same(bg, s.bg) ? "swatch on" : "swatch"}
                style={swatchStyle({ ...bg, angle: 135 })}
                title={bg.colors.join(" → ")}
                onClick={() => set({ ...s, bg: { ...bg, angle: s.bg.kind === "gradient" ? s.bg.angle : 135 } })}
              />
            ))}
            <label className="swatch custom" title="Custom colour">
              <input
                type="color"
                value={s.bg.kind === "solid" ? s.bg.colors[0] : "#ffffff"}
                onChange={(e) => set({ ...s, bg: { kind: "solid", colors: [e.target.value], angle: 0 } })}
              />
            </label>
          </div>
          {s.bg.kind === "gradient" && (
            <Slider label="Gradient angle" value={s.bg.angle} min={0} max={360} format={(v) => `${v}°`} onChange={(angle) => set({ ...s, bg: { ...s.bg, angle } })} />
          )}
        </Section>
        <Section title="Layout">
          <Slider label="Padding" value={s.padding} min={0} max={0.3} step={0.005} format={pct} onChange={(padding) => set({ ...s, padding })} />
          <Slider label="Corner radius" value={s.radius} min={0} max={0.2} step={0.005} format={pct} onChange={(radius) => set({ ...s, radius })} />
          <Slider label="Shadow" value={s.shadow} min={0} max={1} step={0.01} format={pct} onChange={(shadow) => set({ ...s, shadow })} />
        </Section>
        <Section title="Canvas">
          <Chips value={s.aspect} options={ASPECTS} onChange={(aspect) => set({ ...s, aspect })} />
          <p className="note">
            {Math.round(W)} × {Math.round(H)} px
          </p>
        </Section>
      </aside>
    </>
  );
}
