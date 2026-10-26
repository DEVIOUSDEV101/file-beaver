import { useEffect, useRef, useState } from "react";
import { Trash2, Undo2 } from "lucide-react";
import { canvas, ctx2d, drawRedactions, normRect, Pt, Redaction, RedactMode } from "./render";
import { Chips, IconButton, norm, Paint, Section, Stage } from "./ui";

export type RedactState = { pages: Record<number, Redaction[]>; mode: RedactMode; past: Record<number, Redaction[]>[] };

export const REDACT_DEFAULTS: RedactState = { pages: {}, mode: "black", past: [] };

export function undoRedact(s: RedactState): RedactState {
  const prev = s.past[s.past.length - 1];
  return prev ? { ...s, pages: prev, past: s.past.slice(0, -1) } : s;
}

export default function RedactTool({ img, page, s, set, isPdf }: { img: ImageBitmap; page: number; s: RedactState; set: (s: RedactState) => void; isPdf: boolean }) {
  const [draft, setDraft] = useState<{ a: Pt; b: Pt } | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const area = useRef<HTMLDivElement>(null);
  const boxes = s.pages[page] ?? [];

  const change = (next: Redaction[]) => set({ ...s, pages: { ...s.pages, [page]: next }, past: [...s.past, s.pages] });

  const remove = () => {
    if (selected === null) return;
    change(boxes.filter((_, i) => i !== selected));
    setSelected(null);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.key === "Delete" || e.key === "Backspace") && selected !== null) remove();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const onDown = (e: React.PointerEvent) => {
    const p = norm(e, area.current!);
    const hit = boxes.findIndex((b) => p[0] >= b.x && p[0] <= b.x + b.w && p[1] >= b.y && p[1] <= b.y + b.h);
    if (hit >= 0) return setSelected(hit);
    setSelected(null);
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    setDraft({ a: p, b: p });
  };

  const onMove = (e: React.PointerEvent) => draft && setDraft({ ...draft, b: norm(e, area.current!) });

  const onUp = () => {
    if (!draft) return;
    const r = normRect(draft.a, draft.b);
    if (r.w > 0.004 && r.h > 0.004) change([...boxes, { ...r, mode: s.mode }]);
    setDraft(null);
  };

  // Sets the style for new boxes, and restyles the selected box if there is one.
  const setMode = (mode: RedactMode) => {
    if (selected === null) return set({ ...s, mode });
    const restyled = boxes.map((b, i) => (i === selected ? { ...b, mode } : b));
    set({ ...s, mode, pages: { ...s.pages, [page]: restyled }, past: [...s.past, s.pages] });
  };

  const shown = draft ? [...boxes, { ...normRect(draft.a, draft.b), mode: s.mode }] : boxes;

  return (
    <>
      <Stage w={img.width} h={img.height}>
        {(dw, dh) => (
          <div className="draw-area tool-rect" ref={area} onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp}>
            <Paint
              dw={dw}
              dh={dh}
              draw={(c, W, H) => {
                // Pixelate reads the unredacted pixels at display size.
                const src = canvas(W, H);
                ctx2d(src).drawImage(img, 0, 0, W, H);
                c.drawImage(src, 0, 0);
                drawRedactions(c, src, shown, W, H);
              }}
            />
            {selected !== null && boxes[selected] && (
              <div
                className="selection"
                style={{
                  left: `${boxes[selected].x * 100}%`,
                  top: `${boxes[selected].y * 100}%`,
                  width: `${boxes[selected].w * 100}%`,
                  height: `${boxes[selected].h * 100}%`,
                }}
              />
            )}
          </div>
        )}
      </Stage>
      <aside className="panel">
        <Section title="Style">
          <Chips
            value={s.mode}
            options={[
              { value: "black", label: "Black box" },
              { value: "pixelate", label: "Pixelate" },
            ]}
            onChange={setMode}
          />
          <p className="note">Drag over anything you want hidden. Click a box to select it.</p>
        </Section>
        <Section title="Boxes">
          <div className="row">
            <IconButton title="Undo (Ctrl+Z)" onClick={() => set(undoRedact(s))}>
              <Undo2 size={18} />
            </IconButton>
            <IconButton title="Delete selected (Del)" onClick={remove}>
              <Trash2 size={18} />
            </IconButton>
            <span className="note">{boxes.length} on this page</span>
          </div>
        </Section>
        {isPdf && (
          <p className="note warn">
            For real redaction every page is saved as an image, so the hidden text is gone for good. Text in the result
            won't be selectable.
          </p>
        )}
      </aside>
    </>
  );
}
