// Drawing shared by the live preview and the full-resolution save, so what you see is
// exactly what gets saved. Geometry is normalised (0..1 of the displayed image, origin
// top-left). Annotation and redaction rules mirror core/src/pdf_edit.rs, which draws the
// same shapes into PDFs.

export type Pt = [number, number];
export type Rect = { x: number; y: number; w: number; h: number };
export type Rotation = 0 | 90 | 180 | 270;

export type ShapeKind = "arrow" | "rect" | "ellipse" | "pen" | "highlight" | "text" | "step";
export type Shape = {
  kind: ShapeKind;
  color: string;
  /** Stroke width as a fraction of the image diagonal. */
  width: number;
  points: Pt[];
  text?: string;
  /** Text: font size, step: badge radius; both as a fraction of image height. */
  size?: number;
  n?: number;
};

export type RedactMode = "black" | "pixelate";
export type Redaction = Rect & { mode: RedactMode };

export const HIGHLIGHT_ALPHA = 0.35;
// Arial matches Helvetica's metrics, which PDFs use.
const FONT = 'Arial, Helvetica, sans-serif';

export function canvas(w: number, h: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = Math.max(1, Math.round(w));
  c.height = Math.max(1, Math.round(h));
  return c;
}

export function ctx2d(c: HTMLCanvasElement): CanvasRenderingContext2D {
  const x = c.getContext("2d");
  if (!x) throw new Error("canvas unavailable");
  x.imageSmoothingQuality = "high";
  return x;
}

// ---------- orientation ----------

export function orientedSize(w: number, h: number, rot: Rotation): [number, number] {
  return rot % 180 ? [h, w] : [w, h];
}

/** Draws `img` rotated/flipped so it fills W x H. */
export function drawOriented(
  ctx: CanvasRenderingContext2D,
  img: CanvasImageSource,
  rot: Rotation,
  flipH: boolean,
  flipV: boolean,
  W: number,
  H: number,
) {
  ctx.save();
  ctx.translate(W / 2, H / 2);
  ctx.rotate((rot * Math.PI) / 180);
  ctx.scale(flipH ? -1 : 1, flipV ? -1 : 1);
  const [iw, ih] = rot % 180 ? [H, W] : [W, H];
  ctx.drawImage(img, -iw / 2, -ih / 2, iw, ih);
  ctx.restore();
}

// ---------- crop ----------

export type CropState = { rect: Rect; rotation: Rotation; aspect: string; allPages: boolean };

export function renderCrop(img: ImageBitmap, s: CropState): HTMLCanvasElement {
  const [ow, oh] = orientedSize(img.width, img.height, s.rotation);
  const full = canvas(ow, oh);
  drawOriented(ctx2d(full), img, s.rotation, false, false, ow, oh);
  const out = canvas(s.rect.w * ow, s.rect.h * oh);
  ctx2d(out).drawImage(full, -s.rect.x * ow, -s.rect.y * oh);
  return out;
}

// ---------- edit ----------

export type EditState = {
  rotation: Rotation;
  flipH: boolean;
  flipV: boolean;
  /** Percent, 100 = unchanged. */
  brightness: number;
  contrast: number;
  saturation: number;
  /** Output size as a fraction of the (rotated) original, 0.05..1. */
  scale: number;
};

export const EDIT_DEFAULTS: EditState = {
  rotation: 0,
  flipH: false,
  flipV: false,
  brightness: 100,
  contrast: 100,
  saturation: 100,
  scale: 1,
};

export function editFilter(s: EditState) {
  return `brightness(${s.brightness}%) contrast(${s.contrast}%) saturate(${s.saturation}%)`;
}

/** The edited image; `fit` limits the long side (for previews). */
export function renderEdit(img: ImageBitmap, s: EditState, fit?: number): HTMLCanvasElement {
  const [ow, oh] = orientedSize(img.width, img.height, s.rotation);
  let k = s.scale;
  if (fit) k = Math.min(1, fit / Math.max(ow, oh));
  const out = canvas(ow * k, oh * k);
  const c = ctx2d(out);
  c.filter = editFilter(s);
  drawOriented(c, img, s.rotation, s.flipH, s.flipV, out.width, out.height);
  return out;
}

// ---------- add background ----------

export type Background = { kind: "solid" | "gradient"; colors: string[]; angle: number };
export type BgAspect = "auto" | "1:1" | "4:3" | "16:9" | "4:5" | "9:16";
export type AddBgState = {
  bg: Background;
  /** Padding as a fraction of the image's long side. */
  padding: number;
  /** Corner radius as a fraction of the image's short side. */
  radius: number;
  /** 0..1 */
  shadow: number;
  aspect: BgAspect;
};

