import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Download, FolderOpen, UserRound } from "lucide-react";
import { useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc, type Folders, type UpdateChannel, type UpdateInfo } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Checkbox, Section } from "./ui";
import { UpscalerSection } from "./UpscalerSection";

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

        <DiscordSection />

        <UpscalerSection />

        <UpdateSection />

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

/// Discord presence. The only thing this app publishes anywhere, so the switch
/// is here rather than buried, and the copy says exactly what is sent.
function DiscordSection() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const enabled = snapshot?.discordRpc ?? true;

  async function toggle(next: boolean) {
    try {
      await ipc.setDiscordRpc(next);
      await refresh();
    } catch (error) {
      toast("error", String(error));
    }
  }

  return (
    <Section
      title="Discord"
      hint="Shows on your Discord profile that the manager is open. Only counts are sent — never the name of a mod, a collection or a folder."
    >
      <label className="flex cursor-pointer items-center gap-2.5">
        <Checkbox
          checked={enabled}
          onChange={(next) => void toggle(next)}
          label="Show activity on Discord"
        />
        <span className="text-[13px]">Show activity on Discord</span>
      </label>
      <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">
        {enabled
          ? "Your profile shows \u201CManaging mods\u201D and how many of your installed mods are switched on."
          : "Nothing is sent to Discord."}
      </p>
    </Section>
  );
}

/// Self-update. Stable follows tagged releases; nightly follows the rolling
/// prerelease the release workflow rebuilds whenever something lands.
function UpdateSection() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);

  const channel = snapshot?.updateChannel ?? "stable";
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [busy, setBusy] = useState<"checking" | "installing" | null>(null);

  // Shown straight away rather than waiting for a check to succeed.
  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  async function check() {
    setBusy("checking");
    try {
      const result = await ipc.checkForUpdate();
      setInfo(result);
      if (!result.available) toast("info", "You are on the latest build.");
    } catch (error) {
      toast("error", String(error));
    } finally {
      setBusy(null);
    }
  }

  async function switchChannel(next: UpdateChannel) {
    try {
      await ipc.setUpdateChannel(next);
      setInfo(null);
      await refresh();
    } catch (error) {
      toast("error", String(error));
    }
  }

  return (
    <Section
      title="App updates"
      hint="Updates are downloaded from this project's GitHub releases and checked against a signature before anything is replaced."
    >
      <div className="flex items-center gap-2.5">
        <span className="text-[12px] text-ink-muted">Version</span>
        <code className="font-mono text-[12px] text-ink-soft">
          {info?.currentVersion ?? version ?? "—"}
        </code>
        {channel === "nightly" && (
          <span className="rounded bg-warn/15 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide text-warn uppercase">
            nightly
          </span>
        )}
        <div className="ml-auto" />
        <Button variant="outline" disabled={busy !== null} onClick={check}>
          {busy === "checking" ? "Checking…" : "Check now"}
        </Button>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-1.5">
        {(
          [
            ["stable", "Stable", "Tagged releases only"],
            ["nightly", "Nightly", "Latest commit, rough edges"],
          ] as const
        ).map(([id, label, detail]) => (
          <button
            key={id}
            onClick={() => void switchChannel(id)}
            className={
              "rounded-md border px-2.5 py-2 text-left transition-colors " +
              (channel === id
                ? "border-accent bg-accent/10"
                : "border-line hover:border-line-strong")
            }
          >
            <div className="text-[12px] font-medium">{label}</div>
            <div className="text-[10px] text-ink-muted">{detail}</div>
          </button>
        ))}
      </div>

      {info?.available && (
        <div className="mt-3 rounded-md border border-accent-soft/40 bg-accent-soft/10 p-3">
          <div className="flex items-center gap-2">
            <Download size={14} className="shrink-0 text-accent-soft" />
            <span className="text-[12px] font-medium">
              Version {info.available.version} is available
            </span>
            <div className="ml-auto" />
            <Button
              variant="primary"
              size="sm"
              disabled={busy !== null}
              onClick={async () => {
                setBusy("installing");
                try {
                  // On success the app restarts, so nothing after this runs.
                  await ipc.installUpdate();
                } catch (error) {
                  toast("error", String(error));
                  setBusy(null);
                }
              }}
            >
              {busy === "installing" ? "Installing…" : "Install and restart"}
            </Button>
          </div>
          {info.available.notes && (
            <p className="mt-2 max-h-32 overflow-y-auto text-[11px] leading-relaxed whitespace-pre-wrap text-ink-muted">
              {info.available.notes}
            </p>
          )}
        </div>
      )}
    </Section>
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
