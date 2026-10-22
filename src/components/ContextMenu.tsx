import clsx from "clsx";
import { ChevronRight } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { create } from "zustand";

/** One line of a context menu. */
export type MenuItem =
  | {
      label: string;
      icon?: ReactNode;
      /** Shown on the right, e.g. "F2". */
      shortcut?: string;
      danger?: boolean;
      disabled?: boolean;
      onSelect?: () => void;
      /** A submenu instead of an action. */
      items?: MenuItem[];
    }
  | "separator";

const useMenu = create<{ at: { x: number; y: number } | null; items: MenuItem[] }>(() => ({
  at: null,
  items: [],
}));

/** Open a menu at the pointer of `event`. */
export function openContextMenu(event: React.MouseEvent, items: MenuItem[]) {
  event.preventDefault();
  event.stopPropagation();
  useMenu.setState({ at: { x: event.clientX, y: event.clientY }, items });
}

export function closeContextMenu() {
  useMenu.setState({ at: null, items: [] });
}

/** Rendered once, near the root. */
export function ContextMenuHost() {
  const at = useMenu((s) => s.at);
  const items = useMenu((s) => s.items);

  useEffect(() => {
    if (!at) return;
    const close = () => closeContextMenu();
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && close();
    window.addEventListener("mousedown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("wheel", close, { passive: true });
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("wheel", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [at]);

  if (!at) return null;
  return <Panel items={items} x={at.x} y={at.y} />;
}

/** A menu panel, kept inside the window. */
function Panel({ items, x, y, side }: { items: MenuItem[]; x: number; y: number; side?: number }) {
  const ref = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState({ left: x, top: y });
  const [open, setOpen] = useState<number | null>(null);

  useLayoutEffect(() => {
    const box = ref.current?.getBoundingClientRect();
    if (!box) return;
    let left = x;
    let top = y;
    if (left + box.width > window.innerWidth - 4) {
      // A submenu flips to the other side of its parent; a root menu shifts.
      left = side !== undefined ? side - box.width : window.innerWidth - box.width - 4;
    }
    if (top + box.height > window.innerHeight - 4) top = window.innerHeight - box.height - 4;
    setPlace({ left: Math.max(4, left), top: Math.max(4, top) });
  }, [x, y, side]);

  return (
    <div
      ref={ref}
      role="menu"
      onMouseDown={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
      className="fixed z-[60] min-w-48 rounded-lg border border-line-strong bg-raised p-1 shadow-2xl"
      style={place}
    >
      {items.map((item, index) =>
        item === "separator" ? (
          <div key={`sep-${index}`} className="my-1 h-px bg-line" />
        ) : (
          <div key={item.label} className="relative" onMouseEnter={() => setOpen(index)}>
            <button
              role="menuitem"
              disabled={item.disabled}
              onClick={() => {
                if (item.items) return;
                closeContextMenu();
                item.onSelect?.();
              }}
              className={clsx(
                "flex h-7 w-full items-center gap-2 rounded px-2 text-left text-[12px] disabled:opacity-40",
                item.danger ? "text-danger hover:bg-danger/10" : "hover:bg-hover",
                open === index && item.items && "bg-hover",
              )}
            >
              <span className="grid w-4 shrink-0 place-items-center text-ink-muted">{item.icon}</span>
              <span className="min-w-0 flex-1 truncate">{item.label}</span>
              {item.shortcut && (
                <span className="text-[10px] text-ink-muted">{item.shortcut}</span>
              )}
              {item.items && <ChevronRight size={12} className="text-ink-muted" />}
            </button>
            {item.items && open === index && (
              <SubPanel items={item.items} />
            )}
          </div>
        ),
      )}
    </div>
  );
}

/** A submenu beside its line. */
function SubPanel({ items }: { items: MenuItem[] }) {
  const anchor = useRef<HTMLSpanElement>(null);
  const [at, setAt] = useState<{ x: number; y: number; side: number } | null>(null);
  useLayoutEffect(() => {
    const line = anchor.current?.parentElement?.getBoundingClientRect();
    if (line) setAt({ x: line.right + 2, y: line.top - 4, side: line.left - 2 });
  }, []);
  return (
    <span ref={anchor}>
      {at && <Panel items={items} x={at.x} y={at.y} side={at.side} />}
    </span>
  );
}
