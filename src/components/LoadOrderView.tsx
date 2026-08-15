import clsx from "clsx";
import { ArrowDown, ArrowUp, GripVertical, ListOrdered } from "lucide-react";
import { useState } from "react";

import type { ModView } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Empty } from "./ui";

/**
 * Only loose paks in `~mods` are resolved by name order, so those are the only
 * ones worth ordering here. Logic mods are resolved by UE4SS itself.
 */
export function LoadOrderView() {
  const snapshot = useApp((s) => s.snapshot);
  const reorder = useApp((s) => s.reorder);
  const [dragging, setDragging] = useState<number | null>(null);

  const all = snapshot?.mods ?? [];
  const ordered = all.filter((m) => m.componentTypes.includes("genericPak"));

  if (ordered.length === 0) {
    return (
      <Empty
        icon={<ListOrdered size={20} />}
        title="Nothing to order yet"
        hint="Load order applies to loose pak mods in ~mods. Install one and it will show up here."
      />
    );
  }

  /** Persist a new arrangement, keeping mods that are not shown in place. */
  async function commit(next: ModView[]) {
    const rest = all.filter((m) => !next.some((n) => n.id === m.id));
    await reorder([...next, ...rest].map((m) => m.id));
  }

  function move(index: number, delta: number) {
    const next = [...ordered];
    const target = index + delta;
    if (target < 0 || target >= next.length) return;
    const [item] = next.splice(index, 1);
    if (item) next.splice(target, 0, item);
    void commit(next);
  }

  function drop(from: number, to: number) {
    if (from === to) return;
    const next = [...ordered];
    const [item] = next.splice(from, 1);
    if (item) next.splice(to, 0, item);
    void commit(next);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center border-b border-line px-4">
        <p className="text-[12px] text-ink-muted">
          Mods higher in this list load first. Files are renamed with a numeric prefix so the
          game mounts them in this order.
        </p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        <ul className="space-y-1">
          {ordered.map((mod, index) => (
            <li
              key={mod.id}
              draggable
              onDragStart={() => setDragging(index)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={() => {
                if (dragging !== null) drop(dragging, index);
                setDragging(null);
              }}
              onDragEnd={() => setDragging(null)}
              className={clsx(
                "flex h-10 items-center gap-2.5 rounded-md border border-line bg-surface px-2.5",
                dragging === index && "opacity-40",
                !mod.enabled && "opacity-55",
              )}
            >
              <GripVertical size={14} className="shrink-0 cursor-grab text-ink-muted" />
              <span className="w-9 shrink-0 text-[11px] text-ink-muted tabular-nums">
                {String(index + 1).padStart(3, "0")}
              </span>
              <span className="min-w-0 flex-1 truncate text-[13px]">{mod.name}</span>
              {!mod.enabled && <span className="text-[11px] text-ink-muted">disabled</span>}
              <Button
                size="sm"
                onClick={() => move(index, -1)}
                disabled={index === 0}
                aria-label="Move up"
              >
                <ArrowUp size={13} />
              </Button>
              <Button
                size="sm"
                onClick={() => move(index, 1)}
                disabled={index === ordered.length - 1}
                aria-label="Move down"
              >
                <ArrowDown size={13} />
              </Button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
