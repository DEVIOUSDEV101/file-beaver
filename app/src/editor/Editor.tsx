import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ChevronLeft, ChevronRight } from "lucide-react";
import AddBgTool, { ADD_BG_DEFAULTS } from "./AddBgTool";
import AnnotateTool, { ANNOTATE_DEFAULTS, AnnotateState, redo, undo } from "./AnnotateTool";
import CropTool, { CROP_DEFAULTS } from "./CropTool";
import EditTool from "./EditTool";
import RedactTool, { REDACT_DEFAULTS, RedactState, undoRedact } from "./RedactTool";
import { AddBgState, CropState, EDIT_DEFAULTS, EditState, renderAddBg, renderAnnotate, renderCrop, renderEdit, renderRedact, toPngBytes } from "./render";
import "./editor.css";

type Tool = "crop" | "add_bg" | "edit" | "annotate" | "redact";
type Open = { tool: Tool; name: string; kind: "image" | "pdf"; pages: number };

const TITLES: Record<Tool, string> = {
  crop: "Crop",
  add_bg: "Add Background",
  edit: "Edit Image",
  annotate: "Annotate",
  redact: "Redact",
};

const SAVE_LABELS: Record<Tool, string> = {
  crop: "Save",
  add_bg: "Save with Background",
  edit: "Save",
  annotate: "Save",
  redact: "Save Redacted",
};

export default function Editor() {
  const [open, setOpen] = useState<Open | null>(null);
  const [bitmaps, setBitmaps] = useState<Record<number, ImageBitmap>>({});
  const [page, setPage] = useState(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [crop, setCrop] = useState<CropState>(CROP_DEFAULTS);
  const [addBg, setAddBg] = useState<AddBgState>(ADD_BG_DEFAULTS);
  const [edit, setEdit] = useState<EditState>(EDIT_DEFAULTS);
  const [annot, setAnnot] = useState<AnnotateState>(ANNOTATE_DEFAULTS);
  const [redact, setRedact] = useState<RedactState>(REDACT_DEFAULTS);
  // Ignores pages that finish loading after another file was opened.
  const generation = useRef(0);

  const load = async (p: number, gen: number) => {
    try {
      const buf = await invoke<ArrayBuffer>("editor_file", { page: p });
      const bmp = await createImageBitmap(new Blob([buf]), { imageOrientation: "from-image" });
      if (gen === generation.current) setBitmaps((b) => ({ ...b, [p]: bmp }));
    } catch (e) {
      if (gen === generation.current) setError(String(e));
    }
  };

  useEffect(() => {
    document.documentElement.classList.add("editor-root");
    const un = listen<Open>("editor:open", ({ payload }) => {
      const gen = ++generation.current;
      setOpen(payload);
      setBitmaps({});
      setPage(0);
      setError(null);
      setSaving(false);
      setCrop(CROP_DEFAULTS);
      setAddBg(ADD_BG_DEFAULTS);
      setEdit(EDIT_DEFAULTS);
      setAnnot(ANNOTATE_DEFAULTS);
      setRedact(REDACT_DEFAULTS);
      load(0, gen);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const goTo = (p: number) => {
    if (!open || p < 0 || p >= open.pages) return;
    setPage(p);
    if (!bitmaps[p]) load(p, generation.current);
  };

  const cancel = () => invoke("editor_close");

  const canSave =
    !!open &&
    !saving &&
    !!bitmaps[page] &&
    (open.tool !== "annotate" || Object.values(annot.pages).some((s) => s.length > 0)) &&
    (open.tool !== "redact" || Object.values(redact.pages).some((s) => s.length > 0));

  const save = async () => {
    if (!open || !canSave) return;
    setSaving(true);
    setError(null);
    try {
      if (open.kind === "image") {
        const img = bitmaps[0];
        const out = {
          crop: () => renderCrop(img, crop),
          add_bg: () => renderAddBg(img, addBg),
          edit: () => renderEdit(img, edit),
          annotate: () => renderAnnotate(img, annot.pages[0] ?? []),
          redact: () => renderRedact(img, redact.pages[0] ?? []),
        }[open.tool]();
        const bytes = await toPngBytes(out);
        await invoke("editor_save_image", bytes, { headers: { "x-tool": open.tool } });
      } else {
        await invoke("editor_save_pdf", {
          edit: {
            tool: open.tool,
            crop: open.tool === "crop" ? { rect: crop.rect, page: crop.allPages ? null : page } : null,
            shapes: annot.pages,
            redactions: redact.pages,
          },
        });
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  // Keyboard: Esc cancels, Ctrl+S saves, Ctrl+Z / Ctrl+Y undo and redo.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!open) return;
      const typing = (e.target as HTMLElement)?.tagName === "INPUT" || (e.target as HTMLElement)?.tagName === "TEXTAREA";
      if (e.key === "Escape" && !typing) cancel();
      else if (e.ctrlKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        save();
      } else if (e.ctrlKey && !typing && (e.key.toLowerCase() === "z" || e.key.toLowerCase() === "y")) {
        const redoing = e.key.toLowerCase() === "y" || e.shiftKey;
        if (open.tool === "annotate") setAnnot(redoing ? redo(annot) : undo(annot));
        if (open.tool === "redact" && !redoing) setRedact(undoRedact(redact));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (!open) return null;
  const img = bitmaps[page];
  const isPdf = open.kind === "pdf";

  return (
    <div className="editor">
      <header>
        <h1>{TITLES[open.tool]}</h1>
        <span className="file">{open.name}</span>
      </header>
      <main>
        {img ? (
          {
            crop: <CropTool img={img} s={crop} set={setCrop} isPdf={isPdf} />,
            add_bg: <AddBgTool img={img} s={addBg} set={setAddBg} />,
            edit: <EditTool img={img} s={edit} set={setEdit} />,
            annotate: <AnnotateTool img={img} page={page} s={annot} set={setAnnot} />,
            redact: <RedactTool img={img} page={page} s={redact} set={setRedact} isPdf={isPdf} />,
          }[open.tool]
        ) : (
          <div className="stage loading">{error ? "" : "Loading…"}</div>
        )}
      </main>
      <footer>
        {isPdf && open.pages > 1 ? (
          <div className="pager">
            <button className="icon-btn" title="Previous page" onClick={() => goTo(page - 1)} disabled={page === 0}>
              <ChevronLeft size={18} />
            </button>
            <span>
              Page {page + 1} of {open.pages}
            </span>
            <button className="icon-btn" title="Next page" onClick={() => goTo(page + 1)} disabled={page === open.pages - 1}>
              <ChevronRight size={18} />
            </button>
          </div>
        ) : (
          <span />
        )}
        {error && <span className="error">{error}</span>}
        <div className="actions">
          <button className="secondary" onClick={cancel}>
            Cancel
          </button>
          <button className="primary" onClick={save} disabled={!canSave}>
            {saving ? "Saving…" : SAVE_LABELS[open.tool]}
          </button>
        </div>
      </footer>
    </div>
  );
}
