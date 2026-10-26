import { FlipHorizontal2, FlipVertical2, RotateCcw, RotateCw } from "lucide-react";
import { EDIT_DEFAULTS, EditState, orientedSize, renderEdit, Rotation } from "./render";
import { IconButton, Paint, Section, Slider, Stage } from "./ui";

export default function EditTool({ img, s, set }: { img: ImageBitmap; s: EditState; set: (s: EditState) => void }) {
  const [ow, oh] = orientedSize(img.width, img.height, s.rotation);
  const rotate = (by: number) => set({ ...s, rotation: ((((s.rotation + by) % 360) + 360) % 360) as Rotation });
  const pct = (v: number) => `${Math.round(v)}%`;
  const outW = Math.max(1, Math.round(ow * s.scale));
  const outH = Math.max(1, Math.round(oh * s.scale));
  return (
    <>
      <Stage w={ow} h={oh}>
        {(dw, dh) => (
          <Paint dw={dw} dh={dh} draw={(c, W, H) => c.drawImage(renderEdit(img, s, Math.max(W, H)), 0, 0, W, H)} />
        )}
      </Stage>
      <aside className="panel">
        <Section title="Rotate & flip">
          <div className="row">
            <IconButton title="Rotate left" onClick={() => rotate(-90)}>
              <RotateCcw size={18} />
            </IconButton>
            <IconButton title="Rotate right" onClick={() => rotate(90)}>
              <RotateCw size={18} />
            </IconButton>
            <IconButton title="Flip horizontally" on={s.flipH} onClick={() => set({ ...s, flipH: !s.flipH })}>
              <FlipHorizontal2 size={18} />
            </IconButton>
            <IconButton title="Flip vertically" on={s.flipV} onClick={() => set({ ...s, flipV: !s.flipV })}>
              <FlipVertical2 size={18} />
            </IconButton>
          </div>
        </Section>
        <Section title="Adjust">
          <Slider label="Brightness" value={s.brightness} min={40} max={160} format={pct} onChange={(brightness) => set({ ...s, brightness })} />
          <Slider label="Contrast" value={s.contrast} min={40} max={160} format={pct} onChange={(contrast) => set({ ...s, contrast })} />
          <Slider label="Saturation" value={s.saturation} min={0} max={200} format={pct} onChange={(saturation) => set({ ...s, saturation })} />
        </Section>
        <Section title="Resize">
          <Slider label="Scale" value={Math.round(s.scale * 100)} min={5} max={100} format={pct} onChange={(v) => set({ ...s, scale: v / 100 })} />
          <div className="size-row">
            <label>
              W{" "}
              <input
                type="number"
                value={outW}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  if (v >= 1) set({ ...s, scale: Math.min(1, Math.max(0.01, v / ow)) });
                }}
              />
            </label>
            <span>×</span>
            <label>
              H{" "}
              <input
                type="number"
                value={outH}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  if (v >= 1) set({ ...s, scale: Math.min(1, Math.max(0.01, v / oh)) });
                }}
              />
            </label>
            <span className="unit">px</span>
          </div>
        </Section>
        <button className="link" onClick={() => set(EDIT_DEFAULTS)}>
          Reset
        </button>
      </aside>
    </>
  );
}
