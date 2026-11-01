import clsx from "clsx";
import {
  AlertTriangle,
  ArrowDown,
  ArrowDownUp,
  ArrowUp,
  ChevronsDown,
  ChevronsUp,
  GripVertical,
  ListOrdered,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import type { ModView } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Empty } from "./ui";

/** How far the pointer must travel before a press becomes a drag. */
const DRAG_THRESHOLD = 5;

/**
 * The mods whose place in the order matters: loose paks in `~mods`, which
 * mount by name order, and any mod sharing loose files with another or
 * named in a load order rule, since the later one wins. Logic mods are
 * resolved by UE4SS itself.
 *
 * Rows are picked like in any list (click, Ctrl, Shift, Ctrl+A) and moved as a
 * block: by dragging, by the buttons, or by typing a position over the number.
 * Dragging is done with pointer events rather than the browser's drag and
 * drop, because the window takes over file drops from the system and a row
 * dragged inside it would otherwise be mistaken for one.
 */
export function LoadOrderView() {
  const snapshot = useApp((s) => s.snapshot);
  const reorder = useApp((s) => s.reorder);
  const sortByRules = useApp((s) => s.sortByRules);

  const [selection, setSelection] = useState<number[]>([]);
  const [anchor, setAnchor] = useState<number | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [drag, setDrag] = useState<{ ids: number[]; gap: number; x: number; y: number } | null>(null);

  const listRef = useRef<HTMLUListElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const press = useRef<{ id: number; x: number; y: number } | null>(null);

  const all = snapshot?.mods ?? [];
  const ordered = useMemo(
    () =>
      all.filter(
        (m) =>
          m.componentTypes.includes("genericPak") ||
          m.fileConflicts !== null ||
          m.misordered.length > 0,
      ),
    [all],
  );
  const misordered = all.filter((m) => m.misordered.length > 0).length;
  const ids = ordered.map((m) => m.id);

  // Mods that left the list (uninstalled, renamed out of scope) leave the selection.
  useEffect(() => {
    setSelection((current) => current.filter((id) => ids.includes(id)));
    // `ids` is rebuilt every render; its contents are what matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ids.join(",")]);

  /** Persist a new arrangement, keeping mods that are not shown in place. */
  async function commit(next: ModView[]) {
    const rest = all.filter((m) => !next.some((n) => n.id === m.id));
    await reorder([...next, ...rest].map((m) => m.id));
  }

  /**
   * Put `moving` (in their current relative order) so that the first of them
   * sits at `index` of the list the others leave behind.
   */
  function placeBlock(moving: number[], index: number) {
    const block = ordered.filter((m) => moving.includes(m.id));
    const rest = ordered.filter((m) => !moving.includes(m.id));
    const at = Math.max(0, Math.min(rest.length, index));
    rest.splice(at, 0, ...block);
    void commit(rest);
  }

  /** The gap (0..n) a pointer at `y` is nearest to, from where the rows really are. */
  function gapAt(y: number) {
    const rows = listRef.current?.querySelectorAll<HTMLElement>("[data-row]") ?? [];
    let gap = 0;
    rows.forEach((row, index) => {
      const box = row.getBoundingClientRect();
      if (y > box.top + box.height / 2) gap = index + 1;
    });
    return gap;
  }

  /** Where a block dropped on `gap` lands, counted among the rows that stay. */
  function indexAmongRest(moving: number[], gap: number) {
    return ordered.slice(0, gap).filter((m) => !moving.includes(m.id)).length;
  }

  // ----- selection -------------------------------------------------------

  function select(mod: ModView, event: React.MouseEvent | React.PointerEvent) {
    const index = ids.indexOf(mod.id);
    if (event.shiftKey && anchor !== null) {
      const from = ids.indexOf(anchor);
      const [lo, hi] = from < index ? [from, index] : [index, from];
      setSelection(ids.slice(lo, hi + 1));
      return;
    }
    if (event.ctrlKey || event.metaKey) {
      setSelection((c) => (c.includes(mod.id) ? c.filter((id) => id !== mod.id) : [...c, mod.id]));
    } else {
      setSelection([mod.id]);
    }
    setAnchor(mod.id);
  }

  // ----- moving ----------------------------------------------------------

  /** What the buttons on one row act on: the whole selection if the row is in it. */
  const targetsOf = (id: number) => (selection.includes(id) && selection.length > 1 ? selection : [id]);

  /** Swap each picked row with the unpicked neighbour on the given side. */
  function nudge(moving: number[], delta: -1 | 1) {
    const next = [...ordered];
    const picked = (m: ModView) => moving.includes(m.id);
    if (delta === -1) {
      for (let i = 1; i < next.length; i++) {
        if (picked(next[i]!) && !picked(next[i - 1]!)) [next[i - 1], next[i]] = [next[i]!, next[i - 1]!];
      }
    } else {
      for (let i = next.length - 2; i >= 0; i--) {
        if (picked(next[i]!) && !picked(next[i + 1]!)) [next[i + 1], next[i]] = [next[i]!, next[i + 1]!];
      }
    }
    void commit(next);
  }

  const toTop = (moving: number[]) => placeBlock(moving, 0);
  const toBottom = (moving: number[]) => placeBlock(moving, ordered.length);

  function startEdit(mod: ModView) {
    setEditing(mod.id);
    setDraft(String(ids.indexOf(mod.id) + 1));
  }

  function finishEdit(apply: boolean) {
    const id = editing;
    setEditing(null);
    if (!apply || id === null) return;
    const wanted = Number.parseInt(draft, 10);
    if (!Number.isFinite(wanted)) return;
    if (wanted - 1 === ids.indexOf(id)) return;
    placeBlock(targetsOf(id), wanted - 1);
  }

  // ----- dragging --------------------------------------------------------

  function onPointerDown(event: React.PointerEvent, mod: ModView) {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button, input")) return;
    press.current = { id: mod.id, x: event.clientX, y: event.clientY };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function onPointerMove(event: React.PointerEvent, mod: ModView) {
    const start = press.current;
    if (!start || start.id !== mod.id) return;
    if (!drag) {
      if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD) return;
      // Dragging a row that is not picked moves just that row.
      const moving = selection.includes(mod.id) ? ids.filter((id) => selection.includes(id)) : [mod.id];
      if (!selection.includes(mod.id)) setSelection([mod.id]);
      setDrag({ ids: moving, gap: gapAt(event.clientY), x: event.clientX, y: event.clientY });
      return;
    }
    setDrag({ ...drag, gap: gapAt(event.clientY), x: event.clientX, y: event.clientY });

    // Near the top or bottom edge the list scrolls under the pointer.
    const box = scrollRef.current?.getBoundingClientRect();
    if (box && scrollRef.current) {
      if (event.clientY < box.top + 36) scrollRef.current.scrollTop -= 14;
      else if (event.clientY > box.bottom - 36) scrollRef.current.scrollTop += 14;
    }
  }

  function onPointerUp(event: React.PointerEvent, mod: ModView) {
    const start = press.current;
    press.current = null;
    if (!start || start.id !== mod.id) return;
    if (drag) {
      const index = indexAmongRest(drag.ids, drag.gap);
      const unchanged = ids.join(",") === placed(drag.ids, index).join(",");
      setDrag(null);
      if (!unchanged) placeBlock(drag.ids, index);
    } else {
      select(mod, event);
    }
  }

  /** The id order a block dropped at `index` among the rest would give. */
  function placed(moving: number[], index: number) {
    const rest = ids.filter((id) => !moving.includes(id));
    rest.splice(index, 0, ...ids.filter((id) => moving.includes(id)));
    return rest;
  }

  // Escape drops the drag, or clears the picked rows; Ctrl+A picks them all.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (document.querySelector('[role="dialog"]')) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, select")) return;
      if (event.key === "Escape") {
        if (drag) {
          press.current = null;
          setDrag(null);
        } else setSelection([]);
      } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
        event.preventDefault();
        setSelection(ids);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (ordered.length === 0) {
    return (
      <Empty
        icon={<ListOrdered size={20} />}
        title="Nothing to order yet"
        hint="Load order applies to loose pak mods in ~mods. Install one and it will show up here."
      />
    );
  }

  const picked = ids.filter((id) => selection.includes(id));
  const dragName = drag
    ? drag.ids.length === 1
      ? ordered.find((m) => m.id === drag.ids[0])?.name
      : `${drag.ids.length} mods`
    : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {misordered > 0 && (
        <div className="flex shrink-0 items-center gap-2 bg-surface px-5 py-2 text-[12px]">
          <AlertTriangle size={14} className="shrink-0 text-warn" />
          <span className="flex-1">
            The order breaks load order rules for {misordered} mod{misordered === 1 ? "" : "s"}.
          </span>
          <Button size="sm" variant="primary" onClick={() => void sortByRules()}>
            <ArrowDownUp size={12} /> Sort by rules
          </Button>
        </div>
      )}

      <div className="flex h-12 shrink-0 items-center gap-2 px-5">
        {picked.length > 0 ? (
          <>
            <span className="text-[13px]">{picked.length} selected</span>
            <Button size="sm" onClick={() => toTop(picked)} title="Move to the top">
              <ChevronsUp size={13} /> Top
            </Button>
            <Button size="sm" onClick={() => nudge(picked, -1)} title="Move up one place">
              <ArrowUp size={13} /> Up
            </Button>
            <Button size="sm" onClick={() => nudge(picked, 1)} title="Move down one place">
              <ArrowDown size={13} /> Down
            </Button>
            <Button size="sm" onClick={() => toBottom(picked)} title="Move to the bottom">
              <ChevronsDown size={13} /> Bottom
            </Button>
            <Button size="sm" onClick={() => setSelection([])} title="Clear the selection (Esc)">
              <X size={13} />
            </Button>
          </>
        ) : (
          <p className="text-[12px] text-ink-muted">
            Mods higher in this list load first, and a later mod wins where two change the same
            thing. Paks are renamed with a numeric prefix so the game mounts them in this order.
            Drag a row, or double-click its number to type a place.
          </p>
        )}
        <span className="flex-1" />
        {picked.length === 0 && (
          <Button size="sm" onClick={() => setSelection(ids)} title="Select every mod (Ctrl+A)">
            Select all
          </Button>
        )}
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
        <ul ref={listRef} className="relative">
          {ordered.map((mod, index) => {
            const isPicked = selection.includes(mod.id);
            const dragged = drag?.ids.includes(mod.id) ?? false;
            const acts = targetsOf(mod.id);
            return (
              <li
                key={mod.id}
                data-row
                onPointerDown={(event) => onPointerDown(event, mod)}
                onPointerMove={(event) => onPointerMove(event, mod)}
                onPointerUp={(event) => onPointerUp(event, mod)}
                onPointerCancel={() => {
                  press.current = null;
                  setDrag(null);
                }}
                className={clsx(
                  "group relative mb-1 flex h-10 touch-none items-center gap-2.5 rounded-sm px-2.5 transition-[background-color,opacity] duration-150 ease-quiet",
                  isPicked ? "bg-raised" : "bg-surface hover:bg-raised/60",
                  dragged && "opacity-40",
                  !mod.enabled && !dragged && "opacity-60",
                  drag ? "cursor-grabbing" : "cursor-default",
                )}
              >
                {/* The line that shows where the dragged block will land. */}
                {drag && drag.gap === index && !dragged && (
                  <span className="pointer-events-none absolute inset-x-0 -top-[3px] h-0.5 bg-ink" />
                )}
                {drag && drag.gap === ordered.length && index === ordered.length - 1 && (
                  <span className="pointer-events-none absolute inset-x-0 -bottom-[5px] h-0.5 bg-ink" />
                )}

                <GripVertical size={14} className="shrink-0 cursor-grab text-ink-muted" />

                {editing === mod.id ? (
                  <input
                    autoFocus
                    value={draft}
                    inputMode="numeric"
                    aria-label={`Position of ${mod.name}`}
                    onChange={(event) => setDraft(event.target.value.replace(/\D/g, ""))}
                    onFocus={(event) => event.target.select()}
                    onKeyDown={(event) => {
                      event.stopPropagation();
                      if (event.key === "Enter") finishEdit(true);
                      else if (event.key === "Escape") finishEdit(false);
                    }}
                    onBlur={() => finishEdit(false)}
                    className="h-6 w-12 shrink-0 rounded-sm bg-base px-1.5 text-center font-mono text-[11px] outline-none focus:ring-1 focus:ring-accent"
                  />
                ) : (
                  <button
                    onDoubleClick={() => startEdit(mod)}
                    title="Double-click to type a place"
                    className="w-12 shrink-0 text-left font-mono text-[11px] text-ink-muted tabular-nums transition-colors duration-150 ease-quiet hover:text-ink"
                  >
                    {String(index + 1).padStart(3, "0")}
                  </button>
                )}

                <span className="min-w-0 flex-1 truncate text-[13px]" title={mod.name}>
                  {mod.name}
                </span>
                {mod.misordered.length > 0 && (
                  <span className="truncate text-[11px] text-warn" title={mod.misordered.join("; ")}>
                    {mod.misordered[0]}
                  </span>
                )}
                {!mod.enabled && <span className="text-[11px] text-ink-muted">disabled</span>}

                <div className="flex shrink-0 items-center opacity-0 transition-opacity duration-150 ease-quiet group-hover:opacity-100 focus-within:opacity-100">
                  <Button
                    size="sm"
                    onClick={() => toTop(acts)}
                    disabled={index === 0 && acts.length === 1}
                    aria-label="Move to the top"
                    title="Move to the top"
                  >
                    <ChevronsUp size={13} />
                  </Button>
                  <Button
                    size="sm"
                    onClick={() => nudge(acts, -1)}
                    disabled={index === 0 && acts.length === 1}
                    aria-label="Move up"
                    title="Move up one place"
                  >
                    <ArrowUp size={13} />
                  </Button>
                  <Button
                    size="sm"
                    onClick={() => nudge(acts, 1)}
                    disabled={index === ordered.length - 1 && acts.length === 1}
                    aria-label="Move down"
                    title="Move down one place"
                  >
                    <ArrowDown size={13} />
                  </Button>
                  <Button
                    size="sm"
                    onClick={() => toBottom(acts)}
                    disabled={index === ordered.length - 1 && acts.length === 1}
                    aria-label="Move to the bottom"
                    title="Move to the bottom"
                  >
                    <ChevronsDown size={13} />
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      </div>

      {/* What is being dragged, following the pointer. */}
      {drag && dragName && (
        <div
          className="pointer-events-none fixed z-50 max-w-64 truncate bg-hover px-3 py-1.5 text-[12px]"
          style={{ left: drag.x + 14, top: drag.y + 10 }}
        >
          {dragName}
        </div>
      )}
    </div>
  );
}
