import { useVirtualizer } from "@tanstack/react-virtual";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  FolderMinus,
  FolderPlus,
  Package,
  Plus,
  RefreshCw,
  Search,
  StickyNote,
  Trash2,
} from "lucide-react";
import { useMemo, useRef, useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc, modPageUrl, MOD_TYPE_META, type GroupView, type ModView } from "../lib/ipc";
import { useApp, visibleMods } from "../store/useApp";
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
  const selection = useApp((s) => s.selection);
  const setSelection = useApp((s) => s.setSelection);
  const toggle = useApp((s) => s.toggle);
  const refresh = useApp((s) => s.refresh);

  const [lastClicked, setLastClicked] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  const mods = visibleMods(snapshot, search);
  const groups = snapshot?.groups ?? [];

  // Rows are a flat list of headers and mods so one virtualizer covers both.
  const rows = useMemo<Row[]>(() => {
    const out: Row[] = [];
    const ungrouped = mods.filter((m) => m.groupId === null);

    for (const group of groups) {
      const inGroup = mods.filter((m) => m.groupId === group.id);
      if (inGroup.length === 0 && search) continue;
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
  }, [mods, groups, search]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  /** Click, ctrl-click and shift-click, as any list is expected to behave. */
  function selectRow(mod: ModView, event: React.MouseEvent) {
    const ids = mods.map((m) => m.id);
    if (event.shiftKey && lastClicked !== null) {
      const from = ids.indexOf(lastClicked);
      const to = ids.indexOf(mod.id);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from < to ? [from, to] : [to, from];
        setSelection(ids.slice(lo, hi + 1));
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

  // One mod selected shows its details beside the list.
  const detailed =
    selection.length === 1 ? (snapshot?.mods.find((m) => m.id === selection[0]) ?? null) : null;

  /** Bulk actions act on the selection when there is one, otherwise on everything shown. */
  const bulkTargets = selection.length > 0 ? selection : mods.map((m) => m.id);

  if (!snapshot) return null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Toolbar
        search={search}
        setSearch={setSearch}
        onAdd={onAdd}
        selectionCount={selection.length}
        totalShown={mods.length}
        onEnableAll={() => toggle(bulkTargets, true)}
        onDisableAll={() => toggle(bulkTargets, false)}
        onGroup={async () => {
          const name = window.prompt("Group name");
          if (!name?.trim()) return;
          const id = await ipc.createGroup(name.trim());
          if (selection.length > 0) await ipc.assignGroup(selection, id);
          await refresh();
        }}
        onUngroup={async () => {
          await ipc.assignGroup(selection, null);
          await refresh();
        }}
      />

      <GameUpdateBanner />
      <Ue4ssBanner />

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
      {rows.length === 0 ? (
        <Empty
          icon={<Package size={20} />}
          title={search ? "Nothing matches that search" : "No mods installed yet"}
          hint={
            search
              ? "Try a different name, or clear the search box."
              : "Drop a .zip, .7z or .rar anywhere in this window, or use Add mods. The type and install location are worked out for you."
          }
          action={
            !search && (
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
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search mods"
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
          onClick={() => {
            if (window.confirm(`Delete the group "${group.name}"? Its mods stay installed.`)) {
              void ipc.deleteGroup(group.id).then(refresh);
            }
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
}: {
  mod: ModView;
  selected: boolean;
  onSelect: (event: React.MouseEvent) => void;
}) {
  const toggle = useApp((s) => s.toggle);
  const uninstall = useApp((s) => s.uninstall);
  const refresh = useApp((s) => s.refresh);
  const setView = useApp((s) => s.setView);
  const overridden = useApp((s) =>
    s.conflicts?.overridden.find((c) => c.modId === mod.id),
  );

  return (
    <div
      onClick={onSelect}
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
        onDoubleClick={async (event) => {
          event.stopPropagation();
          const name = window.prompt("Rename mod", mod.name);
          if (name?.trim() && name !== mod.name) {
            await ipc.renameMod(mod.id, name.trim());
            await refresh();
          }
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
          if (window.confirm(`Remove "${mod.name}" and delete its files?`)) {
            void uninstall(mod.id);
          }
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
