import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FolderOpen, HardDrive, Layers } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc, type DeploymentMode, type Folders } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, ChoiceCard, Section } from "./ui";

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
      <div role="radiogroup" aria-label="Deployment" className="space-y-2">
        {MODES.map((option) => {
          const Icon = option.icon;
          const selected = option.id === mode;
          return (
            <ChoiceCard
              key={option.id}
              disabled={switching}
              selected={selected}
              onSelect={() => void choose(option.id)}
              icon={<Icon size={16} />}
              title={option.title}
              badge={option.id === "virtual" ? <Badge tone="warn">Experimental</Badge> : undefined}
              detail={option.detail}
            />
          );
        })}
      </div>

      {mode === "virtual" && folders && (
        <div className="mt-3 flex items-center gap-2">
          <code className="min-w-0 flex-1 truncate rounded-md border border-transparent bg-base px-2.5 py-1.5 font-mono text-[11px] text-ink-soft">
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
