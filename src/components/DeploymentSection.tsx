import { revealItemInDir } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import { FolderOpen, HardDrive, Layers } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc, type DeploymentMode, type Folders } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Section } from "./ui";

const MODES: {
  id: DeploymentMode;
  icon: typeof Layers;
  title: string;
  detail: string;
}[] = [
  {
    id: "hardlink",
    icon: HardDrive,
    title: "Hard links",
    detail:
      "Enabled mods are linked into the game folder. The game can be started any way you like, including from Steam.",
  },
  {
    id: "virtual",
    icon: Layers,
    title: "Virtual file system",
    detail:
      "The game folder stays clean and the game sees the mods only when started from the manager, as with Mod Organizer 2. Proxy DLLs and UE4SS itself are still linked in, because Windows loads them before the virtual file system exists. Files the game creates go to the overwrite folder.",
  },
];

/**
 * How enabled mods reach the game.
 *
 * Switching takes every mod out under the old mode and puts it back under
 * the new one, so neither leaves anything behind for the other.
 */
export function DeploymentSection() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [switching, setSwitching] = useState(false);
  const [folders, setFolders] = useState<Folders | null>(null);

  const mode = snapshot?.deploymentMode ?? "hardlink";

  useEffect(() => {
    if (mode === "virtual") void ipc.folders().then(setFolders).catch(() => setFolders(null));
  }, [mode]);

  async function choose(next: DeploymentMode) {
    if (next === mode) return;
    setSwitching(true);
    try {
      const report = await ipc.setDeploymentMode(next);
      await refresh();
      if (report.failed.length > 0) {
        toast(
          "error",
          `Switched, but ${report.failed.length} mod(s) could not be redeployed: ` +
            report.failed.map((f) => `${f.name} — ${f.reason}`).join("; "),
        );
      } else {
        toast(
          "success",
          next === "virtual"
            ? "Mods now reach the game through the virtual file system. Start the game from the manager."
            : "Mods are linked into the game folder again.",
        );
      }
    } catch (error) {
      toast("error", String(error));
    } finally {
      setSwitching(false);
    }
  }

  return (
    <Section
      title="Deployment"
      hint="How enabled mods reach the game. Switching moves every enabled mod across; nothing is lost."
    >
      <div className="space-y-2">
        {MODES.map((option) => {
          const Icon = option.icon;
          const selected = option.id === mode;
          return (
            <button
              key={option.id}
              type="button"
              disabled={switching}
              onClick={() => void choose(option.id)}
              aria-pressed={selected}
              className={clsx(
                "no-drag flex w-full gap-2.5 rounded-md border px-3 py-2.5 text-left transition-colors disabled:opacity-60",
                selected ? "border-accent-soft bg-accent-soft/10" : "border-line bg-base hover:border-line-strong",
              )}
            >
              <Icon size={16} className={clsx("mt-0.5 shrink-0", selected ? "text-accent-soft" : "text-ink-muted")} />
              <span className="min-w-0">
                <span className="flex items-center gap-2 text-[13px] font-medium">
                  {option.title}
                  {option.id === "virtual" && <Badge tone="warn">Experimental</Badge>}
                </span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-ink-muted">
                  {option.detail}
                </span>
              </span>
            </button>
          );
        })}
      </div>

      {mode === "virtual" && folders && (
        <div className="mt-3 flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-md border border-line bg-base px-2.5 py-1.5 font-mono text-[11px] text-ink-soft">
            {folders.overwrite}
          </code>
          <Button size="sm" variant="outline" onClick={() => void revealItemInDir(folders.overwrite)}>
            <FolderOpen size={12} /> Overwrite
          </Button>
        </div>
      )}
    </Section>
  );
}
