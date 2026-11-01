import clsx from "clsx";
import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ClipboardPaste,
  Copy,
  CopyPlus,
  ExternalLink,
  Eye,
  EyeOff,
  File as FileIcon,
  FileImage,
  FileText,
  Folder,
  FolderOpen,
  FolderPlus,
  Loader2,
  Package,
  Pencil,
  Scissors,
  Search,
  Trash2,
  Undo2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { formatBytes } from "../../lib/format";
import {
  ipc,
  type LibraryChange,
  type LibraryEntry,
  type LibraryListing,
  type LibraryPreview,
} from "../../lib/ipc";
import { useApp } from "../../store/useApp";
import { parentDir, useExplorer, type ExplorerTab } from "../../store/useExplorer";
import { askConfirm, askText, isAsking } from "../Ask";
import { openContextMenu, type MenuItem } from "../ContextMenu";
import { Button, Empty } from "../ui";
import { fileMenu, openFile, reveal } from "./fileMenu";
import { Crumbs, formatWhen, RowButton } from "./shared";

const DRAG_THRESHOLD = 5;
const PREVIEW_KEY = "sbmm.explorer.preview";

type SortKey = "name" | "type" | "size" | "modified";

const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "webp", "gif", "bmp", "dds", "tga"]);
const TEXT_EXT = new Set(["txt", "md", "ini", "cfg", "json", "lua", "toml", "yaml", "yml", "xml", "log"]);

const extOf = (name: string) => {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
};

function typeOf(entry: LibraryEntry): string {
  if (entry.isDir) return "Folder";
  const ext = extOf(entry.name);
  return ext ? ext.toUpperCase() : "File";
}

function iconOf(entry: LibraryEntry, atRoot: boolean) {
  if (atRoot) return <Package size={15} className="shrink-0 text-ink-soft" />;
  if (entry.isDir) return <Folder size={15} className="shrink-0 text-ink-soft" />;
  const ext = extOf(entry.name);
  if (IMAGE_EXT.has(ext)) return <FileImage size={15} className="shrink-0 text-ink-muted" />;
  if (TEXT_EXT.has(ext)) return <FileText size={15} className="shrink-0 text-ink-muted" />;
  return <FileIcon size={15} className="shrink-0 text-ink-muted" />;
}

function readPreviewSetting(): boolean {
  try {
    return localStorage.getItem(PREVIEW_KEY) !== "off";
  } catch {
    return true;
  }
}

/**
 * The mod library as it is on disk: one folder per mod, and inside each the
 * files the manager lays into the game. Here they can be moved and copied
 * between folders (dragging, with Ctrl to copy, or Cut/Copy and Paste),
 * renamed in place, deleted — into a trash the last delete can be taken back
 * from — brought in from the system's file manager by dropping them, and
 * looked at in the preview beside the list. Changing a mod's files changes
 * where they end up in the game, so the mods that were touched are detected
 * again afterwards.
 */
