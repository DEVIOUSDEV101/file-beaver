import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Crop, EyeOff, Image, PenLine, Shrink, SlidersHorizontal, Tag, type LucideIcon } from "lucide-react";
import "./ring.css";

// Geometry in logical px. Must match trigger.rs (WIN_LOGICAL, RING_R_LOGICAL)
// and core/src/geometry.rs (HUB_RATIO), which do the hit-testing.
const WIN = 384;
const C = WIN / 2;
const R = 180;
const HUB = 0.34 * R;
const PETAL_IN = 0.4 * R;
const PETAL_OUT = 0.93 * R;
const CORNER = 12;
const GAP = 7;
const PLACEHOLDER_SEGMENTS = 8;

type Segment = { id: string; label: string; icon: string | null; enabled: boolean };
type Menu = { pages: [Segment[], Segment[]]; start_page: number; title: string };
type RingEvent =
  | { type: "show"; backdrop: string | null }
  | { type: "menu"; menu: Menu; page: number }
  | { type: "page"; page: number }
  | { type: "hover"; index: number }
  | { type: "thumb"; src: string }
  | { type: "chosen"; index: number }
  | { type: "hide" };

const ICONS: Record<string, LucideIcon> = {
  compress: Shrink,
  tag: Tag,
  sliders: SlidersHorizontal,
  pen: PenLine,
  image: Image,
  crop: Crop,
  "eye-off": EyeOff,
};

const point = (r: number, a: number) => `${(C + r * Math.sin(a)).toFixed(2)} ${(C - r * Math.cos(a)).toFixed(2)}`;

/**
 * Petal i of n, clockwise from the top. The path is inset by CORNER and drawn with a
 * round-joined stroke of 2*CORNER, which grows it back out with rounded corners.
 */
function petalPath(i: number, n: number) {
  const mid = (i * 2 * Math.PI) / n;
  const half = Math.PI / n;
  const inset = GAP / 2 + CORNER;
  const ri = PETAL_IN + CORNER;
  const ro = PETAL_OUT - CORNER;
  const ai = Math.asin(Math.min(1, inset / ri));
  const ao = Math.asin(Math.min(1, inset / ro));
  return [
    `M ${point(ri, mid - half + ai)}`,
    `L ${point(ro, mid - half + ao)}`,
    `A ${ro} ${ro} 0 0 1 ${point(ro, mid + half - ao)}`,
    `L ${point(ri, mid + half - ai)}`,
    `A ${ri} ${ri} 0 0 0 ${point(ri, mid - half + ai)}`,
    "Z",
  ].join(" ");
}

function PetalLabel({ seg, i, n }: { seg: Segment; i: number; n: number }) {
  const a = (i * 2 * Math.PI) / n;
  const r = (PETAL_IN + PETAL_OUT) / 2;
  const x = C + r * Math.sin(a);
  const y = C - r * Math.cos(a);
  const Icon = seg.icon ? ICONS[seg.icon] : undefined;
  if (!Icon) {
    return (
      <text className="label format" x={x} y={y} textAnchor="middle" dominantBaseline="central">
        {seg.label}
      </text>
    );
  }
  return (
    <>
      <Icon className="icon" x={x - 9} y={y - 19} width={18} height={18} strokeWidth={1.9} />
      <text className="label tool" x={x} y={y + 11} textAnchor="middle" dominantBaseline="central">
        {seg.label}
      </text>
    </>
  );
}

export default function Ring() {
  // Increments on every show so the entry animation replays; 0 = hidden.
  const [shown, setShown] = useState(0);
  const [backdrop, setBackdrop] = useState<string | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [page, setPage] = useState(0);
  const [hover, setHover] = useState(-1);
  const [thumb, setThumb] = useState<string | null>(null);
  const [chosen, setChosen] = useState(-1);

  useEffect(() => {
    document.documentElement.classList.add("ring-root");
    const un = listen<RingEvent>("ring", ({ payload: e }) => {
      switch (e.type) {
        case "show":
          setShown((s) => s + 1);
          setBackdrop(e.backdrop);
          setMenu(null);
          setPage(0);
          setHover(-1);
          setThumb(null);
          setChosen(-1);
          break;
        case "menu":
          setMenu(e.menu);
          setPage(e.page);
          break;
        case "page":
          setPage(e.page);
          setHover(-1);
          break;
        case "hover":
          setHover(e.index);
          break;
        case "thumb":
          setThumb(e.src);
          break;
        case "chosen":
          setChosen(e.index);
          break;
        case "hide":
          setShown(0);
          break;
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  if (!shown) return null;

  const segs: Segment[] = menu
    ? menu.pages[page]
    : Array.from({ length: PLACEHOLDER_SEGMENTS }, (_, i) => ({ id: `p${i}`, label: "", icon: null, enabled: false }));
  const n = segs.length;
  const hovered = hover >= 0 ? segs[hover] : undefined;
  const pill = hovered?.enabled ? hovered.label : menu?.title ?? "";

  return (
    <svg className="ring" key={shown} viewBox={`0 0 ${WIN} ${WIN}`}>
      <defs>
        <clipPath id="disc">
          <circle cx={C} cy={C} r={R} />
        </clipPath>
        <clipPath id="hub">
          <circle cx={C} cy={C} r={HUB - 1} />
        </clipPath>
        <linearGradient id="hot" gradientUnits="userSpaceOnUse" x1="0" y1={C - R} x2="0" y2={C + R}>
          <stop offset="0" stopColor="#f6763b" />
          <stop offset="1" stopColor="#e0451a" />
        </linearGradient>
      </defs>

      <g clipPath="url(#disc)">
        {backdrop && (
          <image
            className="backdrop"
            href={backdrop}
            x={C - R - 16}
            y={C - R - 16}
            width={2 * R + 32}
            height={2 * R + 32}
            preserveAspectRatio="none"
          />
        )}
        <circle className="tint" cx={C} cy={C} r={R} />
      </g>
      <circle className="rim" cx={C} cy={C} r={R - 0.75} />

      <g className={`petals ${menu ? "" : "loading"}`} key={`${page}-${menu ? "m" : "p"}`}>
        {segs.map((s, i) => (
          <g
            key={s.id}
            className={["petal", i === hover && s.enabled && "hot", !s.enabled && menu && "off", i === chosen && "chosen"]
              .filter(Boolean)
              .join(" ")}
          >
            <path d={petalPath(i, n)} />
            {s.label && <PetalLabel seg={s} i={i} n={n} />}
          </g>
        ))}
      </g>

      <circle className="hub" cx={C} cy={C} r={HUB} />
      {thumb && (
        <image
          className="thumb"
          href={thumb}
          x={C - HUB * 0.62}
          y={C - HUB * 0.78}
          width={HUB * 1.24}
          height={HUB * 1.24}
          clipPath="url(#hub)"
          preserveAspectRatio="xMidYMid meet"
        />
      )}
      {pill && (
        <foreignObject x={C - 80} y={C - 15} width={160} height={30}>
          <div className="pill-wrap">
            <div className={`pill ${hovered?.enabled ? "action" : ""}`}>{pill}</div>
          </div>
        </foreignObject>
      )}
    </svg>
  );
}
