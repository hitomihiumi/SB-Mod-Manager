import { useVirtualizer } from "@tanstack/react-virtual";
import clsx from "clsx";
import { AlertTriangle, ArrowDown, RefreshCw, ShieldCheck } from "lucide-react";
import { useMemo, useRef, useState } from "react";

import type { Conflict } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Empty } from "./ui";

const ROW_HEIGHT = 52;

/**
 * Which enabled mods replace the same assets.
 *
 * `~mods` mounts alphanumerically and a later `_P` pak overrides an earlier
 * one, so the mod furthest down the load order is the one the game ends up
 * loading. The point of this screen is to make that visible before the game
 * does, and to say which mod to move.
 */
export function ConflictsView() {
  const report = useApp((s) => s.conflicts);
  const refreshConflicts = useApp((s) => s.refreshConflicts);
  const reindex = useApp((s) => s.reindexAssets);
  const setView = useApp((s) => s.setView);

  const [filter, setFilter] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  const conflicts = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const all = report?.conflicts ?? [];
    if (!needle) return all;
    return all.filter(
      (c) =>
        c.asset.includes(needle) ||
        c.claimants.some((m) => m.name.toLowerCase().includes(needle)),
    );
  }, [report, filter]);

  const virtualizer = useVirtualizer({
    count: conflicts.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
  });

  const involved = new Set(
    (report?.conflicts ?? []).flatMap((c) => c.claimants.map((m) => m.modId)),
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line px-3">
        <h1 className="text-[13px] font-semibold">Conflicts</h1>
        {report && (
          <span className="text-[11px] text-ink-muted">
            {report.conflicts.length === 0
              ? "no enabled mods replace the same asset"
              : `${report.conflicts.length} asset${report.conflicts.length === 1 ? "" : "s"} claimed by ${involved.size} mods`}
          </span>
        )}

        <div className="ml-auto flex items-center gap-1.5">
          <input
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Filter by asset or mod"
            className="h-8 w-56 rounded-md border border-line bg-base px-2.5 text-[12px] placeholder:text-ink-muted focus:border-accent-soft focus:outline-none"
          />
          <Button size="sm" variant="ghost" onClick={() => void refreshConflicts()}>
            <RefreshCw size={13} /> Refresh
          </Button>
        </div>
      </header>

      {report && report.unreadable.length > 0 && (
        <div className="border-b border-line bg-warn/10 px-3 py-2 text-[12px] leading-relaxed text-warn">
          The contents of {report.unreadable.join(", ")} could not be read, so
          anything those mods replace is missing from this list.{" "}
          <button className="underline hover:no-underline" onClick={() => void reindex()}>
            Try again
          </button>
        </div>
      )}

      {conflicts.length === 0 ? (
        <Empty
          icon={report?.conflicts.length ? <AlertTriangle size={20} /> : <ShieldCheck size={20} />}
          title={
            report?.conflicts.length
              ? "Nothing matches that filter"
              : "No mods are fighting over anything"
          }
          hint={
            report?.conflicts.length
              ? "Try a different asset path or mod name."
              : "Every enabled mod replaces assets no other enabled mod touches. Enable more mods, or change the load order, and this list will say what that costs."
          }
        />
      ) : (
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
            {virtualizer.getVirtualItems().map((item) => {
              const conflict = conflicts[item.index];
              if (!conflict) return null;
              return (
                <div
                  key={item.key}
                  className="absolute inset-x-0"
                  style={{ height: item.size, transform: `translateY(${item.start}px)` }}
                >
                  <ConflictRow conflict={conflict} onReorder={() => setView("order")} />
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

function ConflictRow({
  conflict,
  onReorder,
}: {
  conflict: Conflict;
  onReorder: () => void;
}) {
  const winner = conflict.claimants.find((c) => c.wins);
  const losers = conflict.claimants.filter((c) => !c.wins);

  return (
    <div className="flex h-full flex-col justify-center border-b border-line/50 px-3">
      <div className="flex items-center gap-2">
        <span
          className="min-w-0 flex-1 truncate font-mono text-[12px]"
          title={conflict.asset}
        >
          {conflict.asset}
        </span>
        {!conflict.named && (
          <Badge tone="neutral" title="This container has no directory index, so the asset's name is not recorded anywhere — only its chunk id.">
            Unnamed
          </Badge>
        )}
        <button
          onClick={onReorder}
          className="shrink-0 text-[11px] text-ink-muted underline-offset-2 hover:text-ink hover:underline"
        >
          Change order
        </button>
      </div>

      <div className="mt-0.5 flex items-center gap-1.5 text-[11px]">
        {losers.map((mod) => (
          <span key={mod.modId} className="truncate text-ink-muted line-through">
            {mod.name}
          </span>
        ))}
        <ArrowDown size={11} className="shrink-0 text-ink-muted" />
        <span className={clsx("truncate font-medium text-ok")}>{winner?.name}</span>
        <span className="shrink-0 text-ink-muted">loads</span>
      </div>
    </div>
  );
}
