import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc, type Folders } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button } from "./ui";

export function SettingsView() {
  const snapshot = useApp((s) => s.snapshot);
  const chooseGame = useApp((s) => s.chooseGame);
  const refresh = useApp((s) => s.refresh);
  const [folders, setFolders] = useState<Folders | null>(null);

  useEffect(() => {
    void ipc.folders().then(setFolders).catch(() => setFolders(null));
  }, [snapshot?.game?.root]);

  const game = snapshot?.game ?? null;

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-6 p-6">
        <Section
          title="Game folder"
          hint="The folder that contains SB/. Detected automatically for Steam and Epic installs."
        >
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-line bg-base px-2.5 py-2 font-mono text-[12px] text-ink-soft">
              {game?.root ?? "Not selected"}
            </code>
            <Button
              variant="outline"
              onClick={async () => {
                const picked = await open({ directory: true, title: "Select the Stellar Blade folder" });
                if (typeof picked === "string") await chooseGame(picked);
              }}
            >
              Change
            </Button>
          </div>
          {game && (
            <p className="mt-2 text-[11px] text-ink-muted">
              Found via {game.source}
              {game.executable ? ` · ${game.executable}` : ""}
            </p>
          )}
        </Section>

        <Section
          title="Applying changes"
          hint="With this off, toggles are staged and only touch the game folder when you press Apply."
        >
          <label className="flex cursor-pointer items-center gap-2.5">
            <input
              type="checkbox"
              checked={snapshot?.autoApply ?? true}
              onChange={async (event) => {
                await ipc.setAutoApply(event.target.checked);
                await refresh();
              }}
              className="size-4 accent-[var(--color-accent)]"
            />
            <span className="text-[13px]">Apply automatically after every change</span>
          </label>
        </Section>

        <Section
          title="Folders"
          hint="Mods are kept unpacked outside the game folder and hard-linked in when enabled."
        >
          <div className="space-y-1.5">
            <PathRow label="Staged mods" path={folders?.mods} />
            <PathRow label="Backups of replaced game files" path={folders?.backups} />
          </div>
        </Section>

        <Section title="About" hint="">
          <p className="text-[12px] leading-relaxed text-ink-muted">
            Stellar Blade is Unreal Engine 4.26 with IoStore, so pak mods are
            <code className="mx-1 font-mono text-ink-soft">.pak/.utoc/.ucas</code>
            triplets that must share a base name and end in <code className="font-mono text-ink-soft">_P</code>.
            The manager handles that, along with the separate locations UE4SS, logic, movie and
            root mods need.
          </p>
        </Section>
      </div>
    </div>
  );
}

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-line bg-surface p-4">
      <h3 className="text-[13px] font-semibold">{title}</h3>
      {hint && <p className="mt-0.5 mb-3 text-[12px] text-ink-muted">{hint}</p>}
      {children}
    </section>
  );
}

function PathRow({ label, path }: { label: string; path?: string }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-56 shrink-0 text-[12px] text-ink-muted">{label}</span>
      <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink-soft">
        {path ?? "—"}
      </code>
      {path && (
        <Button
          size="sm"
          onClick={() => void revealItemInDir(path).catch(() => undefined)}
          aria-label={`Open ${label}`}
        >
          <FolderOpen size={13} />
        </Button>
      )}
    </div>
  );
}
