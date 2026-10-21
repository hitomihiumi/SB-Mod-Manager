import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Eye, FolderOpen, HardDrive, Loader2, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc, type Cleanup, type StorageReport, type WatchSettings } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Checkbox, Section } from "./ui";

/** Which cleanup frees what in each part of the library. */
const CLEANUP: Record<string, { what: Cleanup; label: string; hint: string }> = {
  mods: {
    what: "orphans",
    label: "Remove leftovers",
    hint: "Extracted archives no installed mod owns — cancelled or failed installs.",
  },
  downloads: {
    what: "downloads",
    label: "Delete archives",
    hint: "Downloaded archives that are not being fetched or installed. Installed mods do not need them.",
  },
  saves: {
    what: "saveBackups",
    label: "Keep newest 3",
    hint: "Automatic save backups beyond the three newest. Ones you made yourself stay.",
  },
  vfs: {
    what: "logs",
    label: "Delete logs",
    hint: "Logs from earlier launches.",
  },
};

/**
 * Where the library's space goes and what can safely go. Game files set
 * aside are listed but never offered: disabling a mod puts them back.
 */
export function StorageSection() {
  const uninstall = useApp((s) => s.uninstall);
  const toast = useApp((s) => s.toast);
  const [report, setReport] = useState<StorageReport | null>(null);
  const [scanning, setScanning] = useState(false);

  async function scan() {
    setScanning(true);
    try {
      setReport(await ipc.storageReport());
    } catch (error) {
      toast("error", String(error));
    } finally {
      setScanning(false);
    }
  }

  async function clean(what: Cleanup) {
    try {
      const freed = await ipc.cleanStorage(what);
      toast("success", `Freed ${formatBytes(freed)}.`);
      await scan();
    } catch (error) {
      toast("error", String(error));
    }
  }

  const largest = Math.max(1, ...(report?.parts.map((p) => p.bytes) ?? [1]));

  return (
    <Section
      title="Disk space"
      hint="What the library holds, and what can go without affecting any installed mod."
    >
      {!report ? (
        <Button variant="outline" disabled={scanning} onClick={() => void scan()}>
          {scanning ? <Loader2 size={13} className="animate-spin" /> : <HardDrive size={13} />}
          {scanning ? "Measuring…" : "Measure"}
        </Button>
      ) : (
        <>
          <div className="mb-3 flex items-center gap-2">
            <span className="flex-1 text-[13px] font-medium">{formatBytes(report.total)} in all</span>
            <Button size="sm" disabled={scanning} onClick={() => void scan()}>
              {scanning ? "Measuring…" : "Measure again"}
            </Button>
          </div>
          <ul className="space-y-2.5">
            {report.parts.map((part) => {
              const cleanup = CLEANUP[part.key];
              return (
                <li key={part.key}>
                  <div className="flex items-center gap-2 text-[12px]">
                    <span className="min-w-0 flex-1 truncate">{part.label}</span>
                    <span className="text-ink-muted tabular-nums">{formatBytes(part.bytes)}</span>
                    <button
                      title={part.path}
                      onClick={() => void revealItemInDir(part.path)}
                      className="text-ink-muted hover:text-ink"
                      aria-label={`Show ${part.label}`}
                    >
                      <FolderOpen size={12} />
                    </button>
                  </div>
                  <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-hover">
                    <div
                      className="h-full rounded-full bg-accent/70"
                      style={{ width: `${(part.bytes / largest) * 100}%` }}
                    />
                  </div>
                  {cleanup && part.cleanableFiles > 0 && (
                    <div className="mt-1 flex items-center gap-2">
                      <span className="flex-1 text-[11px] text-ink-muted" title={cleanup.hint}>
                        {formatBytes(part.cleanableBytes)} can go — {cleanup.hint}
                      </span>
                      <Button size="sm" variant="outline" onClick={() => void clean(cleanup.what)}>
                        <Trash2 size={12} /> {cleanup.label}
                      </Button>
                    </div>
                  )}
                </li>
              );
            })}
          </ul>

          {report.unusedMods.length > 0 && (
            <div className="mt-4 border-t border-line pt-3">
              <p className="mb-1.5 text-[12px] font-medium">Not enabled in any profile</p>
              <ul className="max-h-48 space-y-1 overflow-y-auto">
                {report.unusedMods.map((mod) => (
                  <li key={mod.id} className="flex items-center gap-2 text-[12px]">
                    <span className="min-w-0 flex-1 truncate">{mod.name}</span>
                    <span className="text-[11px] text-ink-muted tabular-nums">
                      {formatBytes(mod.bytes)}
                    </span>
                    <Button
                      size="sm"
                      variant="danger"
                      onClick={async () => {
                        if (!window.confirm(`Remove "${mod.name}" and delete its files?`)) return;
                        await uninstall(mod.id);
                        await scan();
                      }}
                    >
                      Uninstall
                    </Button>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </>
      )}
    </Section>
  );
}

/**
 * Watch the folder the browser saves to, so a mod downloaded by hand from
 * any site is offered for install the moment it finishes.
 */
export function WatchSection() {
  const toast = useApp((s) => s.toast);
  const [settings, setSettings] = useState<WatchSettings | null>(null);

  const load = () => {
    ipc
      .watchSettings()
      .then(setSettings)
      .catch(() => setSettings(null));
  };
  useEffect(load, []);

  async function change(action: () => Promise<void>) {
    try {
      await action();
      load();
    } catch (error) {
      toast("error", String(error));
    }
  }

  return (
    <Section
      title="Watch a downloads folder"
      hint="Mod archives (.zip, .7z, .rar) that finish downloading into this folder are offered for install. What is in it already is left alone."
    >
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md border border-line bg-base px-2.5 py-2 font-mono text-[12px] text-ink-soft">
          {settings?.dir ?? "Not watching"}
        </code>
        <Button
          variant="outline"
          onClick={async () => {
            const picked = await open({ directory: true, title: "Select your downloads folder" });
            if (typeof picked === "string") await change(() => ipc.setWatchDir(picked));
          }}
        >
          <Eye size={13} /> {settings?.dir ? "Change" : "Watch…"}
        </Button>
        {settings?.dir && (
          <Button variant="ghost" onClick={() => void change(() => ipc.setWatchDir(null))}>
            Stop
          </Button>
        )}
      </div>
      {settings?.dir && (
        <label className="mt-3 flex cursor-pointer items-center gap-2.5">
          <Checkbox
            checked={settings.autoInstall}
            onChange={(next) => void change(() => ipc.setWatchAutoInstall(next))}
          />
          <span className="text-[13px]">Install without asking</span>
        </label>
      )}
    </Section>
  );
}
