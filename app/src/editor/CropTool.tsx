import { useRef } from "react";
import { RotateCcw, RotateCw } from "lucide-react";
import { clamp, CropState, drawOriented, orientedSize, Pt, Rect, Rotation } from "./render";
import { Chips, IconButton, norm, Paint, Section, Stage } from "./ui";

const ASPECTS = [
  { value: "free", label: "Free" },
  { value: "original", label: "Original" },
  { value: "1:1", label: "1:1" },
  { value: "4:3", label: "4:3" },
  { value: "3:2", label: "3:2" },
  { value: "16:9", label: "16:9" },
  { value: "4:5", label: "4:5" },
  { value: "9:16", label: "9:16" },
];

export const CROP_DEFAULTS: CropState = { rect: { x: 0, y: 0, w: 1, h: 1 }, rotation: 0, aspect: "free", allPages: true };

/** Width/height of the crop in normalised units for a pixel aspect ratio. */
function ratioOf(aspect: string, ow: number, oh: number): number | null {
  if (aspect === "free") return null;
  const px = aspect === "original" ? ow / oh : (([a, b]) => a / b)(aspect.split(":").map(Number));
  return px * (oh / ow);
}

/** Largest rect with normalised ratio rn, centred on the current rect, inside the image. */
function fitRatio(r: Rect, rn: number): Rect {
  let w = 1;
  let h = w / rn;
  if (h > 1) {
    h = 1;
    w = rn;
  }
  const cx = r.x + r.w / 2;
  const cy = r.y + r.h / 2;
  return { x: clamp(cx - w / 2, 0, 1 - w), y: clamp(cy - h / 2, 0, 1 - h), w, h };
}

type Drag = { mode: "move" | "new" | string; start: Pt; rect: Rect };

const HANDLES = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
const MIN = 0.02;

