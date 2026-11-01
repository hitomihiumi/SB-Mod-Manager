import clsx from "clsx";
import { ChevronRight } from "lucide-react";
import type { ReactNode } from "react";

/** The folders of a `/`-separated path, with the path up to and including each. */
export function crumbsOf(dir: string): { name: string; path: string }[] {
  const parts = dir ? dir.split("/") : [];
  return parts.map((name, index) => ({ name, path: parts.slice(0, index + 1).join("/") }));
}

/**
 * Where you are, as a row of folders that can be clicked. In the library the
 * folders also take a drop, so a file can be dragged to any folder above it.
 */
export function Crumbs({
  root,
  dir,
  nameOf,
  onGo,
  droppable = false,
  target = null,
}: {
  root: ReactNode;
  dir: string;
  /** What to call the folder at `index`, when it is not what the disk says. */
  nameOf?: (index: number, name: string) => string;
  onGo: (path: string) => void;
  droppable?: boolean;
  /** The element a drag is currently over (its `data-drop-key`). */
  target?: string | null;
}) {
  const crumbs = crumbsOf(dir);
  const cls = (active: boolean, last = false) =>
    clsx(
      "rounded-sm px-1.5 py-0.5 transition-colors duration-150 ease-quiet",
      last ? "text-ink" : "text-ink-soft hover:text-ink",
      active && "bg-hover text-ink",
    );
  return (
    <nav aria-label="Path" className="flex min-w-0 items-center gap-0.5 text-[12px]">
      <button onClick={() => onGo("")} className={cls(false, crumbs.length === 0)}>
        {root}
      </button>
      {crumbs.map((crumb, index) => {
        const last = index === crumbs.length - 1;
        return (
          <span key={crumb.path} className="flex min-w-0 items-center gap-0.5">
            <ChevronRight size={12} className="shrink-0 text-ink-muted" />
            <button
              onClick={() => !last && onGo(crumb.path)}
              data-drop-dir={droppable ? crumb.path : undefined}
              data-drop-key={droppable ? `crumb-${crumb.path}` : undefined}
              className={clsx(cls(target === `crumb-${crumb.path}`, last), "truncate")}
            >
              {nameOf ? nameOf(index, crumb.name) : crumb.name}
            </button>
          </span>
        );
      })}
    </nav>
  );
}

/** A small button for a row, shown while the row is hovered or has focus. */
export function RowButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
      title={label}
      aria-label={label}
      className="rounded-sm p-1 text-ink-muted opacity-0 transition-[opacity,color] duration-150 ease-quiet group-hover:opacity-100 hover:text-ink focus-visible:opacity-100"
    >
      {children}
    </button>
  );
}

/** A modification time in the user's own date format. */
export function formatWhen(seconds: number | null): string {
  if (seconds === null) return "";
  return new Date(seconds * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/**
 * The library path of a real file, when it lies in the mods folder — which is
 * what lets a file seen in the game tree be found again among the real ones.
 */
export function pathInLibrary(source: string, modsDir: string | null): string | null {
  if (!modsDir) return null;
  const flat = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
  const file = flat(source);
  const root = flat(modsDir);
  if (!file.toLowerCase().startsWith(root.toLowerCase() + "/")) return null;
  return file.slice(root.length + 1);
}
