import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Settings = {
  enabled: boolean;
  trigger: "shift" | "ctrl_shift" | "alt";
  output: "beside" | "folder";
  output_folder: string;
  launch_at_startup: boolean;
};

const TRIGGERS: { value: Settings["trigger"]; label: string }[] = [
  { value: "shift", label: "Shift" },
  { value: "ctrl_shift", label: "Ctrl + Shift" },
  { value: "alt", label: "Alt" },
];

export default function App() {
  const [s, setS] = useState<Settings | null>(null);
  const [status, setStatus] = useState("");
  const [version, setVersion] = useState("");
  const [updateMsg, setUpdateMsg] = useState("");
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    document.documentElement.classList.add("settings-root");
    // The window can load before the app has finished starting; retry until it's ready
    // (otherwise the settings window would stay blank).
    let alive = true;
    const load = (tries: number) =>
      invoke<Settings>("get_settings")
        .then((v) => alive && setS(v))
        .catch(() => alive && tries > 0 && setTimeout(() => load(tries - 1), 200));
    load(50);
    getVersion().then(setVersion).catch(() => {});
    const unSet = listen<Settings>("settings-changed", (e) => setS(e.payload));
    return () => {
      alive = false;
      unSet.then((f) => f());
    };
  }, []);

  if (!s) return null;

  const update = async (patch: Partial<Settings>) => {
    const next = { ...s, ...patch };
    setS(next);
    try {
      await invoke("save_settings", { settings: next });
      setStatus("Saved");
    } catch (e) {
      setStatus(`Could not save: ${e}`);
    }
    setTimeout(() => setStatus(""), 1500);
  };

  const keyName = TRIGGERS.find((t) => t.value === s.trigger)?.label;

  return (
    <main className="settings">
      <header className="brand">
        <img src="/beaver.svg" alt="" width="40" height="40" />
        <h1>File Beaver</h1>
      </header>
      <p className="hint">
        Drag any file and tap <kbd>{keyName}</kbd> to open the ring. Tap again to switch between Convert and Tools.
      </p>

      <label className="row">
        <span>Enabled</span>
        <input type="checkbox" checked={s.enabled} onChange={(e) => update({ enabled: e.target.checked })} />
      </label>

      <label className="row">
        <span>Trigger key</span>
        <select value={s.trigger} onChange={(e) => update({ trigger: e.target.value as Settings["trigger"] })}>
          {TRIGGERS.map((t) => (
            <option key={t.value} value={t.value}>
              {t.label}
            </option>
          ))}
        </select>
      </label>

      <label className="row">
        <span>Save results</span>
        <select value={s.output} onChange={(e) => update({ output: e.target.value as Settings["output"] })}>
          <option value="beside">Next to the original</option>
          <option value="folder">In a fixed folder</option>
        </select>
      </label>

      {s.output === "folder" && (
        <label className="row">
          <span>Folder</span>
          <input
            type="text"
            placeholder="C:\Users\you\Pictures\File Beaver"
            value={s.output_folder}
            onChange={(e) => setS({ ...s, output_folder: e.target.value })}
            onBlur={(e) => update({ output_folder: e.target.value })}
          />
        </label>
      )}

      <label className="row">
        <span>Launch at startup</span>
        <input
          type="checkbox"
          checked={s.launch_at_startup}
          onChange={(e) => update({ launch_at_startup: e.target.checked })}
        />
      </label>

      <p className="status">{status}</p>

      <section className="about">
        <h2>About</h2>
        <div className="row">
          <span>File Beaver {version}</span>
          <button
            className="check"
            disabled={checking}
            onClick={async () => {
              setChecking(true);
              setUpdateMsg("");
              try {
                setUpdateMsg(await invoke<string>("check_for_updates"));
              } catch (e) {
                setUpdateMsg(`Couldn't check: ${e}`);
              } finally {
                setChecking(false);
              }
            }}
          >
            {checking ? "Checking…" : "Check for updates"}
          </button>
        </div>
        {updateMsg && <p className="hint">{updateMsg}</p>}
      </section>
    </main>
  );
}
