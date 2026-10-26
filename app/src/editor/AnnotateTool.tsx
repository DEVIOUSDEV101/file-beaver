import { useRef, useState } from "react";
import { ArrowUpRight, Circle, Hash, Highlighter, Pencil, Redo2, Square, Type, Undo2 } from "lucide-react";
import { drawShapes, Pt, Shape, ShapeKind } from "./render";
import { IconButton, norm, Paint, Section, Stage } from "./ui";

const TOOLS: { kind: ShapeKind; label: string; icon: typeof Square }[] = [
  { kind: "arrow", label: "Arrow", icon: ArrowUpRight },
  { kind: "rect", label: "Rectangle", icon: Square },
  { kind: "ellipse", label: "Ellipse", icon: Circle },
  { kind: "pen", label: "Pen", icon: Pencil },
  { kind: "highlight", label: "Highlighter", icon: Highlighter },
  { kind: "text", label: "Text", icon: Type },
  { kind: "step", label: "Numbered step", icon: Hash },
];

const COLORS = ["#e5383b", "#f77f00", "#fcbf49", "#2a9d8f", "#1d70b8", "#111111", "#ffffff"];

// Per size S/M/L: stroke (fraction of diagonal), highlighter stroke, text size, step radius (fractions of height).
const SIZES = {
  S: { stroke: 0.003, highlight: 0.018, text: 0.03, step: 0.022 },
  M: { stroke: 0.006, highlight: 0.03, text: 0.045, step: 0.032 },
  L: { stroke: 0.011, highlight: 0.045, text: 0.065, step: 0.045 },
};
type Size = keyof typeof SIZES;

export type AnnotateState = {
  /** Shapes per page (0 for images). */
  pages: Record<number, Shape[]>;
  past: Record<number, Shape[]>[];
  future: Record<number, Shape[]>[];
};

export const ANNOTATE_DEFAULTS: AnnotateState = { pages: {}, past: [], future: [] };

export function undo(s: AnnotateState): AnnotateState {
  const prev = s.past[s.past.length - 1];
  return prev ? { pages: prev, past: s.past.slice(0, -1), future: [s.pages, ...s.future] } : s;
}

export function redo(s: AnnotateState): AnnotateState {
  const next = s.future[0];
  return next ? { pages: next, past: [...s.past, s.pages], future: s.future.slice(1) } : s;
}

export default function AnnotateTool({ img, page, s, set }: { img: ImageBitmap; page: number; s: AnnotateState; set: (s: AnnotateState) => void }) {
  const [kind, setKind] = useState<ShapeKind>("arrow");
  const [color, setColor] = useState(COLORS[0]);
  const [size, setSize] = useState<Size>("M");
  const [draft, setDraft] = useState<Shape | null>(null);
  const [typing, setTyping] = useState<{ at: Pt; text: string } | null>(null);
  const area = useRef<HTMLDivElement>(null);
  const shapes = s.pages[page] ?? [];

  const commit = (shape: Shape) => set({ pages: { ...s.pages, [page]: [...shapes, shape] }, past: [...s.past, s.pages], future: [] });

  const base = (points: Pt[]): Shape => {
    const z = SIZES[size];
    const hl = kind === "highlight";
    return { kind, color: hl && color === COLORS[0] ? "#fcbf49" : color, width: hl ? z.highlight : z.stroke, points, size: kind === "step" ? z.step : z.text };
  };

  const finishTyping = () => {
    if (typing && typing.text.trim()) commit({ ...base([typing.at]), kind: "text", text: typing.text });
    setTyping(null);
  };

  const onDown = (e: React.PointerEvent) => {
    if (typing) return finishTyping();
    const p = norm(e, area.current!);
    if (kind === "text") return setTyping({ at: p, text: "" });
    if (kind === "step") return commit({ ...base([p]), n: shapes.filter((x) => x.kind === "step").length + 1 });
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    setDraft(base([p, p]));
  };

  const onMove = (e: React.PointerEvent) => {
    if (!draft) return;
    const p = norm(e, area.current!);
    setDraft(draft.kind === "pen" || draft.kind === "highlight" ? { ...draft, points: [...draft.points, p] } : { ...draft, points: [draft.points[0], p] });
  };

  const onUp = () => {
    if (!draft) return;
    const [a, b] = [draft.points[0], draft.points[draft.points.length - 1]];
    const moved = Math.hypot(b[0] - a[0], b[1] - a[1]) > 0.004 || draft.points.length > 3;
    if (moved) commit(draft);
    setDraft(null);
  };

  return (
    <>
      <Stage w={img.width} h={img.height}>
        {(dw, dh) => (
          <div className={`draw-area tool-${kind}`} ref={area} onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp}>
            <Paint
              dw={dw}
              dh={dh}
              draw={(c, W, H) => {
                c.drawImage(img, 0, 0, W, H);
                drawShapes(c, draft ? [...shapes, draft] : shapes, W, H);
              }}
            />
            {typing && (
              <textarea
                className="text-entry"
                autoFocus
                value={typing.text}
                placeholder="Type, then Enter"
                style={{ left: typing.at[0] * dw, top: typing.at[1] * dh, color, fontSize: SIZES[size].text * dh, lineHeight: 1.2 }}
                onPointerDown={(e) => e.stopPropagation()}
                onChange={(e) => setTyping({ ...typing, text: e.target.value })}
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    finishTyping();
                  } else if (e.key === "Escape") setTyping(null);
                }}
                onBlur={finishTyping}
              />
            )}
          </div>
        )}
      </Stage>
      <aside className="panel">
        <Section title="Tool">
          <div className="row wrap">
            {TOOLS.map((t) => (
              <IconButton key={t.kind} title={t.label} on={kind === t.kind} onClick={() => setKind(t.kind)}>
                <t.icon size={18} />
              </IconButton>
            ))}
          </div>
        </Section>
        <Section title="Colour">
          <div className="row wrap">
            {COLORS.map((c) => (
              <button key={c} className={c === color ? "dot on" : "dot"} style={{ background: c }} title={c} onClick={() => setColor(c)} />
            ))}
          </div>
        </Section>
        <Section title="Size">
          <div className="chips">
            {(Object.keys(SIZES) as Size[]).map((z) => (
              <button key={z} className={z === size ? "chip on" : "chip"} onClick={() => setSize(z)}>
                {z}
              </button>
            ))}
          </div>
        </Section>
        <Section title="History">
          <div className="row">
            <IconButton title="Undo (Ctrl+Z)" onClick={() => set(undo(s))}>
              <Undo2 size={18} />
            </IconButton>
            <IconButton title="Redo (Ctrl+Y)" onClick={() => set(redo(s))}>
              <Redo2 size={18} />
            </IconButton>
            <span className="note">{shapes.length} on this page</span>
          </div>
        </Section>
      </aside>
    </>
  );
}