export function addBgLayout(iw: number, ih: number, s: AddBgState) {
  const pad = s.padding * Math.max(iw, ih);
  let W = iw + 2 * pad;
  let H = ih + 2 * pad;
  if (s.aspect !== "auto") {
    const [a, b] = s.aspect.split(":").map(Number);
    const r = a / b;
    if (W / H < r) W = H * r;
    else H = W / r;
  }
  return { W, H, x: (W - iw) / 2, y: (H - ih) / 2 };
}

function fillBackground(c: CanvasRenderingContext2D, bg: Background, W: number, H: number) {
  if (bg.kind === "solid" || bg.colors.length < 2) {
    c.fillStyle = bg.colors[0] ?? "#ffffff";
  } else {
    const a = (bg.angle * Math.PI) / 180;
    const len = (Math.abs(W * Math.sin(a)) + Math.abs(H * Math.cos(a))) / 2;
    const [dx, dy] = [Math.sin(a) * len, -Math.cos(a) * len];
    const g = c.createLinearGradient(W / 2 - dx, H / 2 - dy, W / 2 + dx, H / 2 + dy);
    bg.colors.forEach((col, i) => g.addColorStop(i / (bg.colors.length - 1), col));
    c.fillStyle = g;
  }
  c.fillRect(0, 0, W, H);
}

/** The image on its background; `scale` < 1 for previews. */
export function renderAddBg(img: ImageBitmap, s: AddBgState, scale = 1): HTMLCanvasElement {
  const { W, H, x, y } = addBgLayout(img.width, img.height, s);
  const out = canvas(W * scale, H * scale);
  const c = ctx2d(out);
  c.scale(scale, scale);
  fillBackground(c, s.bg, W, H);
  const r = s.radius * Math.min(img.width, img.height);
  const long = Math.max(img.width, img.height);
  if (s.shadow > 0) {
    c.save();
    c.shadowColor = `rgba(0,0,0,${0.5 * s.shadow})`;
    c.shadowBlur = 0.05 * long * s.shadow * scale;
    c.shadowOffsetY = 0.02 * long * s.shadow * scale;
    c.fillStyle = "#ffffff";
    c.beginPath();
    c.roundRect(x, y, img.width, img.height, r);
    c.fill();
    c.restore();
  }
  c.save();
  c.beginPath();
  c.roundRect(x, y, img.width, img.height, r);
  c.clip();
  c.drawImage(img, x, y);
  c.restore();
  return out;
}

// ---------- annotate ----------

/** Draws annotation shapes onto a W x H surface. */
export function drawShapes(c: CanvasRenderingContext2D, shapes: Shape[], W: number, H: number) {
  const diag = Math.hypot(W, H);
  const P = (p: Pt): Pt => [p[0] * W, p[1] * H];
  for (const s of shapes) {
    c.save();
    const lw = Math.max(0.1, s.width * diag);
    c.lineWidth = lw;
    c.lineCap = "round";
    c.lineJoin = "round";
    c.strokeStyle = s.color;
    c.fillStyle = s.color;
    const pts = s.points.map(P);
    switch (s.kind) {
      case "rect":
      case "ellipse": {
        if (pts.length < 2) break;
        const [[x0, y0], [x1, y1]] = pts;
        const [x, y, w, h] = [Math.min(x0, x1), Math.min(y0, y1), Math.abs(x1 - x0), Math.abs(y1 - y0)];
        c.beginPath();
        if (s.kind === "rect") c.rect(x, y, w, h);
        else c.ellipse(x + w / 2, y + h / 2, w / 2, h / 2, 0, 0, Math.PI * 2);
        c.stroke();
        break;
      }
      case "arrow": {
        if (pts.length < 2) break;
        const [[x0, y0], [x1, y1]] = pts;
        const len = Math.max(0.001, Math.hypot(x1 - x0, y1 - y0));
        const [dx, dy] = [(x1 - x0) / len, (y1 - y0) / len];
        const head = Math.min(lw * 4, len);
        const half = lw * 2.4;
        const [bx, by] = [x1 - dx * head, y1 - dy * head];
        c.beginPath();
        c.moveTo(x0, y0);
        c.lineTo(x1 - dx * head * 0.9, y1 - dy * head * 0.9);
        c.stroke();
        c.beginPath();
        c.moveTo(x1, y1);
        c.lineTo(bx - dy * half, by + dx * half);
        c.lineTo(bx + dy * half, by - dx * half);
        c.closePath();
        c.fill();
        break;
      }
      case "pen":
      case "highlight": {
        if (!pts.length) break;
        if (s.kind === "highlight") c.globalAlpha = HIGHLIGHT_ALPHA;
        c.beginPath();
        c.moveTo(pts[0][0], pts[0][1]);
        if (pts.length === 1) c.lineTo(pts[0][0], pts[0][1]);
        for (const [x, y] of pts.slice(1)) c.lineTo(x, y);
        c.stroke();
        break;
      }
      case "text": {
        if (!pts.length || !s.text) break;
        const fs = (s.size ?? 0.04) * H;
        c.font = `${fs}px ${FONT}`;
        c.textBaseline = "alphabetic";
        s.text.split("\n").forEach((line, i) => c.fillText(line, pts[0][0], pts[0][1] + fs * 0.85 + i * fs * 1.2));
        break;
      }
      case "step": {
        if (!pts.length) break;
        const rad = (s.size ?? 0.03) * H;
        const [cx, cy] = pts[0];
        c.beginPath();
        c.arc(cx, cy, rad, 0, Math.PI * 2);
        c.fill();
        const fs = rad * 1.1;
        c.fillStyle = "#ffffff";
        c.font = `${fs}px ${FONT}`;
        c.textAlign = "center";
        c.textBaseline = "alphabetic";
        c.fillText(String(s.n ?? 1), cx, cy + fs * 0.35);
        break;
      }
    }
    c.restore();
  }
}

