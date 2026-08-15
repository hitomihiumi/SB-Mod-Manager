import { open } from "@tauri-apps/plugin-dialog";
import { FolderSearch, HardDrive } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc, type GameInstall } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button } from "./ui";

/** First run: nothing else can happen until we know where the game is. */
export function SetupWizard() {
  const chooseGame = useApp((s) => s.chooseGame);
  const [found, setFound] = useState<GameInstall[] | null>(null);

  useEffect(() => {
    void ipc
      .discoverGames()
      .then(setFound)
      .catch(() => setFound([]));
  }, []);

  return (
    <div className="grid h-full place-items-center px-8">
      <div className="w-full max-w-md">
        <div className="mb-5 flex items-center gap-3">
          <div className="size-9 rounded-lg bg-gradient-to-b from-accent to-accent-soft" />
          <div>
            <h1 className="text-[15px] font-semibold">SB Mod Manager</h1>
            <p className="text-[12px] text-ink-muted">Point it at your Stellar Blade install.</p>
          </div>
        </div>

        {found === null ? (
          <p className="text-[12px] text-ink-muted">Looking for installed copies…</p>
        ) : found.length > 0 ? (
          <div className="space-y-2">
            {found.map((install) => (
              <button
                key={install.root}
                onClick={() => void chooseGame(install.root)}
                className="flex w-full items-center gap-3 rounded-lg border border-line bg-surface px-3 py-2.5 text-left transition-colors hover:border-accent-soft"
              >
                <HardDrive size={16} className="shrink-0 text-ink-muted" />
                <div className="min-w-0 flex-1">
                  <div className="text-[12px] font-medium capitalize">{install.source}</div>
                  <div className="truncate font-mono text-[11px] text-ink-muted">
                    {install.root}
                  </div>
                </div>
              </button>
            ))}
          </div>
        ) : (
          <p className="mb-3 text-[12px] leading-relaxed text-ink-muted">
            No install found automatically. Choose the folder that contains{" "}
            <code className="font-mono text-ink-soft">SB/</code> — for Steam that is usually
            <code className="mx-1 font-mono text-ink-soft">steamapps/common/StellarBlade</code>.
          </p>
        )}

        <div className="mt-4">
          <Button
            variant={found && found.length > 0 ? "outline" : "primary"}
            onClick={async () => {
              const picked = await open({
                directory: true,
                title: "Select the Stellar Blade folder",
              });
              if (typeof picked === "string") await chooseGame(picked);
            }}
          >
            <FolderSearch size={14} /> Browse…
          </Button>
        </div>
      </div>
    </div>
  );
}
