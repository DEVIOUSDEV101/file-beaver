import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import Editor from "./editor/Editor";
import Ring from "./Ring";
import Toast from "./Toast";

// Each window loads index.html with its own hash; the settings window has none.
const view = window.location.hash;

// Send script errors from every window to the app log (local only).
const report = (message: string) => invoke("log_js_error", { view: view || "#settings", message }).catch(() => {});
window.addEventListener("error", (e) => report(`${e.message} at ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report(`unhandled promise rejection: ${String(e.reason)}`));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {view === "#ring" ? <Ring /> : view === "#toast" ? <Toast /> : view === "#editor" ? <Editor /> : <App />}
  </React.StrictMode>,
);