export function LibraryPane({ tab }: { tab: ExplorerTab }) {
  const dir = tab.dir;
  const toast = useApp((s) => s.toast);
  const redetect = useApp((s) => s.redetect);
  const mods = useApp((s) => s.snapshot?.mods);
  const navigate = useExplorer((s) => s.navigate);
  const goBack = useExplorer((s) => s.goBack);
  const goForward = useExplorer((s) => s.goForward);
  const openTab = useExplorer((s) => s.openTab);
  const clipboard = useExplorer((s) => s.clipboard);
  const clipMode = useExplorer((s) => s.clipMode);
  const setClipboard = useExplorer((s) => s.setClipboard);
  const drop = useExplorer((s) => s.drop);
  const setDrop = useExplorer((s) => s.setDrop);
  const revision = useExplorer((s) => s.revision);
  const modsDir = useExplorer((s) => s.modsDir);

  const [listing, setListing] = useState<LibraryListing | null>(null);
  const [loading, setLoading] = useState(false);
  const [selection, setSelection] = useState<string[]>([]);
  const [anchor, setAnchor] = useState<string | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [drag, setDrag] = useState<{ paths: string[]; x: number; y: number; copy: boolean } | null>(null);
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<{ key: SortKey; descending: boolean }>({ key: "name", descending: false });
  const [renaming, setRenaming] = useState<string | null>(null);
  const [showPreview, setShowPreview] = useState(readPreviewSetting);
  const [undo, setUndo] = useState<{ id: string; count: number } | null>(null);
  const press = useRef<{ path: string; x: number; y: number } | null>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const atRoot = dir === "";
  const go = useCallback((next: string) => navigate(tab.id, next), [navigate, tab.id]);

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
  }, [load, mods, revision]);

  // A new folder starts with nothing picked and no filter.
  useEffect(() => {
    setSelection([]);
    setAnchor(null);
    setCursor(null);
    setFilter("");
    setRenaming(null);
  }, [dir]);

  // What is shown: filtered, then sorted, folders always first.
  const entries = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const all = (listing?.entries ?? []).filter(
      (e) => !needle || e.name.toLowerCase().includes(needle) || (e.label ?? "").toLowerCase().includes(needle),
    );
    const name = (e: LibraryEntry) => (e.label ?? e.name).toLowerCase();
    const by = (a: LibraryEntry, b: LibraryEntry): number => {
      switch (sort.key) {
        case "type":
          return typeOf(a).localeCompare(typeOf(b)) || name(a).localeCompare(name(b));
        case "size":
          return a.size - b.size || name(a).localeCompare(name(b));
        case "modified":
          return (a.modified ?? 0) - (b.modified ?? 0) || name(a).localeCompare(name(b));
        default:
          return name(a).localeCompare(name(b), undefined, { numeric: true });
      }
    };
    return [...all].sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      const order = by(a, b);
      return sort.descending ? -order : order;
    });
  }, [listing, filter, sort]);

  const paths = entries.map((e) => e.path);
  const picked = entries.filter((e) => selection.includes(e.path));
  const single = picked.length === 1 ? picked[0]! : null;

  // ----- doing things ----------------------------------------------------

  /** After files changed: say what was left, reload, and re-detect the mods they belong to. */
  const settle = useCallback(
    async (change: LibraryChange) => {
      if (change.skipped.length > 0) {
        const shown = change.skipped.slice(0, 3).join("; ");
        toast("error", `${shown}${change.skipped.length > 3 ? `; and ${change.skipped.length - 3} more` : ""}.`);
      }
      await load();
      if (change.moved > 0 && change.modIds.length > 0) await redetect(change.modIds);
    },
    [load, redetect, toast],
  );

  async function run(action: () => Promise<LibraryChange>) {
    try {
      await settle(await action());
    } catch (error) {
      toast("error", String(error));
    }
  }

  const move = (from: string[], toDir: string) => run(() => ipc.libraryMove(from, toDir));
  const copy = (from: string[], toDir: string) => run(() => ipc.libraryCopy(from, toDir));

  async function paste() {
    if (clipboard.length === 0 || atRoot) return;
    const from = clipboard;
    if (clipMode === "cut") {
      setClipboard([]);
      await move(from, dir);
    } else {
      await copy(from, dir);
    }
  }

  async function remove(items: LibraryEntry[]) {
    if (atRoot || items.length === 0) return;
    const many = items.length > 1;
    const folders = items.some((e) => e.isDir);
    // One file goes without asking — it can be taken back — but a batch or a
    // folder is worth a second look.
    if (
      (many || folders) &&
      !(await askConfirm({
        title: "Delete",
        message: `Delete ${many ? `${items.length} items` : `the folder ${items[0]!.name}`}? They go to the library's trash and can be put back with Undo.`,
        confirmLabel: "Delete",
        danger: true,
      }))
    )
      return;
    try {
      const change = await ipc.libraryDelete(items.map((e) => e.path));
      setSelection([]);
      if (change.undo) setUndo({ id: change.undo, count: change.moved });
      await settle(change);
      if (change.moved > 0) toast("success", `Deleted ${change.moved === 1 ? items[0]!.name : `${change.moved} items`}. Ctrl+Z puts it back.`);
    } catch (error) {
      toast("error", String(error));
    }
  }

  async function restore() {
    if (!undo) return;
    const id = undo.id;
    setUndo(null);
    await run(() => ipc.libraryRestore(id));
  }

  async function commitRename(entry: LibraryEntry, name: string) {
    setRenaming(null);
    const next = name.trim();
    if (!next || next === entry.name) return;
    await run(() => ipc.libraryRename(entry.path, next));
    setSelection([]);
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

  const putAside = (items: string[], mode: "cut" | "copy") => {
    if (atRoot || items.length === 0) return;
    setClipboard(items, mode);
  };

  function openEntry(entry: LibraryEntry, newTab = false) {
    if (entry.isDir) {
      if (newTab) openTab("library", entry.path);
      else go(entry.path);
    } else void openFile(entry.absolute);
  }

  function togglePreview() {
    setShowPreview((on) => {
      try {
        localStorage.setItem(PREVIEW_KEY, on ? "off" : "on");
      } catch {
        // Not remembered, then.
      }
      return !on;
    });
  }

  // ----- selection -------------------------------------------------------

  function select(entry: LibraryEntry, event: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) {
    setCursor(entry.path);
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

  /** Arrow keys: move the cursor, extending the selection with Shift. */
  function step(by: number | "home" | "end", extend: boolean) {
    if (paths.length === 0) return;
    const at = cursor ? paths.indexOf(cursor) : -1;
    const next =
      by === "home" ? 0 : by === "end" ? paths.length - 1 : Math.max(0, Math.min(paths.length - 1, at + by));
    const path = paths[next]!;
    setCursor(path);
    if (extend && anchor) {
      const from = paths.indexOf(anchor);
      const [lo, hi] = from < next ? [from, next] : [next, from];
      setSelection(paths.slice(lo, hi + 1));
    } else {
      setSelection([path]);
      setAnchor(path);
    }
    listRef.current
      ?.querySelector(`[data-path="${CSS.escape(path)}"]`)
      ?.scrollIntoView({ block: "nearest" });
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
    const copying = event.ctrlKey || event.metaKey;
    if (!drag) {
      // The top level holds the mods themselves, which stay where they are.
      if (atRoot || Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD) return;
      const moving = selection.includes(entry.path) ? paths.filter((p) => selection.includes(p)) : [entry.path];
      if (!selection.includes(entry.path)) setSelection([entry.path]);
      setDrag({ paths: moving, x: event.clientX, y: event.clientY, copy: copying });
      return;
    }
    setDrag({ ...drag, x: event.clientX, y: event.clientY, copy: copying });
    const over = dropAt(event.clientX, event.clientY, drag.paths);
    if (over?.key !== useExplorer.getState().drop?.key) setDrop(over);
  }

  function endDrag(commit: boolean, event?: React.PointerEvent) {
    const start = press.current;
    press.current = null;
    const target = useExplorer.getState().drop;
    const dragged = drag;
    setDrag(null);
    setDrop(null);
    if (commit && dragged && target) {
      const copying = event ? event.ctrlKey || event.metaKey : dragged.copy;
      if (copying) void copy(dragged.paths, target.dir);
      else if (target.dir !== dir) void move(dragged.paths, target.dir);
    } else if (commit && !dragged && start && event) {
      const entry = entries.find((e) => e.path === start.path);
      if (entry) select(entry, event);
    }
  }

  // ----- keyboard --------------------------------------------------------

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isAsking() || document.querySelector('[role="dialog"], [role="menu"]')) return;
      const inField = (event.target as HTMLElement | null)?.closest("input, textarea, select");
      const ctrl = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();

      if (ctrl && key === "f") {
        filterRef.current?.focus();
        filterRef.current?.select();
      } else if (inField) {
        return;
      } else if (event.altKey && event.key === "ArrowLeft") goBack(tab.id);
      else if (event.altKey && event.key === "ArrowRight") goForward(tab.id);
      else if (event.key === "Escape") {
        if (drag) {
          press.current = null;
          setDrag(null);
          setDrop(null);
        } else if (selection.length > 0) setSelection([]);
        else if (filter) setFilter("");
        else if (clipboard.length > 0) setClipboard([]);
        else return;
      } else if (event.key === "ArrowDown") step(1, event.shiftKey);
      else if (event.key === "ArrowUp") step(-1, event.shiftKey);
      else if (event.key === "Home") step("home", event.shiftKey);
      else if (event.key === "End") step("end", event.shiftKey);
      else if (ctrl && key === "a") setSelection(paths);
      else if (ctrl && key === "x") putAside(picked.map((e) => e.path), "cut");
      else if (ctrl && key === "c") putAside(picked.map((e) => e.path), "copy");
      else if (ctrl && key === "v") void paste();
      else if (ctrl && key === "d" && !atRoot && picked.length > 0) void copy(picked.map((e) => e.path), dir);
      else if (ctrl && key === "z" && undo) void restore();
      else if (event.key === "Delete" && picked.length > 0) void remove(picked);
      else if (event.key === "F2" && single && !atRoot) setRenaming(single.path);
      else if (event.key === "Enter" && single) openEntry(single);
      else if (event.key === "Backspace" && dir) go(parentDir(dir));
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ----- menus -----------------------------------------------------------

  function menuFor(entry: LibraryEntry): MenuItem[] {
    const items = selection.includes(entry.path) ? picked : [entry];
    const many = items.length > 1;
    const count = many ? ` ${items.length} items` : "";
    const edit: MenuItem[] = atRoot
      ? []
      : [
          "separator",
          {
            label: `Cut${count}`,
            icon: <Scissors size={13} />,
            shortcut: "Ctrl+X",
            onSelect: () => putAside(items.map((e) => e.path), "cut"),
          },
          {
            label: `Copy${count}`,
            icon: <Copy size={13} />,
            shortcut: "Ctrl+C",
            onSelect: () => putAside(items.map((e) => e.path), "copy"),
          },
          {
            label: "Duplicate",
            icon: <CopyPlus size={13} />,
            shortcut: "Ctrl+D",
            onSelect: () => void copy(items.map((e) => e.path), dir),
          },
          ...(many
            ? []
            : [
                {
                  label: "Rename",
                  icon: <Pencil size={13} />,
                  shortcut: "F2",
                  onSelect: () => setRenaming(entry.path),
                } as MenuItem,
              ]),
          "separator",
          {
            label: `Delete${count}`,
            icon: <Trash2 size={13} />,
            shortcut: "Del",
            danger: true,
            onSelect: () => void remove(items),
          },
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
      ...(undo
        ? [
            {
              label: `Undo delete (${undo.count})`,
              icon: <Undo2 size={13} />,
              shortcut: "Ctrl+Z",
              onSelect: () => void restore(),
            } as MenuItem,
          ]
        : []),
      "separator",
      ...(modsDir
        ? [
            {
              label: "Show in folder",
              icon: <FolderOpen size={13} />,
              onSelect: () => reveal(`${modsDir}\\${dir.replaceAll("/", "\\")}`),
            } as MenuItem,
          ]
        : []),
    ];
  }

  const dragLabel = drag
    ? `${drag.copy ? "Copy" : "Move"} ${
        drag.paths.length === 1
          ? (entries.find((e) => e.path === drag.paths[0])?.name ?? "1 item")
          : `${drag.paths.length} items`
      }`
    : null;

  const pickedBytes = picked.reduce((sum, e) => sum + (e.isDir ? 0 : e.size), 0);
  const tabState = useExplorer((s) => s.tabs.find((t) => t.id === tab.id));

  return (
    <>
      <div className="flex shrink-0 items-center gap-1.5 px-5 py-1.5">
        <Button size="sm" disabled={!tabState?.back.length} onClick={() => goBack(tab.id)} title="Back (Alt+←)" aria-label="Back">
          <ArrowLeft size={13} />
        </Button>
        <Button size="sm" disabled={!tabState?.forward.length} onClick={() => goForward(tab.id)} title="Forward (Alt+→)" aria-label="Forward">
          <ArrowRight size={13} />
        </Button>
        <Button size="sm" disabled={atRoot} onClick={() => go(parentDir(dir))} title="Up one folder (Backspace)" aria-label="Up">
          <ArrowUp size={13} />
        </Button>
        <div className="min-w-0 flex-1 px-1">
          <Crumbs
            root="Library"
            dir={dir}
            nameOf={(index, name) => (index === 0 && listing?.modName ? listing.modName : name)}
            onGo={go}
            droppable
            target={drop?.key ?? null}
          />
        </div>
        {loading && <Loader2 size={13} className="animate-spin text-ink-muted" />}
        <label className="flex h-7 w-52 items-center gap-1.5 rounded-sm bg-raised px-2 text-[12px] text-ink-muted focus-within:text-ink-soft">
          <Search size={12} className="shrink-0" />
          <input
            ref={filterRef}
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            onKeyDown={(event) => event.key === "Escape" && (setFilter(""), event.currentTarget.blur())}
            placeholder="Filter this folder (Ctrl+F)"
            className="min-w-0 flex-1 bg-transparent text-ink placeholder:text-ink-muted focus:outline-none"
          />
        </label>
        <Button size="sm" onClick={togglePreview} title={showPreview ? "Hide the preview" : "Show the preview"} aria-pressed={showPreview}>
          {showPreview ? <EyeOff size={13} /> : <Eye size={13} />}
        </Button>
      </div>

      {!atRoot && (
        <div className="flex shrink-0 items-center gap-1 px-5 pb-1.5">
          <Button size="sm" onClick={() => void newFolder()}>
            <FolderPlus size={13} /> New folder
          </Button>
          <Button size="sm" disabled={picked.length === 0} onClick={() => putAside(picked.map((e) => e.path), "cut")} title="Ctrl+X">
            <Scissors size={13} /> Cut
          </Button>
          <Button size="sm" disabled={picked.length === 0} onClick={() => putAside(picked.map((e) => e.path), "copy")} title="Ctrl+C">
            <Copy size={13} /> Copy
          </Button>
          <Button
            size="sm"
            variant={clipboard.length > 0 ? "primary" : "ghost"}
            disabled={clipboard.length === 0}
            onClick={() => void paste()}
            title={`${clipMode === "cut" ? "Move" : "Copy"} what was set aside into this folder (Ctrl+V)`}
          >
            <ClipboardPaste size={13} /> Paste{clipboard.length > 0 ? ` ${clipboard.length}` : ""}
          </Button>
          <Button size="sm" disabled={!single} onClick={() => single && setRenaming(single.path)} title="F2">
            <Pencil size={13} /> Rename
          </Button>
          <Button size="sm" variant="danger" disabled={picked.length === 0} onClick={() => void remove(picked)} title="Delete">
            <Trash2 size={13} /> Delete
          </Button>
          {undo && (
            <Button size="sm" onClick={() => void restore()} title="Put the last delete back (Ctrl+Z)">
              <Undo2 size={13} /> Undo delete
            </Button>
          )}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          {entries.length > 0 && (
            <div className="flex h-7 shrink-0 items-center gap-2.5 border-b border-line px-5 text-[11px] text-ink-muted">
              <SortButton label="Name" k="name" sort={sort} setSort={setSort} className="flex-1 pl-6 text-left" />
              <SortButton label="Type" k="type" sort={sort} setSort={setSort} className="w-16 text-left" />
              <SortButton label="Size" k="size" sort={sort} setSort={setSort} className="w-20 justify-end" />
              <SortButton label="Modified" k="modified" sort={sort} setSort={setSort} className="w-28 justify-end" />
            </div>
          )}
          <div
            className="min-h-0 flex-1 overflow-y-auto"
            data-drop-dir={atRoot ? undefined : dir}
            data-drop-key={atRoot ? undefined : `pane-${tab.id}`}
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
                title={filter ? "Nothing matches" : atRoot ? "No mods installed" : "This folder is empty"}
                hint={
                  filter
                    ? "No file or folder here has that in its name."
                    : atRoot
                      ? "Mods appear here once they are installed."
                      : "Drop files here from Windows Explorer, drag them from another tab, or Paste."
                }
              />
            ) : (
              <ul ref={listRef} className="touch-none">
                {entries.map((entry) => {
                  const chosen = selection.includes(entry.path);
                  const dragged = drag?.paths.includes(entry.path) ?? false;
                  const cutOut = clipMode === "cut" && clipboard.includes(entry.path);
                  const over = drop?.key === `row-${entry.path}`;
                  return (
                    <li
                      key={entry.path}
                      data-path={entry.path}
                      data-drop-dir={entry.isDir && !dragged ? entry.path : undefined}
                      data-drop-key={entry.isDir && !dragged ? `row-${entry.path}` : undefined}
                      onPointerDown={(event) => onPointerDown(event, entry)}
                      onPointerMove={(event) => onPointerMove(event, entry)}
                      onPointerUp={(event) => endDrag(true, event)}
                      onPointerCancel={() => endDrag(false)}
                      onDoubleClick={() => renaming !== entry.path && openEntry(entry)}
                      onAuxClick={(event) => event.button === 1 && openEntry(entry, true)}
                      onContextMenu={(event) => {
                        if (!chosen) {
                          setSelection([entry.path]);
                          setCursor(entry.path);
                        }
                        openContextMenu(event, menuFor(entry));
                      }}
                      className={clsx(
                        "group flex h-8 items-center gap-2.5 px-5 text-[13px] transition-[background-color,opacity] duration-150 ease-quiet",
                        over ? "bg-hover" : chosen ? "bg-raised" : "hover:bg-surface",
                        cursor === entry.path && "shadow-[inset_2px_0_var(--color-ink-muted)]",
                        (dragged || cutOut) && "opacity-45",
                      )}
                    >
                      {iconOf(entry, atRoot)}
                      {renaming === entry.path ? (
                        <RenameField
                          initial={entry.name}
                          isDir={entry.isDir}
                          onDone={(name) => (name === null ? setRenaming(null) : void commitRename(entry, name))}
                        />
                      ) : (
                        <span className="min-w-0 flex-1 truncate" title={entry.name}>
                          {entry.label ?? entry.name}
                          {entry.label && entry.label !== entry.name && (
                            <span className="ml-2 font-mono text-[11px] text-ink-muted">{entry.name}</span>
                          )}
                        </span>
                      )}
                      <RowButton label={entry.isDir ? "Open" : "Open in the default program"} onClick={() => openEntry(entry)}>
                        <ExternalLink size={13} />
                      </RowButton>
                      <span className="w-16 shrink-0 truncate text-[11px] text-ink-muted">{atRoot ? "Mod" : typeOf(entry)}</span>
                      <span className="w-20 shrink-0 text-right font-mono text-[11px] text-ink-muted tabular-nums">
                        {entry.isDir ? "" : formatBytes(entry.size)}
                      </span>
                      <span className="w-28 shrink-0 text-right text-[11px] text-ink-muted">{formatWhen(entry.modified)}</span>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
          <div className="flex h-7 shrink-0 items-center gap-3 border-t border-line px-5 text-[11px] text-ink-muted">
            <span>
              {entries.length} item{entries.length === 1 ? "" : "s"}
              {filter && listing ? ` of ${listing.entries.length}` : ""}
            </span>
            {picked.length > 0 && (
              <span>
                {picked.length} selected{pickedBytes > 0 ? ` · ${formatBytes(pickedBytes)}` : ""}
              </span>
            )}
            {clipboard.length > 0 && (
              <span>
                {clipboard.length} {clipMode === "cut" ? "cut" : "copied"} — Ctrl+V to paste
              </span>
            )}
            <span className="flex-1" />
            {!atRoot && <span>Drag to move · Ctrl+drag to copy · drop files from Windows to add them</span>}
          </div>
        </div>

        {showPreview && (
          <PreviewPanel entry={single} atRoot={atRoot} onOpen={(e) => openEntry(e)} />
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

function SortButton({
  label,
  k,
  sort,
  setSort,
  className,
}: {
  label: string;
  k: SortKey;
  sort: { key: SortKey; descending: boolean };
  setSort: (sort: { key: SortKey; descending: boolean }) => void;
  className?: string;
}) {
  const active = sort.key === k;
  return (
    <button
      onClick={() => setSort({ key: k, descending: active ? !sort.descending : false })}
      className={clsx(
        "flex shrink-0 items-center gap-1 transition-colors duration-150 ease-quiet hover:text-ink-soft",
        active && "text-ink-soft",
        className,
      )}
    >
      {label}
      {active && (sort.descending ? <ArrowDown size={11} /> : <ArrowUp size={11} />)}
    </button>
  );
}

/** A name being edited in its row: the part before the extension is selected, as in Explorer. */
function RenameField({
  initial,
  isDir,
  onDone,
}: {
  initial: string;
  isDir: boolean;
  onDone: (name: string | null) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    const input = ref.current;
    if (!input) return;
    input.focus();
    const dot = initial.lastIndexOf(".");
    input.setSelectionRange(0, !isDir && dot > 0 ? dot : initial.length);
  }, [initial, isDir]);
  const finish = (value: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(value);
  };
  return (
    <input
      ref={ref}
      defaultValue={initial}
      aria-label="New name"
      onPointerDown={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Enter") finish(event.currentTarget.value);
        else if (event.key === "Escape") finish(null);
      }}
      onBlur={(event) => finish(event.currentTarget.value)}
      className="h-6 min-w-0 flex-1 rounded-sm border border-line-strong bg-base px-1.5 text-[13px] text-ink focus:border-ink-muted focus:outline-none"
    />
  );
}

/** The selected file, shown beside the list: a picture, the start of a text file, or what it is. */
function PreviewPanel({
  entry,
  atRoot,
  onOpen,
}: {
  entry: LibraryEntry | null;
  atRoot: boolean;
  onOpen: (entry: LibraryEntry) => void;
}) {
  const [preview, setPreview] = useState<LibraryPreview | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setPreview(null);
    setFailed(false);
    if (!entry || entry.isDir || atRoot) return;
    let live = true;
    const timer = window.setTimeout(() => {
      ipc
        .libraryPreview(entry.path)
        .then((result) => live && setPreview(result))
        .catch(() => live && setFailed(true));
    }, 120);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [entry, atRoot]);

  return (
    <aside className="flex w-80 shrink-0 flex-col border-l border-line">
      {!entry ? (
        <p className="m-auto max-w-52 text-center text-[12px] leading-relaxed text-ink-muted">
          Select a file to see it here.
        </p>
      ) : (
        <>
          <div className="min-h-0 flex-1 overflow-auto p-4">
            {entry.isDir || atRoot ? (
              <div className="grid h-32 place-items-center text-ink-muted">
                {atRoot ? <Package size={36} /> : <Folder size={36} />}
              </div>
            ) : preview?.kind === "image" && preview.image ? (
              <img src={preview.image} alt={entry.name} className="max-h-72 w-full object-contain" />
            ) : preview?.kind === "text" && preview.text !== null ? (
              <pre className="font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-all text-ink-soft">
                {preview.text}
                {preview.truncated && <span className="text-ink-muted">{"\n…"}</span>}
              </pre>
            ) : (
              <div className="grid h-32 place-items-center text-[12px] text-ink-muted">
                {failed || preview?.kind === "none" ? "No preview for this kind of file" : <Loader2 size={16} className="animate-spin" />}
              </div>
            )}
          </div>
          <div className="shrink-0 space-y-1 border-t border-line p-4 text-[11px]">
            <p className="truncate text-[13px] text-ink" title={entry.name}>
              {entry.label ?? entry.name}
            </p>
            <p className="text-ink-muted">
              {entry.isDir ? (atRoot ? "Mod folder" : "Folder") : `${typeOf(entry)} · ${formatBytes(entry.size)}`}
              {entry.modified !== null && ` · ${formatWhen(entry.modified)}`}
            </p>
            <p className="truncate font-mono text-ink-muted" title={entry.absolute}>
              {entry.absolute}
            </p>
            <div className="flex gap-1 pt-1.5">
              <Button size="sm" variant="outline" onClick={() => onOpen(entry)}>
                <ExternalLink size={12} /> Open
              </Button>
              <Button size="sm" onClick={() => reveal(entry.absolute)}>
                <FolderOpen size={12} /> Show in folder
              </Button>
            </div>
          </div>
        </>
      )}
    </aside>
  );
}
