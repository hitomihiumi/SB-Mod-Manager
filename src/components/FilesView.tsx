import { revealItemInDir } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  ChevronRight,
  File as FileIcon,
  Folder,
  FolderOpen,
  FolderTree,
  Home,
  Layers,
  Loader2,
  Package,
  PenLine,
  Swords,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { ipc, type VfsEntry, type VfsListing } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Empty } from "./ui";

type Mode = "browse" | "conflicts";

/**
 * The virtual file tree, in the manner of Mod Organizer 2's "Data" tab.
 *
 * Browse shows the game folder as the enabled mods make it: each file
 * attributed to the mod that provides it, the game's own files where nothing
 * touches them, and which mods are hidden beneath the winner. Conflicts lists
 * every clashing path flat — all of them, or only one mod's when arriving from
 * that mod's flag in the mod list. Both reflect the current enabled set and
 * load order whether or not the virtual file system is the deployment mode.
 */
export function FilesView() {
  const snapshot = useApp((s) => s.snapshot);
  const toast = useApp((s) => s.toast);
  const focus = useApp((s) => s.conflictFocus);
  const setFocus = useApp((s) => s.setConflictFocus);
  const [mode, setMode] = useState<Mode>(focus ? "conflicts" : "browse");
  const [dir, setDir] = useState("");
  const [listing, setListing] = useState<VfsListing | null>(null);
  const [conflicts, setConflicts] = useState<VfsEntry[] | null>(null);
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

  const loadConflicts = useCallback(
    async (modId: number | null) => {
      setLoading(true);
      try {
        setConflicts(await ipc.vfsConflicts(modId));
      } catch (error) {
        toast("error", String(error));
      } finally {
        setLoading(false);
      }
    },
    [toast],
  );

  // Arriving from a mod's flag switches to that mod's conflicts.
  useEffect(() => {
    if (focus) setMode("conflicts");
  }, [focus]);

  // Reload when the mod set or order changes, so the view stays current.
  useEffect(() => {
    if (mode === "browse") void load(dir);
    else void loadConflicts(focus?.modId ?? null);
    // `dir` is left out on purpose: navigating inside the tree calls `load`
    // itself, and this effect only follows mode and mod-list changes.
  }, [mode, focus, load, loadConflicts, snapshot?.mods, snapshot?.deploymentMode]);

  const crumbs = dir ? dir.split("/") : [];

  /** Show a conflicting file where it lives in the tree. */
  function browseTo(path: string) {
    // Both in one render, so the mode effect loads the new folder once.
    setDir(path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "");
    setMode("browse");
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-1 border-b border-line px-3 py-2 text-[12px]">
        <div className="mr-2 flex rounded-md border border-line p-0.5">
          {(
            [
              ["browse", "Browse", FolderTree],
              ["conflicts", "Conflicts", Swords],
            ] as const
          ).map(([id, label, Icon]) => (
            <button
              key={id}
              onClick={() => setMode(id)}
              aria-pressed={mode === id}
              className={clsx(
                "flex items-center gap-1 rounded px-2 py-0.5",
                mode === id ? "bg-hover text-ink" : "text-ink-muted hover:text-ink",
              )}
            >
              <Icon size={12} /> {label}
            </button>
          ))}
        </div>

        {mode === "browse" ? (
          <>
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
          </>
        ) : focus ? (
          <span className="flex items-center gap-1 rounded-md bg-hover px-2 py-0.5 text-ink-soft">
            Only “{focus.name}”
            <button
              onClick={() => setFocus(null)}
              title="Show every conflict"
              className="text-ink-muted hover:text-ink"
            >
              <X size={12} />
            </button>
          </span>
        ) : (
          <span className="text-ink-muted">Every path more than one provider claims</span>
        )}
        {loading && <Loader2 size={13} className="ml-1 animate-spin text-ink-muted" />}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {!snapshot?.game ? (
          <Empty
            icon={<Folder size={22} />}
            title="No game folder"
            hint="Choose the Stellar Blade folder in Settings."
          />
        ) : mode === "conflicts" ? (
          conflicts && conflicts.length === 0 ? (
            <Empty
              icon={<Swords size={22} />}
              title="No file conflicts"
              hint={
                focus
                  ? `No other enabled mod provides any of “${focus.name}”'s files.`
                  : "No two enabled mods provide the same file. Clashes inside paks are on the Conflicts tab."
              }
            />
          ) : (
            <ul className="divide-y divide-line/60">
              {conflicts?.map((entry) => (
                <ConflictRow
                  key={entry.path}
                  entry={entry}
                  focusName={focus?.name ?? null}
                  onBrowse={() => browseTo(entry.path)}
                  onReveal={() => entry.source && void revealItemInDir(entry.source)}
                />
              ))}
            </ul>
          )
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

/** One clashing path: where it is, who wins it, and who is hidden. */
function ConflictRow({
  entry,
  focusName,
  onBrowse,
  onReveal,
}: {
  entry: VfsEntry;
  focusName: string | null;
  onBrowse: () => void;
  onReveal: () => void;
}) {
  const folder = entry.path.includes("/") ? entry.path.slice(0, entry.path.lastIndexOf("/") + 1) : "";
  // From a mod's flag, say plainly whether that mod wins this one.
  const focusLoses = focusName !== null && entry.providedBy !== focusName;

  return (
    <li
      onClick={onBrowse}
      title="Show in the tree"
      className="flex cursor-pointer items-center gap-2.5 px-4 py-2 text-[13px] hover:bg-hover/40"
    >
      <FileIcon size={15} className="shrink-0 text-ink-muted" />
      <span className="min-w-0 flex-1 truncate">
        <span className="text-ink-muted">{folder}</span>
        {entry.name}
      </span>
      {focusName !== null && (
        <Badge tone={focusLoses ? "warn" : "accent"}>{focusLoses ? "Loses" : "Wins"}</Badge>
      )}
      <OriginTag entry={entry} />
      <span className="max-w-60 shrink-0 truncate text-[11px] text-ink-muted" title={entry.shadowedBy.join(", ")}>
        hides {entry.shadowedBy.join(", ")}
      </span>
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
    </li>
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
