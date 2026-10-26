import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AlertTriangle, Check, Info, Loader2, X } from "lucide-react";
import "./toast.css";

type JobEvent =
  | { type: "start"; id: number; title: string; detail: string; queued: number }
  | { type: "progress"; id: number; fraction: number; detail: string }
  | {
      type: "done";
      id: number;
      title: string;
      summary: string;
      outputs: string[];
      errors: string[];
      cancelled: boolean;
      sticky: boolean;
      action: { label: string; command: string } | null;
    }
  | { type: "info"; id: number; title: string; summary: string };

type Card = {
  id: number;
  state: "running" | "done" | "error" | "info";
  title: string;
  detail: string;
  fraction: number;
  queued: number;
  outputs: string[];
  errors: string[];
  action?: { label: string; command: string } | null;
};

/** How long a finished card stays up (paused while hovered). Errors stay until closed. */
const AUTO_HIDE_MS = 3500;

export default function Toast() {
  const [card, setCard] = useState<Card | null>(null);
  const hovered = useRef(false);
  const hideTimer = useRef<number | undefined>(undefined);

  const hide = () => {
    window.clearTimeout(hideTimer.current);
    setCard(null);
    invoke("hide_toast");
  };

  const scheduleHide = () => {
    window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => {
      if (!hovered.current) hide();
      else scheduleHide();
    }, AUTO_HIDE_MS);
  };

  useEffect(() => {
    document.documentElement.classList.add("toast-root");
    const un = listen<JobEvent>("job", ({ payload: e }) => {
      switch (e.type) {
        case "start":
          window.clearTimeout(hideTimer.current);
          setCard({ id: e.id, state: "running", title: e.title, detail: e.detail, fraction: 0, queued: e.queued, outputs: [], errors: [] });
          break;
        case "progress":
          setCard((c) => (c && c.id === e.id ? { ...c, fraction: e.fraction, detail: e.detail } : c));
          break;
        case "done": {
          const failed = e.outputs.length === 0 && e.errors.length > 0;
          setCard((c) => ({
            id: e.id,
            // Sticky cards are notices (e.g. "closed unexpectedly"), not successes.
            state: failed ? "error" : e.sticky ? "info" : "done",
            title: e.title,
            detail: e.summary,
            fraction: 1,
            queued: c?.queued ?? 0,
            outputs: e.outputs,
            errors: e.errors,
            action: e.action,
          }));
          if (!failed && !e.sticky) scheduleHide();
          break;
        }
        case "info":
          setCard({ id: e.id, state: "info", title: e.title, detail: e.summary, fraction: 0, queued: 0, outputs: [], errors: [] });
          scheduleHide();
          break;
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  if (!card) return null;

  const close = () => {
    if (card.state === "running") invoke("cancel_job");
    hide();
  };

  const Icon = { running: Loader2, done: Check, error: AlertTriangle, info: Info }[card.state];

  return (
    <div
      className={`card ${card.state}`}
      key={card.id}
      onMouseEnter={() => (hovered.current = true)}
      onMouseLeave={() => (hovered.current = false)}
    >
      <div className="head">
        <span className="badge">
          <Icon size={15} strokeWidth={2.4} className={card.state === "running" ? "spin" : undefined} />
        </span>
        <span className="title">{card.title}</span>
        <button className="close" onClick={close} title={card.state === "running" ? "Cancel" : "Close"}>
          <X size={14} strokeWidth={2.4} />
        </button>
      </div>
      <div className="detail">
        {card.detail}
        {card.state === "running" && card.queued > 0 && <span className="queued"> · {card.queued} more queued</span>}
      </div>
      {card.state === "running" && (
        <div className="bar">
          <div style={{ width: `${Math.max(4, card.fraction * 100)}%` }} />
        </div>
      )}
      {card.errors.length > 0 && (
        <div className="errors" title={card.errors.join("\n")}>
          {card.errors[0]}
          {card.errors.length > 1 && ` (+${card.errors.length - 1} more)`}
        </div>
      )}
      {card.action && (
        <button
          className="reveal"
          onClick={() => {
            const a = card.action!;
            setCard({ ...card, action: null, detail: "Working…" });
            invoke(a.command).catch((err) => setCard({ ...card, action: null, state: "error", errors: [String(err)] }));
          }}
        >
          {card.action.label}
        </button>
      )}
      {(card.state === "done" || card.state === "info") && card.outputs.length > 0 && (
        <button className="reveal" onClick={() => invoke("reveal_in_folder", { path: card.outputs[0] })}>
          Show in folder
        </button>
      )}
    </div>
  );
}
