import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { ArchiveRestore, FolderOpen, History, RefreshCw, Save, Trash2, Wrench } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc, type GameBuild, type RestorePoint, type Saves } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Checkbox, Section } from "./ui";

function buildName(build: GameBuild): string {
  if (build.buildId) return `build ${build.buildId}`;
  if (build.exeModified) return `the ${new Date(build.exeModified * 1000).toLocaleDateString()} executable`;
  return "an unknown build";
}

function when(date: Date): string {
  return date.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/**
 * Over the mod list after the game changed build: an update can put the
 * game's own files back over modded ones and break mods made for the old
 * build, and nothing else would say so.
 */
export function GameUpdateBanner() {
  const update = useApp((s) => s.snapshot?.gameUpdate ?? null);
  const repairDrift = useApp((s) => s.repairDrift);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const setView = useApp((s) => s.setView);
  if (!update) return null;

  async function dismiss() {
    try {
      await ipc.acknowledgeGameUpdate();
      await refresh();
    } catch (error) {
      toast("error", String(error));
    }
  }

  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-warn/30 bg-warn/10 px-3 py-2 text-[12px]">
      <RefreshCw size={14} className="shrink-0 text-warn" />
      <span className="min-w-0 flex-1 leading-relaxed">
        Stellar Blade was updated ({buildName(update.previous)} → {buildName(update.current)}). Mods
        made for the earlier build may misbehave
        {update.drifted > 0
          ? `, and the update replaced or removed ${update.drifted} modded file${update.drifted === 1 ? "" : "s"}.`
          : "."}
      </span>
      {update.drifted > 0 && (
        <Button size="sm" variant="primary" onClick={() => void repairDrift()}>
          <Wrench size={12} /> Repair files
        </Button>
      )}
      <Button size="sm" variant="outline" onClick={() => setView("settings")}>
        <History size={12} /> Restore points
      </Button>
      <Button size="sm" onClick={() => void dismiss()}>
        Got it
      </Button>
    </div>
  );
}

/**
 * The active profile's past setups. One is kept automatically every time
 * changes reach the game, and the user can name their own.
 */
