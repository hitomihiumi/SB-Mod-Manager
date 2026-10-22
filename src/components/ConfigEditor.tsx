import clsx from "clsx";
import { FileCog, RotateCcw, Save } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { parseIni, setIniValue } from "../lib/ini";
import { formatBytes } from "../lib/format";
import { ipc, type ConfigFile, type ConfigScope } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { askConfirm, isAsking } from "./Ask";
import { Badge, Button, Dialog, Section } from "./ui";

/**
 * Edit one settings file. INI files open as a form — a field per setting,
 * grouped by section, with the comments above each as its help — and every
 * file can be edited as text. Saving writes the file in its own encoding;
 * reset goes back to the copy it was installed (or first found) as.
 */
export function ConfigEditor({
  scope,
  path,
  title,
  onClose,
}: {
  scope: ConfigScope;
  path: string;
  title: string;
  onClose: (saved: boolean) => void;
}) {
  const toast = useApp((s) => s.toast);
  const [loaded, setLoaded] = useState<string | null>(null);
  const [original, setOriginal] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const isIni = /\.ini$/i.test(path);
  const sections = useMemo(() => (isIni ? parseIni(text) : []), [isIni, text]);
  const [mode, setMode] = useState<"form" | "text">("form");
  const form = isIni && mode === "form" && sections.length > 0;

  useEffect(() => {
    ipc
      .readConfig(scope, path)
      .then((config) => {
        setLoaded(config.text);
        setText(config.text);
        setOriginal(config.original);
      })
      .catch((error) => {
        toast("error", String(error));
        onClose(false);
      });
    // The file is read once, when the editor opens.
  }, []);

  const dirty = loaded !== null && text !== loaded;

  // Ctrl+S saves, as in any editor.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        if (dirty && !saving && !isAsking()) void save();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  async function save() {
    setSaving(true);
    try {
      await ipc.writeConfig(scope, path, text);
      toast("success", `Saved ${path.split("/").pop()}.`);
      onClose(true);
    } catch (error) {
      toast("error", String(error));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog
      open
      width="760px"
      title={title}
      onClose={async () => {
        if (isAsking()) return;
        const leave =
          !dirty ||
          (await askConfirm({
            title: "Unsaved changes",
            message: "Close without saving your changes?",
            confirmLabel: "Discard",
            danger: true,
          }));
        if (leave) onClose(false);
      }}
      footer={
        <>
          {original !== null && original !== text && (
            <Button
              variant="ghost"
              title={
                scope.kind === "game"
                  ? "Back to the file as it was before the manager first saved it"
                  : "Back to the file as the mod was installed with it"
              }
              onClick={() => setText(original)}
            >
              <RotateCcw size={13} /> Reset to original
            </Button>
          )}
          <span className="flex-1 truncate font-mono text-[11px] text-ink-muted" title={path}>
            {path}
          </span>
          <Button variant="ghost" onClick={() => onClose(false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!dirty || saving} onClick={() => void save()}>
            <Save size={13} /> Save
          </Button>
        </>
      }
    >
      {isIni && sections.length > 0 && (
        <div className="mb-3 inline-flex rounded-md border border-line p-0.5 text-[12px]">
          {(["form", "text"] as const).map((each) => (
            <button
              key={each}
              onClick={() => setMode(each)}
              className={clsx(
                "rounded px-2.5 py-1 transition-colors",
                mode === each ? "bg-hover text-ink" : "text-ink-muted hover:text-ink-soft",
              )}
            >
              {each === "form" ? "Settings" : "Text"}
            </button>
          ))}
        </div>
      )}
      {scope.kind === "game" && (
        <p className="mb-3 text-[11px] leading-relaxed text-warn">
          Close the game first: it writes its settings back when it exits, over anything saved
          here while it runs.
        </p>
      )}

      {loaded === null ? (
        <div className="h-96 animate-pulse rounded-md bg-hover" />
      ) : form ? (
        <div className="max-h-[60vh] space-y-4 overflow-y-auto pr-1">
          {sections.map((section) => (
            <fieldset key={`${section.name}:${section.settings[0]?.line}`}>
              {section.name && (
                <legend className="mb-1.5 font-mono text-[12px] font-medium text-accent-soft">
                  [{section.name}]
                </legend>
              )}
              <div className="space-y-1.5">
                {section.settings.map((setting) => (
                  <label
                    key={setting.line}
                    className="grid grid-cols-[minmax(0,220px)_1fr] items-start gap-3"
                  >
                    <span className="pt-1.5 font-mono text-[12px] break-all text-ink-soft">
                      {setting.key}
                    </span>
                    <span>
                      <SettingInput
                        value={setting.value}
                        onChange={(value) => setText((current) => setIniValue(current, setting.line, value))}
                      />
                      {setting.help && (
                        <span className="mt-0.5 block text-[11px] leading-relaxed whitespace-pre-line text-ink-muted">
                          {setting.help}
                        </span>
                      )}
                    </span>
                  </label>
                ))}
              </div>
            </fieldset>
          ))}
        </div>
      ) : (
        <textarea
          value={text}
          onChange={(event) => setText(event.target.value)}
          spellCheck={false}
          aria-label={path}
          className="h-[60vh] w-full resize-none rounded-md border border-line bg-base p-3 font-mono text-[12px] leading-relaxed text-ink focus:border-accent-soft focus:outline-none"
        />
      )}
    </Dialog>
  );
}

/** A checkbox for true/false settings, a text field for anything else. */
function SettingInput({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const lower = value.toLowerCase();
  if (lower === "true" || lower === "false") {
    const on = lower === "true";
    // Written back in the file's own capitalisation.
    const word = (next: boolean) => {
      const plain = next ? "true" : "false";
      if (value === value.toUpperCase()) return plain.toUpperCase();
      if (value[0] === value[0]?.toUpperCase()) return plain[0]!.toUpperCase() + plain.slice(1);
      return plain;
    };
    return (
      <span className="flex h-8 items-center">
        <input
          type="checkbox"
          checked={on}
          onChange={(event) => onChange(word(event.target.checked))}
          className="size-4 accent-[var(--color-accent)]"
        />
      </span>
    );
  }
  return (
    <input
      value={value}
      onChange={(event) => onChange(event.target.value)}
      spellCheck={false}
      className="h-8 w-full rounded-md border border-line bg-base px-2.5 font-mono text-[12px] focus:border-accent-soft focus:outline-none"
    />
  );
}

/**
 * The game's own INI files — Engine.ini, GameUserSettings.ini and the
 * rest Unreal keeps in %LOCALAPPDATA%\\SB\\Saved\\Config — for the tweaks
 * mod pages tell people to make by hand.
 */
export function GameConfigSection() {
  const [files, setFiles] = useState<ConfigFile[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const load = () => {
    ipc
      .configFiles({ kind: "game" })
      .then(setFiles)
      .catch(() => setFiles([]));
  };
  useEffect(load, []);

  return (
    <Section
      title="Game settings files"
      hint="Stellar Blade's own INI files, for the tweaks mod pages ask you to make by hand. The version found before the first save here is kept, to reset to."
    >
      {files === null ? null : files.length === 0 ? (
        <p className="text-[12px] text-ink-muted">
          None yet — the game writes them the first time it runs.
        </p>
      ) : (
        <ul className="space-y-1">
          {files.map((file) => (
            <li key={file.path}>
              <button
                onClick={() => setEditing(file.path)}
                className="flex w-full items-center gap-2 rounded-md border border-line px-2.5 py-1.5 text-left hover:border-line-strong"
              >
                <FileCog size={13} className="shrink-0 text-ink-muted" />
                <span className="min-w-0 flex-1 truncate font-mono text-[12px]">{file.path}</span>
                {file.edited && <Badge tone="warn">Edited</Badge>}
                <span className="text-[11px] text-ink-muted">{formatBytes(file.bytes)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {editing && (
        <ConfigEditor
          scope={{ kind: "game" }}
          path={editing}
          title={editing.split("/").pop() ?? editing}
          onClose={(saved) => {
            setEditing(null);
            if (saved) load();
          }}
        />
      )}
    </Section>
  );
}
