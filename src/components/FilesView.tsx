import { revealItemInDir } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  ChevronRight,
  File as FileIcon,
  Folder,
  FolderOpen,
  Home,
  Layers,
  Loader2,
  Package,
  PenLine,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { ipc, type VfsEntry, type VfsListing } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Empty } from "./ui";

/**
 * The virtual file tree, in the manner of Mod Organizer 2's "Data" tab.
 *
 * It shows the game folder as the enabled mods make it: each file attributed
 * to the mod that provides it, the game's own files where nothing touches
 * them, and — the reason the tab exists — which mods are hidden beneath the
 * winner. It reflects the current enabled set and load order whether or not
 * the virtual file system is the active deployment mode.
 */
export function FilesView() {
  const snapshot = useApp((s) => s.snapshot);
  const toast = useApp((s) => s.toast);
  const [dir, setDir] = useState("");
  const [listing, setListing] = useState<VfsListing | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(
    async (next: string) => {
      setLoading(true);
      try {
        const result = await ipc.vfsTree(next);
        setListing(result);
        setDir(result.dir);
      } catch (error) {
        toast("error", String(error));
      } finally {
        setLoading(false);
      }
    },
    [toast],
  );

  useEffect(() => {
    void load("");
    // Reload when the mod set or order changes, so the tree stays current.
  }, [load, snapshot?.mods, snapshot?.deploymentMode]);

  const crumbs = dir ? dir.split("/") : [];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-1 border-b border-line px-4 py-2.5 text-[12px]">
        <button
          onClick={() => void load("")}
          className="flex items-center gap-1 rounded px-1.5 py-0.5 text-ink-soft hover:bg-hover hover:text-ink"
        >
          <Home size={13} /> Game
        </button>
        {crumbs.map((part, index) => {
          const path = crumbs.slice(0, index + 1).join("/");
          const last = index === crumbs.length - 1;
          return (
            <span key={path} className="flex items-center gap-1">
              <ChevronRight size={12} className="text-ink-muted" />
              <button
                onClick={() => !last && void load(path)}
                className={clsx(
                  "rounded px-1.5 py-0.5",
                  last ? "text-ink" : "text-ink-soft hover:bg-hover hover:text-ink",
                )}
              >
                {part}
              </button>
            </span>
          );
        })}
        {loading && <Loader2 size={13} className="ml-1 animate-spin text-ink-muted" />}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {!snapshot?.game ? (
          <Empty
            icon={<Folder size={22} />}
            title="No game folder"
            hint="Choose the Stellar Blade folder in Settings."
          />
        ) : listing && listing.entries.length === 0 ? (
          <Empty
            icon={<Folder size={22} />}
            title="Nothing here"
            hint={dir ? "This folder is empty in the virtual view." : "No mods are enabled yet."}
          />
        ) : (
          <ul className="divide-y divide-line/60">
            {listing?.entries.map((entry) => (
              <Row
                key={entry.path}
                entry={entry}
                onOpen={() => entry.isDir && void load(entry.path)}
                onReveal={() => entry.source && void revealItemInDir(entry.source)}
              />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function Row({
  entry,
  onOpen,
  onReveal,
}: {
  entry: VfsEntry;
  onOpen: () => void;
  onReveal: () => void;
}) {
  const conflicted = entry.shadowedBy.length > 0;
  const Icon = entry.isDir ? Folder : FileIcon;

  return (
    <li
      className={clsx(
        "flex items-center gap-2.5 px-4 py-1.5 text-[13px]",
        entry.isDir ? "cursor-pointer hover:bg-hover/60" : "hover:bg-hover/40",
      )}
      onClick={entry.isDir ? onOpen : undefined}
    >
      <Icon size={15} className={clsx("shrink-0", entry.isDir ? "text-accent-soft" : "text-ink-muted")} />
      <span className="min-w-0 flex-1 truncate">{entry.name}</span>

      {!entry.isDir && <OriginTag entry={entry} />}
      {conflicted && (
        <Badge tone="warn" title={`Hides: ${entry.shadowedBy.join(", ")}`}>
          +{entry.shadowedBy.length} hidden
        </Badge>
      )}
      {!entry.isDir && entry.source && (
        <button
          onClick={(event) => {
            event.stopPropagation();
            onReveal();
          }}
          title="Show the real file"
          className="rounded p-1 text-ink-muted hover:bg-hover hover:text-ink"
        >
          <FolderOpen size={13} />
        </button>
      )}
    </li>
  );
}

/** Who provides the winning version, colour-coded by kind. */
function OriginTag({ entry }: { entry: VfsEntry }) {
  if (entry.origin === "game") {
    return <span className="shrink-0 text-[11px] text-ink-muted">Game</span>;
  }
  const Icon = entry.origin === "overwrite" ? PenLine : entry.origin === "generated" ? Layers : Package;
  return (
    <span className="flex shrink-0 items-center gap-1 text-[11px] text-ink-soft">
      <Icon size={11} className="text-accent-soft" />
      <span className="max-w-40 truncate">{entry.providedBy}</span>
    </span>
  );
}
