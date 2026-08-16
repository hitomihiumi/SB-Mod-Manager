import clsx from "clsx";
import { ExternalLink, Layers, Loader2, Search } from "lucide-react";
import { useEffect, useState } from "react";

import {
  ipc,
  type CollectionPlan,
  type ManualMod,
  type PlannedMod,
  type QueuedFile,
} from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Checkbox, Empty } from "./ui";

/**
 * Installing a collection.
 *
 * The screen only chooses what goes into the download queue; the queue already
 * knows how to fetch a file outright on a Premium account and how to walk a
 * free one through the website a mod at a time, so nothing about the two modes
 * is decided here.
 */
export function CollectionsView() {
  const snapshot = useApp((s) => s.snapshot);
  const setView = useApp((s) => s.setView);
  const refreshDownloads = useApp((s) => s.refreshDownloads);
  const toast = useApp((s) => s.toast);

  const pendingCollection = useApp((s) => s.pendingCollection);
  const setPendingCollection = useApp((s) => s.setPendingCollection);

  const [link, setLink] = useState("");
  const [plan, setPlan] = useState<CollectionPlan | null>(null);
  const [chosen, setChosen] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);

  const premium = snapshot?.nexus?.isPremium ?? false;
  const hasKey = snapshot?.nexus != null;

  async function resolve(value: string = link) {
    setBusy(true);
    try {
      const found = await ipc.resolveCollection(value.trim());
      setPlan(found);
      // Required mods that are not installed yet start selected; optional ones
      // are a choice, not a default, and installed ones have nothing to do.
      setChosen(
        new Set(
          found.mods.filter((m) => !m.optional && !m.installed).map((m) => m.fileId),
        ),
      );
    } catch (error) {
      setPlan(null);
      toast("error", String(error));
    } finally {
      setBusy(false);
    }
  }

  async function install() {
    if (!plan) return;
    const files: QueuedFile[] = plan.mods
      .filter((m) => chosen.has(m.fileId))
      .map((m) => ({ modId: m.modId, fileId: m.fileId, name: m.name, version: m.version }));
    if (files.length === 0) return;

    setBusy(true);
    try {
      const queued = await ipc.installCollection(plan.slug, files);
      await refreshDownloads();
      setView("downloads");
      toast(
        "success",
        premium
          ? `Queued ${queued} mod${queued === 1 ? "" : "s"} from ${plan.name}.`
          : `Queued ${queued} mod${queued === 1 ? "" : "s"}. Each one opens its Nexus page — press Mod Manager Download there and the rest is automatic.`,
      );
    } catch (error) {
      toast("error", String(error));
    } finally {
      setBusy(false);
    }
  }

  // A link that arrived from the browser is looked up straight away — the user
  // already pressed the button on the site, so asking for another click here
  // would be one step too many.
  useEffect(() => {
    if (!pendingCollection) return;
    setLink(pendingCollection);
    setPendingCollection(null);
    void resolve(pendingCollection);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingCollection]);

  function toggle(fileId: number) {
    setChosen((current) => {
      const next = new Set(current);
      if (next.has(fileId)) next.delete(fileId);
      else next.add(fileId);
      return next;
    });
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line px-3">
        <h1 className="text-[13px] font-semibold">Collections</h1>
        {plan && (
          <span className="text-[11px] text-ink-muted">
            {plan.name} · revision {plan.revision || "latest"}
          </span>
        )}
      </header>

      {!hasKey && (
        <p className="border-b border-line bg-warn/10 px-3 py-2 text-[12px] text-warn">
          Add your Nexus API key in Settings before installing a collection.
        </p>
      )}

      <form
        className="flex shrink-0 items-center gap-2 border-b border-line px-3 py-2.5"
        onSubmit={(event) => {
          event.preventDefault();
          if (link.trim()) void resolve(link);
        }}
      >
        <input
          value={link}
          onChange={(event) => setLink(event.target.value)}
          placeholder="Collection link or slug"
          spellCheck={false}
          className="h-8 min-w-0 flex-1 rounded-md border border-line-strong bg-base px-2.5 font-mono text-[12px] outline-none focus:border-accent"
        />
        <Button type="submit" size="sm" variant="outline" disabled={busy || !link.trim()}>
          {busy && !plan ? <Loader2 size={13} className="animate-spin" /> : <Search size={13} />}
          Look up
        </Button>
      </form>

      {!plan ? (
        <Empty
          icon={<Layers size={20} />}
          title="No collection loaded"
          hint="Paste a collection link from Nexus — the nxm:// link its Add to manager button produces, the page address, or just the slug."
        />
      ) : (
        <>
          <div className="min-h-0 flex-1 overflow-y-auto">
            {plan.mods.map((mod) => (
              <ModRow
                key={`${mod.modId}-${mod.fileId}`}
                mod={mod}
                checked={chosen.has(mod.fileId)}
                onToggle={() => toggle(mod.fileId)}
              />
            ))}

            {plan.manual.length > 0 && <ManualList entries={plan.manual} />}
          </div>

          <footer className="flex h-12 shrink-0 items-center gap-3 border-t border-line bg-raised px-3">
            <span className="text-[12px] text-ink-soft">
              {chosen.size} of {plan.mods.length} selected
            </span>
            {!premium && hasKey && (
              <span className="truncate text-[11px] text-ink-muted">
                Free account: one click per mod on the page that opens, nothing else.
              </span>
            )}
            <div className="ml-auto" />
            <Button
              variant="primary"
              size="sm"
              disabled={busy || chosen.size === 0 || !hasKey}
              onClick={() => void install()}
            >
              {busy ? "Queueing…" : `Install ${chosen.size}`}
            </Button>
          </footer>
        </>
      )}
    </div>
  );
}

