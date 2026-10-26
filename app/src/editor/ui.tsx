import { ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Pt } from "./render";

/** Fits content of natural size w x h into the available space; children get the display size. */
export function Stage({ w, h, children }: { w: number; h: number; children: (dw: number, dh: number) => ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const [avail, setAvail] = useState<[number, number]>([0, 0]);
  useLayoutEffect(() => {
    const el = box.current!;
    const ro = new ResizeObserver(() => setAvail([el.clientWidth, el.clientHeight]));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const PAD = 28;
  const k = Math.min((avail[0] - 2 * PAD) / w, (avail[1] - 2 * PAD) / h, 3);
  const dw = Math.max(1, Math.floor(w * k));
  const dh = Math.max(1, Math.floor(h * k));
  return (
    <div className="stage" ref={box}>
      {avail[0] > 0 && k > 0 && (
        <div className="stage-content" style={{ width: dw, height: dh }}>
          {children(dw, dh)}
        </div>
      )}
    </div>
  );
}

/** A canvas sized dw x dh CSS px (sharp on high-DPI), redrawn by `draw` on every render. */
export function Paint({ dw, dh, draw, className }: { dw: number; dh: number; draw: (c: CanvasRenderingContext2D, W: number, H: number) => void; className?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const dpr = window.devicePixelRatio || 1;
  useEffect(() => {
    const cv = ref.current;
    if (!cv) return;
    cv.width = Math.round(dw * dpr);
    cv.height = Math.round(dh * dpr);
    const c = cv.getContext("2d")!;
    c.imageSmoothingQuality = "high";
    c.clearRect(0, 0, cv.width, cv.height);
    draw(c, cv.width, cv.height);
  });
  return <canvas ref={ref} className={className} style={{ width: dw, height: dh }} />;
}

/** Pointer position inside `el`, normalised to 0..1 (clamped). */
export function norm(e: { clientX: number; clientY: number }, el: Element): Pt {
  const r = el.getBoundingClientRect();
  return [Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), Math.min(1, Math.max(0, (e.clientY - r.top) / r.height))];
}

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="section">
      <h3>{title}</h3>
      {children}
    </section>
  );
}

export function Chips<T extends string>({ value, options, onChange }: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="chips">
      {options.map((o) => (
        <button key={o.value} className={o.value === value ? "chip on" : "chip"} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Slider({ label, value, min, max, step = 1, format, onChange }: { label: string; value: number; min: number; max: number; step?: number; format?: (v: number) => string; onChange: (v: number) => void }) {
  return (
    <label className="slider">
      <span className="slider-head">
        <span>{label}</span>
        <span className="slider-value">{format ? format(value) : value}</span>
      </span>
      <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => onChange(Number(e.target.value))} />
    </label>
  );
}

export function IconButton({ title, onClick, children, on }: { title: string; onClick: () => void; children: ReactNode; on?: boolean }) {
  return (
    <button className={on ? "icon-btn on" : "icon-btn"} title={title} aria-label={title} onClick={onClick}>
      {children}
    </button>
  );
}
