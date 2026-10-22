import { useVirtualizer } from "@tanstack/react-virtual";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  AlertTriangle,
  ArrowDownWideNarrow,
  ArrowUpNarrowWide,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FolderMinus,
  FolderOpen,
  FolderPlus,
  Info,
  Package,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Search,
  StickyNote,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { formatBytes } from "../lib/format";
import {
  ipc,
  modPageUrl,
  MOD_TYPE_META,
  type GroupView,
  type ModTypeId,
  type ModView,
} from "../lib/ipc";
import {
  DEFAULT_LIST_VIEW,
  useApp,
  visibleMods,
  type ModFilter,
  type ModSort,
} from "../store/useApp";
import { askConfirm, askText, isAsking } from "./Ask";
import { openContextMenu, type MenuItem } from "./ContextMenu";
import { OverwriteBar } from "./OverwriteBar";
import { ProfileSwitcher } from "./ProfileSwitcher";
import { ModDetails } from "./ModDetails";
import { UpdateAllButton } from "./ModListSections";
import { GameUpdateBanner } from "./SafetySections";
import { Ue4ssBanner } from "./Ue4ssSection";
import { Badge, Button, Checkbox, Empty } from "./ui";

const ROW_HEIGHT = 38;

type Row =
  | { kind: "group"; group: GroupView | null; mods: ModView[] }
  | { kind: "mod"; mod: ModView };