function ModRow({
  mod,
  checked,
  onToggle,
}: {
  mod: PlannedMod;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center gap-2.5 border-b border-line/50 px-3 py-2">
      <Checkbox checked={checked} onChange={onToggle} label={`Include ${mod.name}`} />
      <span
        className={clsx(
          "min-w-0 flex-1 truncate text-[13px]",
          mod.installed ? "text-ink-muted" : "text-ink",
        )}
      >
        {mod.name}
      </span>
      {mod.version && (
        <span className="shrink-0 font-mono text-[11px] text-ink-muted">{mod.version}</span>
      )}
      {mod.optional && <Badge tone="neutral">Optional</Badge>}
      {mod.installed && <Badge tone="accent">Installed</Badge>}
    </div>
  );
}

/// Entries the API cannot fetch. Naming them is the whole point — a collection
/// that quietly drops them looks installed while the game is still missing mods.
function ManualList({ entries }: { entries: ManualMod[] }) {
  return (
    <div className="border-t border-line px-3 py-3">
      <h2 className="text-[12px] font-semibold text-warn">
        {entries.length} mod{entries.length === 1 ? "" : "s"} must be downloaded by hand
      </h2>
      <p className="mt-0.5 mb-2 text-[11px] leading-relaxed text-ink-muted">
        These are not hosted on Nexus, so the manager cannot fetch them for you. Drop
        the archives onto this window once you have them.
      </p>
      {entries.map((entry, index) => (
        <div key={`${entry.name}-${index}`} className="flex items-center gap-2 py-1">
          <span className="min-w-0 flex-1 truncate text-[12px]">{entry.name}</span>
          <span className="shrink-0 text-[11px] text-ink-muted">{entry.reason}</span>
          {entry.url && (
            <a
              href={entry.url}
              target="_blank"
              rel="noreferrer"
              className="shrink-0 text-ink-muted hover:text-ink"
              aria-label={`Open the page for ${entry.name}`}
            >
              <ExternalLink size={12} />
            </a>
          )}
        </div>
      ))}
    </div>
  );
}
