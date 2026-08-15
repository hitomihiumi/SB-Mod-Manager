import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { FolderOpen, UserRound } from "lucide-react";
import { useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc, type Folders } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button } from "./ui";

export function SettingsView() {
  const snapshot = useApp((s) => s.snapshot);
  const chooseGame = useApp((s) => s.chooseGame);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [folders, setFolders] = useState<Folders | null>(null);
  const [moving, setMoving] = useState(false);

  useEffect(() => {
    void ipc
      .folders()
      .then(setFolders)
      .catch(() => setFolders(null));
  }, [snapshot?.game?.root]);

  const game = snapshot?.game ?? null;

  /// Moving the library takes every enabled mod out of the game folder and puts
  /// it back afterwards, so it can take a while on a large collection.
  async function moveLibrary() {
    const picked = await open({
      directory: true,
      title: "Choose a folder for the mod library",
    });
    if (typeof picked !== "string") return;

    setMoving(true);
    try {
      const report = await ipc.setLibraryRoot(picked);
      setFolders(report.folders);
      await refresh();

      if (report.redeployed.failed.length > 0) {
        toast(
          "error",
          `Moved, but ${report.redeployed.failed.length} mod(s) could not be re-enabled: ` +
            report.redeployed.failed.map((f) => `${f.name} — ${f.reason}`).join("; "),
        );
      } else {
        toast("success", `Library moved to ${picked}.`);
      }
    } catch (error) {
      toast("error", String(error));
    } finally {
      setMoving(false);
    }
  }

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

        <NexusSection />

        <Section
          title="Mod library"
          hint="Mods are kept unpacked here, outside the game folder, and hard-linked in when enabled. Move it to another drive to keep it off the system disk."
        >
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-line bg-base px-2.5 py-2 font-mono text-[12px] text-ink-soft">
              {folders?.library ?? "—"}
            </code>
            <Button variant="outline" disabled={moving} onClick={moveLibrary}>
              {moving ? "Moving…" : "Move…"}
            </Button>
          </div>

          {folders && (
            <p className="mt-2 text-[11px] text-ink-muted">
              {formatBytes(folders.libraryBytes)} in use
            </p>
          )}

          {folders && !folders.sameVolumeAsGame && (
            <p className="mt-2 rounded-md border border-warn/40 bg-warn/10 px-2.5 py-2 text-[11px] leading-relaxed text-warn">
              The library is on a different drive from the game, so hard links are not
              possible. Enabled mods are copied instead, which stores every one of them
              twice and makes applying changes slower. Put the library on the same drive
              as the game to avoid that.
            </p>
          )}

          <div className="mt-3 space-y-1.5 border-t border-line pt-3">
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

/// Nexus account. The key goes to the OS credential store, never the database,
/// and premium status is shown because it decides whether a collection can
/// install without the user clicking through each mod page.
function NexusSection() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);

  const account = snapshot?.nexus ?? null;
  const [key, setKey] = useState("");
  const [checking, setChecking] = useState(false);

  async function save(value: string) {
    setChecking(true);
    try {
      const result = await ipc.setNexusKey(value);
      setKey("");
      await refresh();
      toast(
        "success",
        result ? `Signed in as ${result.name}.` : "Nexus key removed.",
      );
    } catch (error) {
      toast("error", String(error));
    } finally {
      setChecking(false);
    }
  }

  return (
    <Section
      title="Nexus Mods"
      hint="Needed to download mods and collections. Your key is kept in the Windows Credential Manager, not in the manager's database."
    >
      {account ? (
        <div className="flex items-center gap-2.5">
          <UserRound size={16} className="shrink-0 text-ink-muted" />
          <div className="min-w-0 flex-1">
            <div className="text-[13px] font-medium">{account.name}</div>
            <div className="text-[11px] text-ink-muted">
              {account.isPremium ? "Premium" : "Free account"}
            </div>
          </div>
          <Button variant="ghost" disabled={checking} onClick={() => void save("")}>
            Sign out
          </Button>
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <input
            type="password"
            value={key}
            onChange={(event) => setKey(event.target.value)}
            placeholder="Personal API key"
            spellCheck={false}
            className="h-8 min-w-0 flex-1 rounded-md border border-line bg-base px-2.5 font-mono text-[12px] placeholder:font-sans placeholder:text-ink-muted focus:border-accent-soft focus:outline-none"
          />
          <Button
            variant="primary"
            disabled={checking || key.trim().length === 0}
            onClick={() => void save(key)}
          >
            {checking ? "Checking…" : "Connect"}
          </Button>
        </div>
      )}

      {!account && (
        <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">
          Create one under Account settings → API keys on the Nexus Mods site.
        </p>
      )}

      {account && !account.isPremium && (
        <p className="mt-2.5 rounded-md border border-line bg-base px-2.5 py-2 text-[11px] leading-relaxed text-ink-muted">
          Nexus only hands direct download links to Premium accounts. Without it, use the
          <span className="text-ink-soft"> Mod Manager Download </span>
          button on a mod page — the manager picks the link up and installs the mod for you.
          Collections install the same way, one click per mod.
        </p>
      )}
    </Section>
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