export function RestorePointsSection() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [points, setPoints] = useState<RestorePoint[]>([]);
  const [label, setLabel] = useState("");
  const [confirming, setConfirming] = useState<number | null>(null);
  const [working, setWorking] = useState(false);

  const load = useCallback(() => {
    ipc
      .restorePoints()
      .then(setPoints)
      .catch(() => setPoints([]));
  }, []);
  // Applying changes adds points, and the snapshot changes when it does.
  useEffect(load, [load, snapshot]);

  async function run(action: () => Promise<string>) {
    setWorking(true);
    try {
      toast("success", await action());
      await refresh();
      load();
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
      setConfirming(null);
    }
  }

  const profile = snapshot?.profiles.find((p) => p.active)?.name ?? "this profile";

  return (
    <Section
      title="Restore points"
      hint={`Which mods “${profile}” had enabled, and in what order. One is kept each time changes reach the game; go back to any of them if a change did not work out.`}
    >
      <div className="mb-3 flex gap-2">
        <input
          value={label}
          onChange={(event) => setLabel(event.target.value)}
          onKeyDown={(event) =>
            event.key === "Enter" &&
            label.trim() &&
            void run(async () => {
              await ipc.createRestorePoint(label);
              setLabel("");
              return `Saved “${label.trim()}”.`;
            })
          }
          placeholder="Name the current setup, e.g. Before the new outfits"
          className="h-8 min-w-0 flex-1 rounded-md border border-line bg-base px-2.5 text-[12px] focus:border-accent-soft focus:outline-none"
        />
        <Button
          variant="outline"
          disabled={working || !label.trim()}
          onClick={() =>
            void run(async () => {
              await ipc.createRestorePoint(label);
              setLabel("");
              return `Saved “${label.trim()}”.`;
            })
          }
        >
          <Save size={13} /> Save point
        </Button>
      </div>

      {points.length === 0 ? (
        <p className="text-[12px] text-ink-muted">
          None yet — the first is kept the next time changes are applied.
        </p>
      ) : (
        <ul className="max-h-72 divide-y divide-line overflow-y-auto rounded-md border border-line">
          {points.map((point, index) => (
            <li key={point.id} className="flex items-center gap-2 px-2.5 py-2">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <span className="truncate text-[12px] font-medium">{point.label}</span>
                  {!point.automatic && <Badge tone="accent">Yours</Badge>}
                  {index === 0 && <Badge>Latest</Badge>}
                </div>
                <div className="text-[11px] text-ink-muted">
                  {when(new Date(point.createdAt))} · {point.enabled} enabled
                  {point.gameBuild ? ` · ${point.gameBuild}` : ""}
                  {point.uninstalled > 0 && (
                    <span className="text-warn"> · {point.uninstalled} since uninstalled</span>
                  )}
                </div>
              </div>
              {confirming === point.id ? (
                <>
                  <span className="text-[11px] text-ink-soft">Go back to this?</span>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={working}
                    onClick={() =>
                      void run(async () => {
                        const report = await ipc.restoreToPoint(point.id);
                        if (report.applied.failed.length > 0) {
                          throw new Error(
                            `Went back, but ${report.applied.failed.length} mod(s) could not be deployed: ` +
                              report.applied.failed.map((f) => `${f.name} — ${f.reason}`).join("; "),
                          );
                        }
                        return report.uninstalled > 0
                          ? `Went back to “${point.label}”; ${report.uninstalled} mod(s) it had are no longer installed.`
                          : `Went back to “${point.label}”.`;
                      })
                    }
                  >
                    Yes
                  </Button>
                  <Button size="sm" onClick={() => setConfirming(null)}>
                    No
                  </Button>
                </>
              ) : (
                <>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={working}
                    onClick={() => setConfirming(point.id)}
                  >
                    <History size={12} /> Go back
                  </Button>
                  <Button
                    size="sm"
                    variant="danger"
                    title="Delete this point"
                    disabled={working}
                    onClick={() =>
                      void run(async () => {
                        await ipc.deleteRestorePoint(point.id);
                        return `Deleted “${point.label}”.`;
                      })
                    }
                  >
                    <Trash2 size={12} />
                  </Button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">
        Going back keeps the setup you leave as a point too, so it can be undone. Mods installed
        since are switched off; uninstalled ones cannot come back.
      </p>
    </Section>
  );
}

const KIND_LABEL: Record<string, string> = {
  auto: "Before launch",
  manual: "Yours",
  "before-restore": "Before a restore",
};

/** Backups of the game's saves, made before each launch and on request. */
export function SavesSection() {
  const toast = useApp((s) => s.toast);
  const [saves, setSaves] = useState<Saves | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [working, setWorking] = useState(false);

  const load = useCallback(() => {
    ipc
      .saves()
      .then(setSaves)
      .catch(() => setSaves(null));
  }, []);
  useEffect(load, [load]);

  async function run(action: () => Promise<string | void>) {
    setWorking(true);
    try {
      const message = await action();
      if (message) toast("success", message);
      load();
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
      setConfirming(null);
    }
  }

  async function chooseDir() {
    const picked = await open({ directory: true, title: "Select the folder Stellar Blade saves to" });
    if (typeof picked === "string") await run(() => ipc.setSavesDir(picked));
  }

  return (
    <Section
      title="Save backups"
      hint="A mod that goes wrong can damage a save, and a save made with a mod may not load without it. The saves folder is zipped before each launch when something in it changed."
    >
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md border border-line bg-base px-2.5 py-2 font-mono text-[12px] text-ink-soft">
          {saves?.dir ?? "Not found — choose the folder"}
        </code>
        {saves?.exists && saves.dir && (
          <Button variant="ghost" title="Show in Explorer" onClick={() => void revealItemInDir(saves.dir!)}>
            <FolderOpen size={13} />
          </Button>
        )}
        <Button variant="outline" onClick={() => void chooseDir()}>
          Change
        </Button>
        {saves?.customDir && (
          <Button variant="ghost" onClick={() => void run(() => ipc.setSavesDir(null))}>
            Default
          </Button>
        )}
      </div>
      <p className="mt-2 text-[11px] text-ink-muted">
        {saves === null
          ? ""
          : !saves.exists
            ? "No saves folder here yet. Stellar Blade keeps its saves in %LOCALAPPDATA%\\SB\\Saved\\SaveGames."
            : `${saves.files} file${saves.files === 1 ? "" : "s"} in the saves folder`}
      </p>

      <div className="mt-3 flex items-center gap-3">
        <label className="flex flex-1 cursor-pointer items-center gap-2.5">
          <Checkbox
            checked={saves?.backupOnLaunch ?? true}
            onChange={(next) => void run(() => ipc.setBackupSavesOnLaunch(next))}
          />
          <span className="text-[13px]">Back up before every launch</span>
        </label>
        <Button
          variant="outline"
          disabled={working || !saves?.exists}
          onClick={() =>
            void run(async () => {
              await ipc.backUpSaves();
              return "Saves backed up.";
            })
          }
        >
          <Save size={13} /> Back up now
        </Button>
      </div>

      {saves && saves.backups.length > 0 && (
        <ul className="mt-3 max-h-64 divide-y divide-line overflow-y-auto rounded-md border border-line">
          {saves.backups.map((backup) => (
            <li key={backup.file} className="flex items-center gap-2 px-2.5 py-2">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5 text-[12px]">
                  {when(new Date(backup.createdAt * 1000))}
                  <Badge tone={backup.kind === "manual" ? "accent" : undefined}>
                    {KIND_LABEL[backup.kind] ?? backup.kind}
                  </Badge>
                </div>
                <div className="text-[11px] text-ink-muted">{formatBytes(backup.bytes)}</div>
              </div>
              {confirming === backup.file ? (
                <>
                  <span className="text-[11px] text-ink-soft">Replace the current saves?</span>
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={working}
                    onClick={() =>
                      void run(async () => {
                        await ipc.restoreSaves(backup.file);
                        return "Saves restored. The ones replaced were backed up first.";
                      })
                    }
                  >
                    Yes
                  </Button>
                  <Button size="sm" onClick={() => setConfirming(null)}>
                    No
                  </Button>
                </>
              ) : (
                <>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={working}
                    onClick={() => setConfirming(backup.file)}
                  >
                    <ArchiveRestore size={12} /> Restore
                  </Button>
                  <Button
                    size="sm"
                    variant="danger"
                    title="Delete this backup"
                    disabled={working}
                    onClick={() => void run(() => ipc.deleteSaveBackup(backup.file))}
                  >
                    <Trash2 size={12} />
                  </Button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
      <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">
        Close the game before restoring. With Steam Cloud on, Steam may offer its own copy at the
        next start — keep the local one.
      </p>
    </Section>
  );
}