export default function CropTool({ img, s, set, isPdf }: { img: ImageBitmap; s: CropState; set: (s: CropState) => void; isPdf: boolean }) {
  const [ow, oh] = orientedSize(img.width, img.height, s.rotation);
  const rn = ratioOf(s.aspect, ow, oh);
  const drag = useRef<Drag | null>(null);
  const area = useRef<HTMLDivElement>(null);

  const onDown = (e: React.PointerEvent, mode: string) => {
    e.stopPropagation();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drag.current = { mode, start: norm(e, area.current!), rect: s.rect };
  };

  const onMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const p = norm(e, area.current!);
    const [dx, dy] = [p[0] - d.start[0], p[1] - d.start[1]];
    const r = d.rect;
    let next: Rect;
    if (d.mode === "move") {
      next = { ...r, x: clamp(r.x + dx, 0, 1 - r.w), y: clamp(r.y + dy, 0, 1 - r.h) };
    } else {
      // Anchor = the side(s) not being dragged.
      let [x0, y0, x1, y1] = d.mode === "new" ? [d.start[0], d.start[1], p[0], p[1]] : [r.x, r.y, r.x + r.w, r.y + r.h];
      if (d.mode !== "new") {
        if (d.mode.includes("w")) x0 = clamp(r.x + dx, 0, x1 - MIN);
        if (d.mode.includes("e")) x1 = clamp(r.x + r.w + dx, x0 + MIN, 1);
        if (d.mode.includes("n")) y0 = clamp(r.y + dy, 0, y1 - MIN);
        if (d.mode.includes("s")) y1 = clamp(r.y + r.h + dy, y0 + MIN, 1);
      }
      let x = Math.min(x0, x1);
      let y = Math.min(y0, y1);
      let w = Math.max(MIN, Math.abs(x1 - x0));
      let h = Math.max(MIN, Math.abs(y1 - y0));
      if (rn) {
        const vertical = d.mode === "n" || d.mode === "s";
        if (vertical) w = h * rn;
        else h = w / rn;
        // Keep the anchored side fixed and stay inside the image.
        const fromLeft = d.mode === "new" ? p[0] < d.start[0] : d.mode.includes("w");
        const fromTop = d.mode === "new" ? p[1] < d.start[1] : d.mode.includes("n");
        const ax = fromLeft ? Math.max(x0, x1) : Math.min(x0, x1);
        const ay = fromTop ? Math.max(y0, y1) : Math.min(y0, y1);
        const maxW = fromLeft ? ax : 1 - ax;
        const maxH = fromTop ? ay : 1 - ay;
        const k = Math.min(1, maxW / w, maxH / h);
        w *= k;
        h *= k;
        x = fromLeft ? ax - w : ax;
        y = fromTop ? ay - h : ay;
        if (vertical) x = clamp(r.x + (r.w - w) / 2, 0, 1 - w);
        else if (d.mode === "e" || d.mode === "w") y = clamp(r.y + (r.h - h) / 2, 0, 1 - h);
      }
      next = { x, y, w: Math.min(w, 1 - x), h: Math.min(h, 1 - y) };
    }
    set({ ...s, rect: next });
  };

  const onUp = () => (drag.current = null);

  const setAspect = (aspect: string) => {
    const r = ratioOf(aspect, ow, oh);
    set({ ...s, aspect, rect: r ? fitRatio(s.rect, r) : s.rect });
  };

  const rotate = (by: number) => set({ ...s, rotation: (((s.rotation + by) % 360) + 360) % 360 as Rotation, rect: CROP_DEFAULTS.rect });

  // PDFs are measured in points; their pages are shown at 150 dpi.
  const unit = isPdf ? 72 / 150 : 1;
  const setPx = (which: "w" | "h", value: number) => {
    if (!value || value < 1) return;
    const px = value / unit;
    const r = s.rect;
    let w = which === "w" ? clamp(px / ow, MIN, 1) : r.w;
    let h = which === "h" ? clamp(px / oh, MIN, 1) : r.h;
    if (rn) {
      if (which === "w") h = w / rn;
      else w = h * rn;
      const k = Math.min(1, 1 / w, 1 / h);
      w *= k;
      h *= k;
    }
    set({ ...s, rect: { x: clamp(r.x, 0, 1 - w), y: clamp(r.y, 0, 1 - h), w, h } });
  };

  const r = s.rect;
  const box = { left: `${r.x * 100}%`, top: `${r.y * 100}%`, width: `${r.w * 100}%`, height: `${r.h * 100}%` };
  return (
    <>
      <Stage w={ow} h={oh}>
        {(dw, dh) => (
          <div className="crop-area" ref={area} onPointerDown={(e) => onDown(e, "new")} onPointerMove={onMove} onPointerUp={onUp}>
            <Paint dw={dw} dh={dh} draw={(c, W, H) => drawOriented(c, img, s.rotation, false, false, W, H)} />
            {/* Shading is clipped to the image; the frame and handles are not, so edge handles stay grabbable. */}
            <div className="shade-clip">
              <div className="shade" style={box} />
            </div>
            <div className="crop-box" style={box} onPointerDown={(e) => onDown(e, "move")}>
              <div className="thirds" />
              {HANDLES.map((h) => (
                <div key={h} className={`handle ${h}`} onPointerDown={(e) => onDown(e, h)} />
              ))}
            </div>
          </div>
        )}
      </Stage>
      <aside className="panel">
        <Section title="Aspect ratio">
          <Chips value={s.aspect} options={ASPECTS} onChange={setAspect} />
        </Section>
        <Section title="Size">
          <div className="size-row">
            <label>
              W <input type="number" value={Math.round(r.w * ow * unit)} onChange={(e) => setPx("w", Number(e.target.value))} />
            </label>
            <span>×</span>
            <label>
              H <input type="number" value={Math.round(r.h * oh * unit)} onChange={(e) => setPx("h", Number(e.target.value))} />
            </label>
            <span className="unit">{isPdf ? "pt" : "px"}</span>
          </div>
        </Section>
        {!isPdf && (
          <Section title="Rotate">
            <div className="row">
              <IconButton title="Rotate left" onClick={() => rotate(-90)}>
                <RotateCcw size={18} />
              </IconButton>
              <IconButton title="Rotate right" onClick={() => rotate(90)}>
                <RotateCw size={18} />
              </IconButton>
            </div>
          </Section>
        )}
        {isPdf && (
          <Section title="Pages">
            <label className="check">
              <input type="checkbox" checked={s.allPages} onChange={(e) => set({ ...s, allPages: e.target.checked })} />
              Crop every page the same way
            </label>
            <p className="note">PDF crops are lossless: text stays selectable.</p>
          </Section>
        )}
        <button className="link" onClick={() => set({ ...CROP_DEFAULTS, allPages: s.allPages })}>
          Reset
        </button>
      </aside>
    </>
  );
}