/** Height of a text shape's box (normalised), for hit areas and editing. */
export function textLines(s: Shape) {
  return (s.text ?? "").split("\n").length;
}

export function renderAnnotate(img: ImageBitmap, shapes: Shape[]): HTMLCanvasElement {
  const out = canvas(img.width, img.height);
  const c = ctx2d(out);
  c.drawImage(img, 0, 0);
  drawShapes(c, shapes, img.width, img.height);
  return out;
}

// ---------- redact ----------

/** Pixelate block size; same rule as pdf_edit::pixel_block. */
export function pixelBlock(w: number, h: number) {
  return Math.max(8, Math.round(Math.max(w, h) / 80));
}

/** Burns boxes into `c` (W x H) whose current pixels come from `src`. */
export function drawRedactions(c: CanvasRenderingContext2D, src: CanvasImageSource, boxes: Redaction[], W: number, H: number) {
  const block = pixelBlock(W, H);
  for (const b of boxes) {
    const x = Math.floor(b.x * W);
    const y = Math.floor(b.y * H);
    const w = Math.ceil((b.x + b.w) * W) - x;
    const h = Math.ceil((b.y + b.h) * H) - y;
    if (w < 1 || h < 1) continue;
    if (b.mode === "black") {
      c.fillStyle = "#000";
      c.fillRect(x, y, w, h);
      continue;
    }
    // Pixelate: average blocks by shrinking then enlarging without smoothing.
    const small = canvas(Math.ceil(w / block), Math.ceil(h / block));
    const sc = ctx2d(small);
    sc.drawImage(src, x, y, small.width * block, small.height * block, 0, 0, small.width, small.height);
    c.save();
    c.imageSmoothingEnabled = false;
    c.beginPath();
    c.rect(x, y, w, h);
    c.clip();
    c.drawImage(small, 0, 0, small.width, small.height, x, y, small.width * block, small.height * block);
    c.restore();
  }
}

export function renderRedact(img: ImageBitmap, boxes: Redaction[]): HTMLCanvasElement {
  const out = canvas(img.width, img.height);
  const c = ctx2d(out);
  c.drawImage(img, 0, 0);
  const snapshot = canvas(img.width, img.height);
  ctx2d(snapshot).drawImage(img, 0, 0);
  drawRedactions(c, snapshot, boxes, img.width, img.height);
  return out;
}

// ---------- helpers ----------

export function toPngBytes(c: HTMLCanvasElement): Promise<Uint8Array> {
  return new Promise((resolve, reject) =>
    c.toBlob(async (b) => {
      if (!b) return reject(new Error("couldn't encode the image"));
      resolve(new Uint8Array(await b.arrayBuffer()));
    }, "image/png"),
  );
}

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function normRect(a: Pt, b: Pt): Rect {
  const x = clamp(Math.min(a[0], b[0]), 0, 1);
  const y = clamp(Math.min(a[1], b[1]), 0, 1);
  return { x, y, w: clamp(Math.max(a[0], b[0]), 0, 1) - x, h: clamp(Math.max(a[1], b[1]), 0, 1) - y };
}
