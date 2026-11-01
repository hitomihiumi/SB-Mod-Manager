import clsx from "clsx";
import {
  ArrowUp,
  ExternalLink,
  File as FileIcon,
  Folder,
  FolderOpen,
  FolderPlus,
  Loader2,
  Package,
  Pencil,
  Scissors,
  ClipboardPaste,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { formatBytes } from "../../lib/format";
import { ipc, type LibraryChange, type LibraryEntry, type LibraryListing } from "../../lib/ipc";
import { useApp } from "../../store/useApp";
import { parentDir, useExplorer, type ExplorerTab } from "../../store/useExplorer";
import { askText, isAsking } from "../Ask";
import { openContextMenu, type MenuItem } from "../ContextMenu";
import { Button, Empty } from "../ui";
import { fileMenu, openFile, reveal } from "./fileMenu";
import { Crumbs, formatWhen, RowButton } from "./shared";

const DRAG_THRESHOLD = 5;

/**
 * The mod library as it is on disk: one folder per mod, and inside each the
 * files the manager lays into the game. Here they can be moved between
 * folders (by dragging, or Cut and Paste), renamed and opened in other
 * programs. Moving a mod's files changes where they end up in the game, so the
 * mods that were touched are detected again afterwards.
 */
export function LibraryPane({ tab }: { tab: ExplorerTab }) {
  const dir = tab.dir;
  const toast = useApp((s) => s.toast);
  const redetect = useApp((s) => s.redetect);
  const mods = useApp((s) => s.snapshot?.mods);
  const show = useExplorer((s) => s.show);
  const openTab = useExplorer((s) => s.openTab);
  const clipboard = useExplorer((s) => s.clipboard);
  const setClipboard = useExplorer((s) => s.setClipboard);
  const drop = useExplorer((s) => s.drop);
  const setDrop = useExplorer((s) => s.setDrop);

  const [listing, setListing] = useState<LibraryListing | null>(null);
  const [loading, setLoading] = useState(false);
  const [selection, setSelection] = useState<string[]>([]);
  const [anchor, setAnchor] = useState<string | null>(null);
  const [drag, setDrag] = useState<{ paths: string[]; x: number; y: number } | null>(null);
  const press = useRef<{ path: string; x: number; y: number } | null>(null);

  const entries = listing?.entries ?? [];
  const atRoot = dir === "";
  const go = useCallback((next: string) => show(tab.id, "library", next), [show, tab.id]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setListing(await ipc.libraryList(dir));
    } catch (error) {
      toast("error", String(error));
      // A folder that has gone is not worth staying in.
      if (dir) go(parentDir(dir));
    } finally {
      setLoading(false);
    }
  }, [dir, go, toast]);

  useEffect(() => {
    void load();
  }, [load, mods]);

  // A new folder starts with nothing picked.
  useEffect(() => {
    setSelection([]);
    setAnchor(null);
  }, [dir]);

  // ----- doing things ----------------------------------------------------

  /** After files changed place: say what was left, reload, and re-detect the mods they belong to. */
  async function settle(change: LibraryChange) {
    if (change.skipped.length > 0) {
      const shown = change.skipped.slice(0, 3).join("; ");
      toast("error", `${shown}${change.skipped.length > 3 ? `; and ${change.skipped.length - 3} more` : ""}.`);
    }
    await load();
    if (change.moved > 0 && change.modIds.length > 0) await redetect(change.modIds);
  }

  async function move(paths: string[], toDir: string) {
    try {
      setSelection([]);
      await settle(await ipc.libraryMove(paths, toDir));
    } catch (error) {
      toast("error", String(error));
    }
  }

  async function paste() {
    if (clipboard.length === 0 || atRoot) return;
    const paths = clipboard;
    setClipboard([]);
    await move(paths, dir);
  }

  async function rename(entry: LibraryEntry) {
    if (atRoot) return;
    const name = await askText({
      title: "Rename",
      label: entry.isDir ? "Folder name" : "File name",
      initial: entry.name,
      confirmLabel: "Rename",
    });
    if (name === null || name.trim() === entry.name) return;
    try {
      await settle(await ipc.libraryRename(entry.path, name));
      setSelection([]);
    } catch (error) {
      toast("error", String(error));
    }
  }

  async function newFolder() {
    if (atRoot) return;
    const name = await askText({ title: "New folder", label: "Name", confirmLabel: "Create" });
    if (name === null || name.trim() === "") return;
    try {
      await ipc.libraryMkdir(dir, name);
      await load();
    } catch (error) {
      toast("error", String(error));
    }
  }

  const cut = (paths: string[]) => {
    if (atRoot || paths.length === 0) return;
    setClipboard(paths);
  };

  function openEntry(entry: LibraryEntry, newTab = false) {
    if (entry.isDir) {
      if (newTab) openTab("library", entry.path);
      else go(entry.path);
    } else void openFile(entry.absolute);
  }

  // ----- selection -------------------------------------------------------

  const paths = entries.map((e) => e.path);

  function select(entry: LibraryEntry, event: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) {
    if (event.shiftKey && anchor !== null) {
      const from = paths.indexOf(anchor);
      const to = paths.indexOf(entry.path);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from < to ? [from, to] : [to, from];
        setSelection(paths.slice(lo, hi + 1));
        return;
      }
    }
    if (event.ctrlKey || event.metaKey) {
      setSelection((c) => (c.includes(entry.path) ? c.filter((p) => p !== entry.path) : [...c, entry.path]));
    } else {
      setSelection([entry.path]);
    }
    setAnchor(entry.path);
  }

  // ----- dragging --------------------------------------------------------

  /** The folder under the pointer that takes a drop, unless it is part of what is dragged. */
  function dropAt(x: number, y: number, dragged: string[]) {
    const element = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-dir]");
    const target = element?.dataset.dropDir;
    const key = element?.dataset.dropKey;
    if (!element || target === undefined || key === undefined) return null;
    if (dragged.some((p) => target === p || target.startsWith(`${p}/`))) return null;
    return { dir: target, key };
  }

  function onPointerDown(event: React.PointerEvent, entry: LibraryEntry) {
    if (event.button !== 0 || (event.target as HTMLElement).closest("button, input")) return;
    press.current = { path: entry.path, x: event.clientX, y: event.clientY };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function onPointerMove(event: React.PointerEvent, entry: LibraryEntry) {
    const start = press.current;
    if (!start || start.path !== entry.path) return;
    if (!drag) {
      // The top level holds the mods themselves, which stay where they are.
      if (atRoot || Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD) return;
      const moving = selection.includes(entry.path) ? paths.filter((p) => selection.includes(p)) : [entry.path];
      if (!selection.includes(entry.path)) setSelection([entry.path]);
      setDrag({ paths: moving, x: event.clientX, y: event.clientY });
      return;
    }
    setDrag({ ...drag, x: event.clientX, y: event.clientY });
    const over = dropAt(event.clientX, event.clientY, drag.paths);
    if (over?.key !== useExplorer.getState().drop?.key) setDrop(over);
  }

  function endDrag(commit: boolean, event?: React.PointerEvent) {
    const start = press.current;
    press.current = null;
    const target = useExplorer.getState().drop;
    const moving = drag?.paths;
    setDrag(null);
    setDrop(null);
    if (commit && moving && target && target.dir !== dir) void move(moving, target.dir);
    else if (commit && !moving && start && event) {
      const entry = entries.find((e) => e.path === start.path);
      if (entry) select(entry, event);
    }
  }

  // ----- keyboard --------------------------------------------------------

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isAsking() || document.querySelector('[role="dialog"], [role="menu"]')) return;
      if ((event.target as HTMLElement | null)?.closest("input, textarea, select")) return;
      const ctrl = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      const picked = entries.filter((e) => selection.includes(e.path));

      if (event.key === "Escape") {
        if (drag) {
          press.current = null;
          setDrag(null);
          setDrop(null);
        } else if (selection.length > 0) setSelection([]);
        else if (clipboard.length > 0) setClipboard([]);
        else return;
      } else if (ctrl && key === "a") setSelection(paths);
      else if (ctrl && key === "x") cut(picked.map((e) => e.path));
      else if (ctrl && key === "v") void paste();
      else if (event.key === "F2" && picked.length === 1) void rename(picked[0]!);
      else if (event.key === "Enter" && picked.length === 1) openEntry(picked[0]!);
      else if (event.key === "Backspace" && dir) go(parentDir(dir));
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ----- menus -----------------------------------------------------------

  function menuFor(entry: LibraryEntry): MenuItem[] {
    const picked = selection.includes(entry.path) ? entries.filter((e) => selection.includes(e.path)) : [entry];
    const many = picked.length > 1;
    const edit: MenuItem[] = atRoot
      ? []
      : [
          "separator",
          {
            label: many ? `Cut ${picked.length} items` : "Cut",
            icon: <Scissors size={13} />,
            shortcut: "Ctrl+X",
            onSelect: () => cut(picked.map((e) => e.path)),
          },
          ...(many
            ? []
            : [
                {
                  label: "Rename",
                  icon: <Pencil size={13} />,
                  shortcut: "F2",
                  onSelect: () => void rename(entry),
                } as MenuItem,
              ]),
        ];
    if (many) return edit.slice(1);
    if (entry.isDir) {
      return [
        { label: "Open", icon: <Folder size={13} />, onSelect: () => go(entry.path) },
        { label: "Open in a new tab", icon: <ExternalLink size={13} />, onSelect: () => openTab("library", entry.path) },
        { label: "Show in folder", icon: <FolderOpen size={13} />, onSelect: () => reveal(entry.absolute) },
        ...edit,
      ];
    }
    return [...fileMenu(entry.absolute), ...edit];
  }

  function menuForBackground(): MenuItem[] {
    if (atRoot) return [];
    return [
      { label: "New folder", icon: <FolderPlus size={13} />, onSelect: () => void newFolder() },
      {
        label: clipboard.length > 0 ? `Paste ${clipboard.length} here` : "Paste",
        icon: <ClipboardPaste size={13} />,
        shortcut: "Ctrl+V",
        disabled: clipboard.length === 0,
        onSelect: () => void paste(),
      },
    ];
  }

  const dragLabel = drag
    ? drag.paths.length === 1
      ? (entries.find((e) => e.path === drag.paths[0])?.name ?? "1 item")
      : `${drag.paths.length} items`
    : null;

  return (
    <>
      <div className="flex shrink-0 items-center gap-2 px-5 py-1.5">
        <Crumbs
          root="Library"
          dir={dir}
          nameOf={(index, name) => (index === 0 && listing?.modName ? listing.modName : name)}
          onGo={go}
          droppable
          target={drop?.key ?? null}
        />
        {loading && <Loader2 size={13} className="animate-spin text-ink-muted" />}
        <span className="flex-1" />
        {!atRoot && (
          <>
            {clipboard.length > 0 && (
              <Button size="sm" variant="primary" onClick={() => void paste()} title="Move the cut items into this folder (Ctrl+V)">
                <ClipboardPaste size={13} /> Paste {clipboard.length} here
              </Button>
            )}
            {selection.length > 0 && (
              <Button size="sm" onClick={() => cut(selection)} title="Cut the selected items (Ctrl+X)">
                <Scissors size={13} /> Cut
              </Button>
            )}
            <Button size="sm" onClick={() => void newFolder()}>
              <FolderPlus size={13} /> New folder
            </Button>
            <Button size="sm" onClick={() => go(parentDir(dir))} title="Up one folder (Backspace)">
              <ArrowUp size={13} />
            </Button>
          </>
        )}
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto"
        onContextMenu={(event) => {
          const items = menuForBackground();
          if (items.length > 0) openContextMenu(event, items);
        }}
        onPointerDown={(event) => {
          // A press on empty space lets go of the selection.
          if (event.target === event.currentTarget) setSelection([]);
        }}
      >
        {listing && entries.length === 0 ? (
          <Empty
            icon={<Folder size={22} />}
            title={atRoot ? "No mods installed" : "This folder is empty"}
            hint={
              atRoot
                ? "Mods appear here once they are installed."
                : "Drag files here from another tab, or Cut them and Paste here."
            }
          />
        ) : (
          <ul className="touch-none">
            {entries.map((entry) => {
              const picked = selection.includes(entry.path);
              const dragged = drag?.paths.includes(entry.path) ?? false;
              const cutOut = clipboard.includes(entry.path);
              const over = drop?.key === `row-${entry.path}`;
              return (
                <li
                  key={entry.path}
                  data-drop-dir={entry.isDir && !dragged ? entry.path : undefined}
                  data-drop-key={entry.isDir && !dragged ? `row-${entry.path}` : undefined}
                  onPointerDown={(event) => onPointerDown(event, entry)}
                  onPointerMove={(event) => onPointerMove(event, entry)}
                  onPointerUp={(event) => endDrag(true, event)}
                  onPointerCancel={() => endDrag(false)}
                  onDoubleClick={() => openEntry(entry)}
                  onAuxClick={(event) => event.button === 1 && openEntry(entry, true)}
                  onContextMenu={(event) => {
                    if (!picked) setSelection([entry.path]);
                    openContextMenu(event, menuFor(entry));
                  }}
                  className={clsx(
                    "group flex h-9 items-center gap-2.5 px-5 text-[13px] transition-[background-color,opacity] duration-150 ease-quiet",
                    over ? "bg-hover" : picked ? "bg-raised" : "hover:bg-surface",
                    (dragged || cutOut) && "opacity-45",
                  )}
                >
                  {atRoot ? (
                    <Package size={15} className="shrink-0 text-ink-soft" />
                  ) : entry.isDir ? (
                    <Folder size={15} className="shrink-0 text-ink-soft" />
                  ) : (
                    <FileIcon size={15} className="shrink-0 text-ink-muted" />
                  )}
                  <span className="min-w-0 flex-1 truncate" title={entry.name}>
                    {entry.label ?? entry.name}
                  </span>
                  {entry.label && entry.label !== entry.name && (
                    <span className="shrink-0 font-mono text-[11px] text-ink-muted">{entry.name}</span>
                  )}
                  <RowButton label={entry.isDir ? "Open" : "Open in the default program"} onClick={() => openEntry(entry)}>
                    <ExternalLink size={13} />
                  </RowButton>
                  <RowButton label="Show in folder" onClick={() => reveal(entry.absolute)}>
                    <FolderOpen size={13} />
                  </RowButton>
                  <span className="w-20 shrink-0 text-right font-mono text-[11px] text-ink-muted tabular-nums">
                    {entry.isDir ? "" : formatBytes(entry.size)}
                  </span>
                  <span className="w-28 shrink-0 text-right text-[11px] text-ink-muted">
                    {formatWhen(entry.modified)}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {drag && dragLabel && (
        <div
          className="pointer-events-none fixed z-50 max-w-64 truncate bg-hover px-3 py-1.5 text-[12px]"
          style={{ left: drag.x + 14, top: drag.y + 10 }}
        >
          {dragLabel}
        </div>
      )}
    </>
  );
}
