import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FolderOpen, PackagePlus, PenLine, Trash2 } from "lucide-react";
import { useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Dialog } from "./ui";
import { askConfirm } from "./Ask";

/**
 * The overwrite folder, shown under the mod list as Mod Organizer 2 shows it:
 * what the game created while running under the virtual file system, with
 * the means to keep it as a mod of its own or throw it away.
 */
export function OverwriteBar() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");
  const [working, setWorking] = useState(false);

  const overwrite = snapshot?.overwrite;
  if (!overwrite || overwrite.files === 0) return null;

  async function run(action: () => Promise<string>) {
    setWorking(true);
    try {
      toast("success", await action());
      await refresh();
      setNaming(false);
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
    }
  }

  return (
    <div className="flex h-11 shrink-0 items-center gap-2 border-t border-line bg-surface px-3">
      <PenLine size={14} className="text-accent-soft" />
      <span className="text-[13px]">Overwrite</span>
      <span className="text-[11px] text-ink-muted">
        {overwrite.files} file{overwrite.files === 1 ? "" : "s"} · {formatBytes(overwrite.bytes)} the
        game created
      </span>
      <span className="flex-1" />
      <Button size="sm" title="Show the overwrite folder" onClick={() => void revealItemInDir(overwrite.path)}>
        <FolderOpen size={13} /> Open
      </Button>
      <Button
        size="sm"
        disabled={working}
        onClick={() => {
          setName("Overwrite");
          setNaming(true);
        }}
      >
        <PackagePlus size={13} /> Create mod
      </Button>
      <Button
        size="sm"
        variant="danger"
        disabled={working}
        onClick={async () => {
          const sure = await askConfirm({
            title: "Clear overwrite",
            message: `Delete the ${overwrite.files} file(s) in overwrite? Logs and settings the game created will be lost.`,
            confirmLabel: "Delete",
            danger: true,
          });
          if (sure) {
            void run(async () => `Removed ${await ipc.clearOverwrite()} file(s) from overwrite.`);
          }
        }}
      >
        <Trash2 size={13} /> Clear
      </Button>

      <Dialog
        open={naming}
        title="Create a mod from overwrite"
        onClose={() => setNaming(false)}
        width="420px"
        footer={
          <>
            <Button variant="ghost" onClick={() => setNaming(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={working || name.trim() === ""}
              onClick={() =>
                void run(async () => {
                  await ipc.createModFromOverwrite(name);
                  return `“${name.trim()}” created from overwrite and enabled.`;
                })
              }
            >
              Create
            </Button>
          </>
        }
      >
        <div className="space-y-2">
          <input
            autoFocus
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Mod name"
            className="h-8 w-full rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
          />
          <p className="text-[11px] leading-relaxed text-ink-muted">
            The files move out of overwrite into a new mod, enabled at the end of the load order —
            where overwrite sat — so the game sees exactly the same files.
          </p>
        </div>
      </Dialog>
    </div>
  );
}