export function ModsView({ onAdd }: { onAdd: () => void }) {
  const snapshot = useApp((s) => s.snapshot);
  const search = useApp((s) => s.search);
  const setSearch = useApp((s) => s.setSearch);
  const listView = useApp((s) => s.listView);
  const selection = useApp((s) => s.selection);
  const setSelection = useApp((s) => s.setSelection);
  const toggle = useApp((s) => s.toggle);
  const actions = useModActions();

  const [lastClicked, setLastClicked] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const mods = visibleMods(snapshot, search, listView);
  const groups = snapshot?.groups ?? [];
  const narrowed = search.trim() !== "" || listView.filter !== "all" || !!listView.type || !!listView.tag;

  // Rows are a flat list of headers and mods so one virtualizer covers both.
  const rows = useMemo<Row[]>(() => {
    const out: Row[] = [];
    const ungrouped = mods.filter((m) => m.groupId === null);

    for (const group of groups) {
      const inGroup = mods.filter((m) => m.groupId === group.id);
      if (inGroup.length === 0 && narrowed) continue;
      out.push({ kind: "group", group, mods: inGroup });
      if (!group.collapsed) {
        out.push(...inGroup.map((mod) => ({ kind: "mod" as const, mod })));
      }
    }

    if (ungrouped.length > 0) {
      if (groups.length > 0) {
        out.push({ kind: "group", group: null, mods: ungrouped });
      }
      out.push(...ungrouped.map((mod) => ({ kind: "mod" as const, mod })));
    }
    return out;
  }, [mods, groups, narrowed]);

  /** The mods as the list shows them, for keyboard movement and ranges. */
  const shownIds = useMemo(
    () => rows.flatMap((row) => (row.kind === "mod" ? [row.mod.id] : [])),
    [rows],
  );

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  /** Click, ctrl-click and shift-click, as any list is expected to behave. */
  function selectRow(mod: ModView, event: React.MouseEvent) {
    if (event.shiftKey && lastClicked !== null) {
      const from = shownIds.indexOf(lastClicked);
      const to = shownIds.indexOf(mod.id);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from < to ? [from, to] : [to, from];
        setSelection(shownIds.slice(lo, hi + 1));
        return;
      }
    }
    if (event.ctrlKey || event.metaKey) {
      setSelection(
        selection.includes(mod.id)
          ? selection.filter((id) => id !== mod.id)
          : [...selection, mod.id],
      );
    } else {
      setSelection([mod.id]);
    }
    setLastClicked(mod.id);
  }

  /** Move the selection with the arrow keys, extending it with Shift. */
  function step(to: number, extend: boolean) {
    const id = shownIds[Math.max(0, Math.min(shownIds.length - 1, to))];
    if (id === undefined) return;
    if (extend && lastClicked !== null) {
      const from = shownIds.indexOf(lastClicked);
      const at = shownIds.indexOf(id);
      const [lo, hi] = from < at ? [from, at] : [at, from];
      setSelection(shownIds.slice(lo, hi + 1));
    } else {
      setSelection([id]);
      setLastClicked(id);
    }
    const row = rows.findIndex((r) => r.kind === "mod" && r.mod.id === id);
    if (row !== -1) virtualizer.scrollToIndex(row, { align: "auto" });
  }

  // The list's keyboard: only while nothing else — a text box, a dialog,
  // a question — has the keys.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing =
        target?.closest("input, textarea, select, [contenteditable=true]") !== null &&
        target !== null;
      if (isAsking() || document.querySelector('[role="dialog"], [role="menu"]')) return;
      const ctrl = event.ctrlKey || event.metaKey;

      if ((ctrl && event.key.toLowerCase() === "f") || (!typing && event.key === "/")) {
        event.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      if (typing) {
        if (event.key === "Escape" && target === searchRef.current) {
          setSearch("");
          searchRef.current?.blur();
        }
        return;
      }

      const current = selection.length > 0 ? selection : [];
      const at = lastClicked !== null ? shownIds.indexOf(lastClicked) : -1;
      const handled = (() => {
        switch (event.key) {
          case "ArrowDown":
            step(at + 1, event.shiftKey);
            return true;
          case "ArrowUp":
            step(at === -1 ? 0 : at - 1, event.shiftKey);
            return true;
          case "Home":
            step(0, event.shiftKey);
            return true;
          case "End":
            step(shownIds.length - 1, event.shiftKey);
            return true;
          case "Escape":
            setSelection([]);
            return true;
          case " ":
            if (current.length === 0) return false;
            void actions.toggleMany(current);
            return true;
          case "Delete":
            if (current.length === 0) return false;
            void actions.uninstallMany(current);
            return true;
          case "F2": {
            const mod = snapshot?.mods.find((m) => m.id === current[0]);
            if (current.length !== 1 || !mod) return false;
            void actions.rename(mod);
            return true;
          }
          default:
            break;
        }
        if (ctrl && event.key.toLowerCase() === "a") {
          setSelection(shownIds);
          return true;
        }
        if (ctrl && event.key.toLowerCase() === "g" && current.length > 0) {
          void actions.groupMany(current);
          return true;
        }
        return false;
      })();
      if (handled) event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // One mod selected shows its details beside the list.
  const detailed =
    selection.length === 1 ? (snapshot?.mods.find((m) => m.id === selection[0]) ?? null) : null;

  /** Bulk actions act on the selection when there is one, otherwise on everything shown. */
  const bulkTargets = selection.length > 0 ? selection : mods.map((m) => m.id);

  if (!snapshot) return null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Toolbar
        searchRef={searchRef}
        search={search}
        setSearch={setSearch}
        onAdd={onAdd}
        selectionCount={selection.length}
        totalShown={mods.length}
        onEnableAll={() => toggle(bulkTargets, true)}
        onDisableAll={() => toggle(bulkTargets, false)}
        onGroup={() => void actions.groupMany(selection)}
        onUngroup={() => void actions.moveToGroup(selection, null)}
      />
      <FilterBar shown={mods.length} total={snapshot.mods.length} />

      <GameUpdateBanner />
      <Ue4ssBanner />

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
      {rows.length === 0 ? (
        <Empty
          icon={<Package size={20} />}
          title={narrowed ? "Nothing matches" : "No mods installed yet"}
          hint={
            narrowed
              ? "Try a different search, or clear the filters."
              : "Drop a .zip, .7z or .rar anywhere in this window, or use Add mods. The type and install location are worked out for you."
          }
          action={
            !narrowed && (
              <Button variant="primary" onClick={onAdd}>
                <Plus size={14} /> Add mods
              </Button>
            )
          }
        />
      ) : (
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
          <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
            {virtualizer.getVirtualItems().map((item) => {
              const row = rows[item.index];
              if (!row) return null;
              return (
                <div
                  key={item.key}
                  className="absolute inset-x-0"
                  style={{ height: item.size, transform: `translateY(${item.start}px)` }}
                >
                  {row.kind === "group" ? (
                    <GroupHeader row={row} />
                  ) : (
                    <ModRow
                      mod={row.mod}
                      selected={selection.includes(row.mod.id)}
                      onSelect={(event) => selectRow(row.mod, event)}
                      onMenu={(event) => {
                        // Right-clicking outside the selection selects that mod.
                        const targets = selection.includes(row.mod.id)
                          ? selection
                          : [row.mod.id];
                        if (!selection.includes(row.mod.id)) {
                          setSelection([row.mod.id]);
                          setLastClicked(row.mod.id);
                        }
                        openContextMenu(event, actions.menuFor(row.mod, targets));
                      }}
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

        </div>
        {detailed && <ModDetails mod={detailed} onClose={() => setSelection([])} />}
      </div>

      {snapshot.overwrite.files > 0 && <OverwriteBar />}
    </div>
  );
}

function Toolbar({
  searchRef,
  search,
  setSearch,
  onAdd,
  selectionCount,
  totalShown,
  onEnableAll,
  onDisableAll,
  onGroup,
  onUngroup,
}: {
  searchRef: React.RefObject<HTMLInputElement | null>;
  search: string;
  setSearch: (value: string) => void;
  onAdd: () => void;
  selectionCount: number;
  totalShown: number;
  onEnableAll: () => void;
  onDisableAll: () => void;
  onGroup: () => void;
  onUngroup: () => void;
}) {
  const scope = selectionCount > 0 ? `${selectionCount} selected` : `all ${totalShown}`;
  return (
    <div className="flex h-12 min-w-0 shrink-0 items-center gap-1.5 border-b border-line px-3 xl:gap-2">
      <ProfileSwitcher />
      <div className="mx-1 h-5 w-px bg-line" />
      {/* The search box gives up width first, so the buttons never run off the edge. */}
      <div className="relative w-56 min-w-20 shrink">
        <Search
          size={13}
          className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-muted"
        />
        <input
          ref={searchRef}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search mods"
          title="Search names, tags and notes (Ctrl+F)"
          aria-label="Search mods"
          className="h-8 w-full rounded-md border border-line bg-base pr-2 pl-7 text-[12px] placeholder:text-ink-muted focus:border-accent-soft focus:outline-none"
        />
      </div>

      <div className="mx-1 h-5 w-px bg-line" />

      {/* With a selection the badge names the scope, so the buttons can stay short. */}
      {selectionCount > 0 && (
        <span className="shrink-0 whitespace-nowrap">
          <Badge tone="accent">{selectionCount} selected</Badge>
        </span>
      )}
      <Button size="sm" onClick={onEnableAll} title={`Enable ${scope}`}>
        Enable{selectionCount > 0 ? "" : " all"}
      </Button>
      <Button size="sm" onClick={onDisableAll} title={`Disable ${scope}`}>
        Disable{selectionCount > 0 ? "" : " all"}
      </Button>

      {selectionCount > 0 && (
        <>
          <Button
            size="sm"
            className="px-1.5"
            onClick={onGroup}
            title="Group selected"
            aria-label="Group selected"
          >
            <FolderPlus size={14} />
          </Button>
          <Button
            size="sm"
            className="px-1.5"
            onClick={onUngroup}
            title="Remove selected from their group"
            aria-label="Ungroup selected"
          >
            <FolderMinus size={14} />
          </Button>
        </>
      )}

      <div className="ml-auto" />
      <UpdateAllButton />
      <CheckUpdatesButton />
      <Button variant="primary" size="sm" onClick={onAdd}>
        <Plus size={13} /> Add mods
      </Button>
    </div>
  );
}

/// Only mods installed from Nexus can be checked, so the button says nothing
/// when there are none rather than failing when pressed.
function CheckUpdatesButton() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [checking, setChecking] = useState(false);

  const fromNexus = (snapshot?.mods ?? []).filter((m) => m.nexusModId !== null).length;
  if (fromNexus === 0) return null;

  async function check() {
    setChecking(true);
    try {
      const report = await ipc.checkModUpdates();
      await refresh();
      toast(
        report.outdated > 0 ? "info" : "success",
        report.outdated > 0
          ? `${report.outdated} mod${report.outdated === 1 ? " has" : "s have"} a newer version on Nexus.`
          : "Everything is up to date.",
      );
    } catch (error) {
      toast("error", String(error));
    } finally {
      setChecking(false);
    }
  }

  return (
    <Button size="sm" disabled={checking} onClick={() => void check()} title="Check Nexus for newer versions">
      <RefreshCw size={13} className={checking ? "animate-spin" : undefined} />
      <span className="hidden xl:inline">{checking ? "Checking…" : "Check updates"}</span>
    </Button>
  );
}

function GroupHeader({ row }: { row: Extract<Row, { kind: "group" }> }) {
  const toggle = useApp((s) => s.toggle);
  const refresh = useApp((s) => s.refresh);
  const { group, mods } = row;

  const enabled = mods.filter((m) => m.enabled).length;
  const allOn = mods.length > 0 && enabled === mods.length;
  const someOn = enabled > 0 && enabled < mods.length;

  return (
    <div className="group flex h-full items-center gap-2 border-b border-line/60 bg-raised/60 px-3">
      {group ? (
        <button
          onClick={() => {
            void ipc.setGroupCollapsed(group.id, !group.collapsed).then(refresh);
          }}
          className="text-ink-muted hover:text-ink"
          aria-label={group.collapsed ? "Expand group" : "Collapse group"}
        >
          {group.collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
        </button>
      ) : (
        <span className="w-3.5" />
      )}

      <Checkbox
        checked={allOn}
        indeterminate={someOn}
        onChange={(next) => toggle(mods.map((m) => m.id), next)}
        label={`Toggle ${group?.name ?? "ungrouped"}`}
      />

      {group?.color && (
        <span className="size-2 rounded-full" style={{ background: group.color }} />
      )}
      <span className="text-[11px] font-semibold tracking-wide uppercase">
        {group?.name ?? "Ungrouped"}
      </span>
      <span className="text-[11px] text-ink-muted">
        {enabled}/{mods.length}
      </span>

      {group && (
        <button
          onClick={async () => {
            const sure = await askConfirm({
              title: "Delete group",
              message: `Delete the group “${group.name}”? Its mods stay installed.`,
              confirmLabel: "Delete group",
              danger: true,
            });
            if (sure) await ipc.deleteGroup(group.id).then(refresh);
          }}
          className="ml-auto text-ink-muted opacity-0 transition-opacity group-hover:opacity-100 hover:text-danger"
          aria-label="Delete group"
        >
          <Trash2 size={13} />
        </button>
      )}
    </div>
  );
}

function ModRow({
  mod,
  selected,
  onSelect,
  onMenu,
}: {
  mod: ModView;
  selected: boolean;
  onSelect: (event: React.MouseEvent) => void;
  onMenu: (event: React.MouseEvent) => void;
}) {
  const toggle = useApp((s) => s.toggle);
  const setView = useApp((s) => s.setView);
  const actions = useModActions();
  const overridden = useApp((s) =>
    s.conflicts?.overridden.find((c) => c.modId === mod.id),
  );

  return (
    <div
      onClick={onSelect}
      onContextMenu={onMenu}
      className={clsx(
        "group flex h-full items-center gap-2.5 border-b border-line/40 px-3 transition-colors",
        selected ? "bg-accent-soft/12" : "hover:bg-hover/60",
      )}
    >
      <span className="w-3.5" />
      <Checkbox
        checked={mod.enabled}
        onChange={(next) => toggle([mod.id], next)}
        label={`Toggle ${mod.name}`}
      />

      <button
        onDoubleClick={(event) => {
          event.stopPropagation();
          void actions.rename(mod);
        }}
        className={clsx(
          "min-w-0 flex-1 truncate text-left text-[13px]",
          mod.enabled ? "text-ink" : "text-ink-muted",
        )}
        title={mod.name}
      >
        {mod.name}
      </button>

      {mod.tags.slice(0, 2).map((tag) => (
        <span
          key={tag}
          className="shrink-0 rounded-full border border-line px-1.5 text-[10px] leading-4 text-ink-soft"
        >
          {tag}
        </span>
      ))}
      {mod.tags.length > 2 && (
        <span className="shrink-0 text-[10px] text-ink-muted" title={mod.tags.slice(2).join(", ")}>
          +{mod.tags.length - 2}
        </span>
      )}
      {mod.notes && (
        <StickyNote size={12} className="shrink-0 text-ink-muted" aria-label="Has notes">
          <title>{mod.notes}</title>
        </StickyNote>
      )}

      {mod.warnings.length > 0 && (
        <AlertTriangle size={13} className="shrink-0 text-warn" aria-label="Has warnings">
          <title>{mod.warnings.join("\n")}</title>
        </AlertTriangle>
      )}

      {mod.missingRequirements.map((requirement) => (
        <Badge
          key={requirement}
          tone="danger"
          title={`This mod does nothing without ${requirement}, and the game will not load it. Install it from Settings.`}
        >
          Needs {requirement}
        </Badge>
      ))}

      {mod.misordered.length > 0 && (
        <Badge
          tone="warn"
          title={`Load order rules this breaks: ${mod.name} ${mod.misordered.join("; ")}. Sort the order from the mod's details or the Load order screen.`}
        >
          Order
        </Badge>
      )}

      {mod.fileConflicts && <FileConflictFlag mod={mod} />}

      {/* Enabled, but a mod further down the order replaces some of it. */}
      {overridden && overridden.losing > 0 && (
        <button
          onClick={(event) => {
            event.stopPropagation();
            setView("conflicts");
          }}
          title={`${overridden.losing} of this mod's assets are replaced by a mod later in the load order.${overridden.winning > 0 ? ` It replaces ${overridden.winning} from earlier mods.` : ""}`}
          className="shrink-0"
        >
          <Badge tone="danger">{overridden.losing} overridden</Badge>
        </button>
      )}

      {mod.latestVersion && (
        <button
          onClick={(event) => {
            event.stopPropagation();
            if (mod.nexusModId !== null) void openUrl(modPageUrl(mod.nexusModId));
          }}
          title={`Nexus has ${mod.latestVersion}, you have ${mod.version ?? "an unknown version"}. Opens the mod page.`}
          className="shrink-0"
        >
          <Badge tone="warn">Update</Badge>
        </button>
      )}

      <div className="flex shrink-0 gap-1">
        {mod.componentTypes.slice(0, 3).map((type) => (
          <Badge
            key={type}
            tone={type === "unknown" ? "danger" : "accent"}
            title={MOD_TYPE_META[type].hint}
          >
            {MOD_TYPE_META[type].label}
          </Badge>
        ))}
      </div>

      {/* Enabled but not on disk yet: the change is waiting for Apply. */}
      {mod.enabled !== mod.deployed && <Badge tone="warn">Pending</Badge>}

      <span className="w-16 shrink-0 text-right text-[11px] text-ink-muted tabular-nums">
        {formatBytes(mod.sizeBytes)}
      </span>

      <button
        onClick={(event) => {
          event.stopPropagation();
          void actions.uninstallMany([mod.id]);
        }}
        className="shrink-0 text-ink-muted opacity-0 transition-opacity group-hover:opacity-100 hover:text-danger"
        aria-label={`Uninstall ${mod.name}`}
      >
        <Trash2 size={13} />
      </button>
    </div>
  );
}

/**
 * Loose files this mod shares with other enabled mods, flagged the way Mod
 * Organizer 2 does. Clicking opens the Files view on exactly those paths.
 */
function FileConflictFlag({ mod }: { mod: ModView }) {
  const showFileConflicts = useApp((s) => s.showFileConflicts);
  const conflicts = mod.fileConflicts;
  if (!conflicts) return null;

  const { wins, loses, redundant } = conflicts;
  const [tone, label, detail]: ["accent" | "warn" | "danger", string, string] = redundant
    ? [
        "danger",
        "Redundant",
        `Every file of this mod (${loses}) is replaced by mods later in the load order, so it does nothing.`,
      ]
    : wins > 0 && loses > 0
      ? [
          "warn",
          `±${wins}/${loses}`,
          `Replaces ${wins} file(s) of earlier mods, and loses ${loses} to later ones.`,
        ]
      : wins > 0
        ? ["accent", `+${wins}`, `Replaces ${wins} file(s) from mods earlier in the load order.`]
        : ["warn", `−${loses}`, `${loses} file(s) are replaced by mods later in the load order.`];

  return (
    <button
      onClick={(event) => {
        event.stopPropagation();
        showFileConflicts(mod.id, mod.name);
      }}
      title={`${detail} Click to see which.`}
      className="shrink-0"
    >
      <Badge tone={tone}>{label}</Badge>
    </button>
  );
}

const FILTERS: { id: ModFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "enabled", label: "Enabled" },
  { id: "disabled", label: "Disabled" },
  { id: "updates", label: "Updates" },
  { id: "problems", label: "Problems" },
  { id: "pending", label: "Not applied" },
];

const SORTS: { id: ModSort; label: string }[] = [
  { id: "order", label: "Load order" },
  { id: "name", label: "Name" },
  { id: "size", label: "Size" },
  { id: "installed", label: "Installed" },
  { id: "type", label: "Type" },
];

/** Which mods the list shows and in what order. */
function FilterBar({ shown, total }: { shown: number; total: number }) {
  const snapshot = useApp((s) => s.snapshot);
  const listView = useApp((s) => s.listView);
  const setListView = useApp((s) => s.setListView);

  const mods = snapshot?.mods ?? [];
  const types = [...new Set(mods.flatMap((m) => m.componentTypes))] as ModTypeId[];
  const tags = [...new Set(mods.flatMap((m) => m.tags))].sort((a, b) => a.localeCompare(b));
  const changed =
    listView.filter !== "all" || listView.type !== null || listView.tag !== null ||
    listView.sort !== "order" || listView.descending;
  const select =
    "h-6 rounded-md border border-line bg-base px-1.5 text-[11px] text-ink-soft focus:border-accent-soft focus:outline-none";

  return (
    <div className="flex h-9 shrink-0 items-center gap-1 overflow-x-auto border-b border-line px-3 text-[11px]">
      {FILTERS.map((filter) => (
        <button
          key={filter.id}
          onClick={() => setListView({ filter: filter.id })}
          className={clsx(
            "h-6 shrink-0 rounded-full px-2.5 transition-colors",
            listView.filter === filter.id
              ? "bg-accent/15 text-accent-soft"
              : "text-ink-muted hover:bg-hover hover:text-ink-soft",
          )}
        >
          {filter.label}
        </button>
      ))}
      <div className="mx-1 h-4 w-px shrink-0 bg-line" />
      <select
        aria-label="Filter by type"
        value={listView.type ?? ""}
        onChange={(event) => setListView({ type: (event.target.value || null) as ModTypeId | null })}
        className={select}
      >
        <option value="">Any type</option>
        {types.map((type) => (
          <option key={type} value={type}>
            {MOD_TYPE_META[type].label}
          </option>
        ))}
      </select>
      {tags.length > 0 && (
        <select
          aria-label="Filter by tag"
          value={listView.tag ?? ""}
          onChange={(event) => setListView({ tag: event.target.value || null })}
          className={select}
        >
          <option value="">Any tag</option>
          {tags.map((tag) => (
            <option key={tag} value={tag}>
              {tag}
            </option>
          ))}
        </select>
      )}
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <span className="text-ink-muted tabular-nums">
          {shown === total ? `${total} mods` : `${shown} of ${total}`}
        </span>
        {changed && (
          <button
            title="Clear filters and sorting"
            aria-label="Clear filters and sorting"
            onClick={() => setListView(DEFAULT_LIST_VIEW)}
            className="grid size-6 place-items-center rounded-md text-ink-muted hover:bg-hover hover:text-ink"
          >
            <X size={12} />
          </button>
        )}
        <span className="ml-1 text-ink-muted">Sort</span>
        <select
          aria-label="Sort by"
          value={listView.sort}
          onChange={(event) => setListView({ sort: event.target.value as ModSort })}
          className={select}
        >
          {SORTS.map((sort) => (
            <option key={sort.id} value={sort.id}>
              {sort.label}
            </option>
          ))}
        </select>
        <button
          title={listView.descending ? "Descending" : "Ascending"}
          aria-label="Reverse the order"
          onClick={() => setListView({ descending: !listView.descending })}
          className="grid size-6 place-items-center rounded-md text-ink-muted hover:bg-hover hover:text-ink"
        >
          {listView.descending ? <ArrowDownWideNarrow size={13} /> : <ArrowUpNarrowWide size={13} />}
        </button>
      </div>
    </div>
  );
}

/** What can be done to mods from the list, its keyboard and its menu. */
function useModActions() {
  const snapshot = useApp((s) => s.snapshot);
  const toggle = useApp((s) => s.toggle);
  const uninstall = useApp((s) => s.uninstall);
  const refresh = useApp((s) => s.refresh);
  const setSelection = useApp((s) => s.setSelection);
  const toast = useApp((s) => s.toast);
  const mods = snapshot?.mods ?? [];
  const named = (ids: number[]) => mods.filter((m) => ids.includes(m.id));

  async function rename(mod: ModView) {
    const name = await askText({
      title: "Rename mod",
      label: "Name",
      initial: mod.name,
      confirmLabel: "Rename",
    });
    if (!name || name === mod.name) return;
    try {
      await ipc.renameMod(mod.id, name);
      await refresh();
    } catch (error) {
      toast("error", String(error));
    }
  }

  /** Switch them all on, or all off when every one is on already. */
  async function toggleMany(ids: number[]) {
    const all = named(ids);
    await toggle(ids, !all.every((m) => m.enabled));
  }

  async function uninstallMany(ids: number[]) {
    const all = named(ids);
    if (all.length === 0) return;
    const sure = await askConfirm({
      title: all.length === 1 ? "Uninstall mod" : `Uninstall ${all.length} mods`,
      message:
        all.length === 1
          ? `Remove “${all[0]!.name}” and delete its files from the library?`
          : `Remove these mods and delete their files from the library?\n\n${all
              .slice(0, 8)
              .map((m) => `• ${m.name}`)
              .join("\n")}${all.length > 8 ? `\n…and ${all.length - 8} more` : ""}`,
      confirmLabel: "Uninstall",
      danger: true,
    });
    if (!sure) return;
    for (const mod of all) await uninstall(mod.id);
    setSelection([]);
  }

  async function groupMany(ids: number[]) {
    if (ids.length === 0) return;
    const name = await askText({ title: "New group", label: "Group name", confirmLabel: "Create" });
    if (!name) return;
    try {
      const id = await ipc.createGroup(name);
      await ipc.assignGroup(ids, id);
      await refresh();
    } catch (error) {
      toast("error", String(error));
    }
  }

  async function moveToGroup(ids: number[], group: number | null) {
    try {
      await ipc.assignGroup(ids, group);
      await refresh();
    } catch (error) {
      toast("error", String(error));
    }
  }

  async function openFolder(mod: ModView) {
    try {
      const { mods: root } = await ipc.folders();
      const separator = root.includes("\\") ? "\\" : "/";
      await revealItemInDir(`${root}${separator}${mod.folder}`);
    } catch (error) {
      toast("error", String(error));
    }
  }

  /** The right-click menu for `mod`, acting on `ids` (the selection, or it). */
  function menuFor(mod: ModView, ids: number[]): MenuItem[] {
    const many = ids.length > 1;
    const all = named(ids);
    const allOn = all.every((m) => m.enabled);
    const groups = snapshot?.groups ?? [];
    return [
      {
        label: allOn ? (many ? `Disable ${ids.length} mods` : "Disable") : many ? `Enable ${ids.length} mods` : "Enable",
        icon: allOn ? <PowerOff size={13} /> : <Power size={13} />,
        shortcut: "Space",
        onSelect: () => void toggle(ids, !allOn),
      },
      {
        label: "Rename…",
        icon: <Pencil size={13} />,
        shortcut: "F2",
        disabled: many,
        onSelect: () => void rename(mod),
      },
      {
        label: "Details",
        icon: <Info size={13} />,
        disabled: many,
        onSelect: () => setSelection([mod.id]),
      },
      "separator",
      {
        label: "Open folder",
        icon: <FolderOpen size={13} />,
        disabled: many,
        onSelect: () => void openFolder(mod),
      },
      {
        label: mod.latestVersion ? `Nexus page (update to ${mod.latestVersion})` : "Nexus page",
        icon: <ExternalLink size={13} />,
        disabled: many || mod.nexusModId === null,
        onSelect: () => mod.nexusModId !== null && void openUrl(modPageUrl(mod.nexusModId)),
      },
      "separator",
      {
        label: "Move to group",
        icon: <FolderPlus size={13} />,
        items: [
          ...groups.map((group) => ({
            label: group.name,
            onSelect: () => void moveToGroup(ids, group.id),
          })),
          ...(groups.length > 0 ? (["separator"] as MenuItem[]) : []),
          { label: "New group…", shortcut: "Ctrl+G", onSelect: () => void groupMany(ids) },
        ],
      },
      {
        label: "Remove from group",
        icon: <FolderMinus size={13} />,
        disabled: all.every((m) => m.groupId === null),
        onSelect: () => void moveToGroup(ids, null),
      },
      "separator",
      {
        label: many ? `Uninstall ${ids.length} mods…` : "Uninstall…",
        icon: <Trash2 size={13} />,
        shortcut: "Del",
        danger: true,
        onSelect: () => void uninstallMany(ids),
      },
    ];
  }

  return { rename, toggleMany, uninstallMany, groupMany, moveToGroup, menuFor };
}
