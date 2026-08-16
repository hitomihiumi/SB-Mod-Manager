import { Cpu, RotateCcw, Sparkles } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { ipc, type UpscalerComponent, type UpscalerEntry, type UpscalerStatus } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Section } from "./ui";

/**
 * Updating the game's DLSS and FSR DLLs.
 *
 * Both vendors keep a stable ABI within a release line, so a newer DLL of the
 * same line drops straight in. The swap goes through the ordinary deployment
 * record, which is why "Restore" can give the game's own file back exactly.
 */
export function UpscalerSection() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);

  const [status, setStatus] = useState<UpscalerStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [working, setWorking] = useState<UpscalerComponent | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setStatus(await ipc.upscalerStatus());
    } catch {
      // Checking for newer releases needs the network; without it the section
      // simply shows nothing rather than an error the user cannot act on.
      setStatus(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (snapshot?.game) void load();
  }, [snapshot?.game?.root, load]);

  async function run(
    component: UpscalerComponent,
    action: () => Promise<string>,
  ) {
    setWorking(component);
    try {
      toast("success", await action());
      await load();
      await refresh();
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(null);
    }
  }

  return (
    <Section
      title="Upscaling"
      hint="DLSS and FSR ship as plain DLLs, so a newer one can be dropped in without waiting for a game patch. The file it replaces is kept, and Restore puts it back."
    >
      {status?.adapter && (
        <p className="mb-3 flex items-center gap-1.5 text-[11px] text-ink-muted">
          <Cpu size={12} />
          {status.adapter.name}
        </p>
      )}

      {loading && !status ? (
        <p className="text-[12px] text-ink-muted">Checking…</p>
      ) : !status || status.entries.length === 0 ? (
        <p className="text-[12px] leading-relaxed text-ink-muted">
          No DLSS or FSR files were found in the game folder. That is normal if the
          game has not been launched yet, or if the check could not reach GitHub.
        </p>
      ) : (
        <div className="space-y-2.5">
          {status.entries.map((entry) => (
            <UpscalerRow
              key={entry.component}
              entry={entry}
              busy={working === entry.component}
              disabled={working !== null}
              onUpdate={() => run(entry.component, () => ipc.updateUpscaler(entry.component))}
              onRestore={() =>
                run(entry.component, async () => {
                  await ipc.restoreUpscaler(entry.component);
                  return `${entry.label} restored to the version the game shipped with.`;
                })
              }
            />
          ))}
        </div>
      )}
    </Section>
  );
}

function UpscalerRow({
  entry,
  busy,
  disabled,
  onUpdate,
  onRestore,
}: {
  entry: UpscalerEntry;
  busy: boolean;
  disabled: boolean;
  onUpdate: () => void;
  onRestore: () => void;
}) {
  const current = entry.managedVersion ?? entry.installedVersion;
  // Versions here are the vendors' own, so a plain inequality is the right
  // test — there is no ordering to guess at.
  const canUpdate =
    entry.supported && entry.latestVersion !== null && entry.latestVersion !== current;
  // Not knowing what is published is different from knowing nothing is newer,
  // and saying "up to date" for the first would be a claim we cannot make.
  const unknown = entry.latestVersion === null;

  return (
    <div className="rounded-md border border-line bg-base px-2.5 py-2">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-[13px]">{entry.label}</span>

        {entry.managedVersion && <Badge tone="accent">Updated</Badge>}
        {canUpdate && <Badge tone="warn">{entry.latestVersion} available</Badge>}

        {entry.managedVersion && (
          <Button size="sm" variant="ghost" disabled={disabled} onClick={onRestore}>
            <RotateCcw size={12} /> Restore
          </Button>
        )}
        <Button
          size="sm"
          variant={canUpdate ? "primary" : "outline"}
          disabled={disabled || !canUpdate}
          onClick={onUpdate}
        >
          <Sparkles size={12} />
          {busy ? "Working…" : canUpdate ? "Update" : unknown ? "Can't check" : "Up to date"}
        </Button>
      </div>

      <p className="mt-1 text-[11px] text-ink-muted">
        {current ? `Version ${current}` : "Version unknown"}
        {entry.latestVersion
          ? ` · newest available ${entry.latestVersion}`
          : " · could not reach GitHub to see what is published"}
        {entry.installed.length > 1 && ` · ${entry.installed.length} copies in the game folder`}
      </p>

      {entry.blockedReason && (
        <p className="mt-1 text-[11px] leading-relaxed text-warn">{entry.blockedReason}</p>
      )}
      {entry.note && !entry.blockedReason && (
        <p className="mt-1 text-[11px] leading-relaxed text-ink-muted">{entry.note}</p>
      )}
    </div>
  );
}
