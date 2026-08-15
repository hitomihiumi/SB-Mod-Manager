import { useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import { MOD_TYPE_META, SELECTABLE_TYPES, type ModTypeId } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Dialog } from "./ui";

/**
 * Shown only when detection could not decide on its own. Everything else
 * installs without interrupting the user.
 */
export function InstallDialog() {
  const queue = useApp((s) => s.queue);
  const confirmInstall = useApp((s) => s.confirmInstall);
  const cancelInstall = useApp((s) => s.cancelInstall);
  const skipRemaining = useApp((s) => s.skipRemaining);

  const staged = queue[0] ?? null;

  const [name, setName] = useState("");
  const [type, setType] = useState<ModTypeId>("genericPak");

  useEffect(() => {
    if (staged) {
      setName(staged.suggestedName);
      const detected = staged.components[0]?.modType;
      setType(detected && detected !== "unknown" ? detected : "genericPak");
    }
  }, [staged]);

  if (!staged) return null;

  const fileCount = staged.components.reduce((sum, c) => sum + c.files.length, 0);
  const notes = staged.components.flatMap((c) => c.notes);
  const targetDir = MOD_TYPE_META[type].hint;

  return (
    <Dialog
      open
      title={
        queue.length > 1
          ? `Where do these files belong? (1 of ${queue.length})`
          : "Where do these files belong?"
      }
      onClose={() => void cancelInstall()}
      footer={
        <>
          {queue.length > 1 && (
            <Button variant="ghost" onClick={() => void skipRemaining()}>
              Skip all {queue.length}
            </Button>
          )}
          <div className="flex-1" />
          <Button variant="ghost" onClick={() => void cancelInstall()}>
            Skip
          </Button>
          <Button
            variant="primary"
            disabled={!name.trim()}
            onClick={() => void confirmInstall(name.trim(), type)}
          >
            Install
          </Button>
        </>
      }
    >
      <p className="mb-4 text-[12px] leading-relaxed text-ink-muted">
        This archive has no marker the detector recognises — no pak files, no{" "}
        <code className="font-mono text-ink-soft">LogicMods</code> folder, no UE4SS script. Pick
        the type and it will be remembered for archives shaped like this one.
      </p>

      <label className="mb-1.5 block text-[12px] font-medium">Name</label>
      <input
        value={name}
        onChange={(event) => setName(event.target.value)}
        className="mb-4 h-8 w-full rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
      />

      <label className="mb-1.5 block text-[12px] font-medium">Type</label>
      <div className="mb-2 grid grid-cols-2 gap-1.5">
        {SELECTABLE_TYPES.map((candidate) => (
          <button
            key={candidate}
            onClick={() => setType(candidate)}
            className={
              "rounded-md border px-2.5 py-2 text-left transition-colors " +
              (type === candidate
                ? "border-accent bg-accent/10"
                : "border-line hover:border-line-strong")
            }
          >
            <div className="text-[12px] font-medium">{MOD_TYPE_META[candidate].label}</div>
            <div className="truncate font-mono text-[10px] text-ink-muted">
              {MOD_TYPE_META[candidate].hint}
            </div>
          </button>
        ))}
      </div>

      <div className="mt-4 rounded-md border border-line bg-base p-3">
        <div className="mb-2 flex items-center gap-2">
          <Badge tone="accent">{MOD_TYPE_META[type].label}</Badge>
          <span className="text-[11px] text-ink-muted">
            {fileCount} file{fileCount === 1 ? "" : "s"} · {formatBytes(staged.sizeBytes)}
          </span>
        </div>
        <p className="mb-2 font-mono text-[11px] text-ink-soft">→ {targetDir}</p>
        {notes.length > 0 && (
          <ul className="mb-2 space-y-0.5">
            {notes.map((note) => (
              <li key={note} className="text-[11px] leading-relaxed text-ink-muted">
                {note}
              </li>
            ))}
          </ul>
        )}
        <ul className="max-h-32 space-y-0.5 overflow-y-auto">
          {staged.components
            .flatMap((c) => c.files)
            .slice(0, 40)
            .map((file) => (
              <li key={file.source} className="truncate font-mono text-[11px] text-ink-muted">
                {file.source}
              </li>
            ))}
        </ul>
      </div>
    </Dialog>
  );
}
