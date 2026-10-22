import { create } from "zustand";

import {
  ipc,
  type AppSnapshot,
  type ModTypeId,
  type ModView,
  type ConflictReport,
  type FoundArchive,
  type QueueItem,
  type StagedInstall,
} from "../lib/ipc";

/** Which mods the list shows. */
export type ModFilter = "all" | "enabled" | "disabled" | "updates" | "problems" | "pending";
/** How the list is ordered; `order` is the load order. */
export type ModSort = "order" | "name" | "size" | "installed" | "type";

export interface ListView {
  filter: ModFilter;
  /** Only mods with a component of this type. */
  type: ModTypeId | null;
  /** Only mods carrying this tag. */
  tag: string | null;
  sort: ModSort;
  descending: boolean;
}

export const DEFAULT_LIST_VIEW: ListView = {
  filter: "all",
  type: null,
  tag: null,
  sort: "order",
  descending: false,
};

export type View =
  | "mods"
  | "order"
  | "conflicts"
  | "files"
  | "downloads"
  | "collections"
  | "settings";

interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  message: string;
}

/** Progress of a batch install, so the status bar can say "3 of 12". */
export interface Busy {
  label: string;
  done: number;
  total: number;
}

interface AppStore {
  ready: boolean;
  view: View;
  snapshot: AppSnapshot | null;
  selection: number[];
  search: string;
  listView: ListView;
  setListView: (change: Partial<ListView>) => void;
  busy: Busy | null;
  /** Archives the detector could not classify, waiting to be asked about. */
  queue: StagedInstall[];
  /** Nexus downloads, in whatever state the backend last reported. */
  downloads: QueueItem[];
  /** A collection link that arrived from the browser, waiting to be looked up. */
  pendingCollection: string | null;
  /** Archives that appeared in the watched downloads folder, not yet answered. */
  found: FoundArchive[];
  /** Which enabled mods replace the same assets. Null until first read. */
  conflicts: ConflictReport | null;
  toasts: Toast[];

  init: () => Promise<void>;
  /** Show what a shortcut or command line did, and the state it left. */
  takeNotices: () => Promise<void>;
  refresh: () => Promise<void>;
  refreshDownloads: () => Promise<void>;
  /** Apply a progress event without a round trip to the backend. */
  patchDownload: (item: QueueItem) => void;
  addNxmLink: (url: string) => Promise<void>;
  cancelDownload: (id: number) => Promise<void>;
  clearFinishedDownloads: () => Promise<void>;
  setPendingCollection: (slug: string | null) => void;
  refreshConflicts: () => Promise<void>;
  reindexAssets: () => Promise<void>;
  setView: (view: View) => void;
  /** The mod the Files view's conflict list is narrowed to, if any. */
  conflictFocus: { modId: number; name: string } | null;
  /** Open the Files view on the clashes one mod takes part in. */
  showFileConflicts: (modId: number, name: string) => void;
  setConflictFocus: (focus: { modId: number; name: string } | null) => void;
  setSearch: (search: string) => void;
  setSelection: (ids: number[]) => void;

  toggle: (ids: number[], enabled: boolean) => Promise<void>;
  reorder: (ids: number[]) => Promise<void>;
  /** Reorder to obey the load order rules; `adding` adds one first. */
  sortByRules: (adding?: { before: number; after: number }) => Promise<void>;
  applyChanges: () => Promise<void>;
  /** Put deployed files changed outside the manager back. */
  repairDrift: () => Promise<void>;
  uninstall: (id: number) => Promise<void>;

  stagePaths: (paths: string[]) => Promise<void>;
  confirmInstall: (name: string, typeOverride?: ModTypeId, variants?: string[]) => Promise<void>;
  /** Install the staged FOMOD at the head of the queue, as the wizard was answered. */
  confirmFomod: (name: string, selection: [number, number, number][]) => Promise<void>;
  /** Ask about an install that arrived from elsewhere — a download that holds several versions. */
  enqueueStaged: (staged: StagedInstall) => void;
  cancelInstall: () => Promise<void>;
  skipRemaining: () => Promise<void>;

  chooseGame: (path: string) => Promise<void>;
  /** New archives in the watched folder: installed straight away, or offered. */
  foundArchives: (archives: FoundArchive[]) => Promise<void>;
  /** Install the offered archives, or with `install` false turn them down. */
  answerFound: (install: boolean) => Promise<void>;
  toast: (kind: Toast["kind"], message: string) => void;
  dismissToast: (id: number) => void;
  dismissAllToasts: () => void;
}

let toastSeq = 0;

export const useApp = create<AppStore>((set, get) => ({
  ready: false,
  view: "mods",
  conflictFocus: null,
  snapshot: null,
  selection: [],
  search: "",
  listView: DEFAULT_LIST_VIEW,
  setListView: (change) => set((state) => ({ listView: { ...state.listView, ...change } })),
  busy: null,
  queue: [],
  downloads: [],
  pendingCollection: null,
  conflicts: null,
  found: [],
  toasts: [],

  async init() {
    await get().refresh();
    await get().refreshDownloads();
    set({ ready: true });
    await get().takeNotices();
  },

  async takeNotices() {
    try {
      const notices = await ipc.takeNotices();
      if (notices.length === 0) return;
      // A profile switched from outside changes the whole list.
      await get().refresh();
      for (const notice of notices) get().toast(notice.kind, notice.message);
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async refresh() {
    try {
      const snapshot = await ipc.snapshot();
      // The conflict picture is a function of what is enabled and in what
      // order, so it is re-read whenever either could have changed.
      void ipc.conflicts().then((conflicts) => set({ conflicts })).catch(() => {});
      set((state) => ({
        snapshot,
        // Drop selections whose mods no longer exist.
        selection: state.selection.filter((id) =>
          snapshot.mods.some((m) => m.id === id),
        ),
      }));
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async refreshDownloads() {
    try {
      set({ downloads: await ipc.downloadQueue() });
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  patchDownload(item) {
    set((state) => ({
      downloads: state.downloads.some((d) => d.id === item.id)
        ? state.downloads.map((d) => (d.id === item.id ? item : d))
        : [...state.downloads, item],
    }));
  },

  async addNxmLink(url) {
    try {
      await ipc.addNxmLink(url);
      await get().refreshDownloads();
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async cancelDownload(id) {
    try {
      await ipc.cancelDownload(id);
      await get().refreshDownloads();
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async clearFinishedDownloads() {
    try {
      await ipc.clearFinishedDownloads();
      await get().refreshDownloads();
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  setPendingCollection: (pendingCollection) => set({ pendingCollection }),

  async refreshConflicts() {
    try {
      set({ conflicts: await ipc.conflicts() });
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  /// Read the containers of mods that have no complete asset list yet.
  async reindexAssets() {
    set({ busy: { label: "Reading mod contents", done: 0, total: 1 } });
    try {
      const looked = await ipc.indexModAssets();
      await get().refreshConflicts();
      get().toast(
        "success",
        looked === 0
          ? "Every mod has already been read."
          : `Read ${looked} mod${looked === 1 ? "" : "s"}.`,
      );
    } catch (error) {
      get().toast("error", String(error));
    } finally {
      set({ busy: null });
    }
  },

  setView: (view) => set({ view }),
  showFileConflicts: (modId, name) => set({ view: "files", conflictFocus: { modId, name } }),
  setConflictFocus: (conflictFocus) => set({ conflictFocus }),
  setSearch: (search) => set({ search }),
  setSelection: (selection) => set({ selection }),

  async toggle(ids, enabled) {
    if (ids.length === 0) return;
    try {
      await ipc.setEnabled(ids, enabled);
      await get().refresh();
      if (get().snapshot?.autoApply) await get().applyChanges();
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async reorder(ids) {
    try {
      await ipc.setOrder(ids);
      await get().refresh();
      if (get().snapshot?.autoApply) await get().applyChanges();
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async sortByRules(adding) {
    try {
      const moved = adding
        ? await ipc.addOrderRule(adding.before, adding.after)
        : await ipc.sortByRules();
      await get().refresh();
      if (moved > 0 && get().snapshot?.autoApply) await get().applyChanges();
      get().toast(
        "success",
        moved > 0
          ? `Load order sorted: ${moved} mod${moved === 1 ? "" : "s"} moved.`
          : adding
            ? "Rule added; the order already obeys it."
            : "The order already obeys every rule.",
      );
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async applyChanges() {
    const snapshot = get().snapshot;
    if (!snapshot?.game) {
      get().toast("error", "Select the Stellar Blade folder first.");
      return;
    }
    set({ busy: { label: "Applying changes", done: 0, total: 1 } });
    try {
      const report = await ipc.apply();
      const changed = report.deployed.length + report.removed.length;
      if (report.failed.length > 0) {
        get().toast(
          "error",
          `${report.failed.length} mod(s) failed: ${report.failed
            .map((f) => `${f.name} — ${f.reason}`)
            .join("; ")}`,
        );
      } else if (changed > 0) {
        get().toast("success", `Applied ${changed} change${changed === 1 ? "" : "s"}.`);
      }
      await get().refresh();
    } catch (error) {
      get().toast("error", String(error));
    } finally {
      set({ busy: null });
    }
  },

  async repairDrift() {
    set({ busy: { label: "Repairing files", done: 0, total: 1 } });
    try {
      const report = await ipc.repairDrift();
      const fixed = report.relinked + report.displaced + report.adopted + report.recorded;
      if (report.failed.length > 0) {
        get().toast(
          "error",
          `${report.failed.length} file(s) could not be repaired: ` +
            report.failed.map((f) => `${f.name} — ${f.reason}`).join("; "),
        );
      } else {
        const notes = [
          report.displaced > 0 &&
            `${report.displaced} replaced by the game kept as its own file again`,
          report.adopted > 0 && `${report.adopted} settings edit${report.adopted === 1 ? "" : "s"} kept in the mod`,
        ].filter(Boolean);
        get().toast(
          "success",
          `Repaired ${fixed} file${fixed === 1 ? "" : "s"}${notes.length ? ` (${notes.join(", ")})` : ""}.`,
        );
      }
      await get().refresh();
    } catch (error) {
      get().toast("error", String(error));
    } finally {
      set({ busy: null });
    }
  },

  async uninstall(id) {
    set({ busy: { label: "Removing", done: 0, total: 1 } });
    try {
      await ipc.uninstall(id);
      await get().refresh();
    } catch (error) {
      get().toast("error", String(error));
    } finally {
      set({ busy: null });
    }
  },

  async stagePaths(paths) {
    // Every path is taken to completion. A single unrecognised archive used to
    // abandon the rest of the batch; now it joins a queue and the others carry
    // on installing.
    const pending: StagedInstall[] = [];
    let installed = 0;
    let failed = 0;

    for (const [index, path] of paths.entries()) {
      set({ busy: { label: basename(path), done: index, total: paths.length } });
      try {
        const staged = looksLikeArchive(path)
          ? await ipc.stageArchive(path)
          : await ipc.stageFolder(path);

        if (staged.needsConfirmation) {
          pending.push(staged);
          continue;
        }
        await ipc.confirmInstall(staged.stagingId, staged.suggestedName);
        installed += 1;
      } catch (error) {
        failed += 1;
        get().toast("error", `${basename(path)}: ${error}`);
      }
    }

    set((state) => ({ busy: null, queue: [...state.queue, ...pending] }));
    await get().refresh();

    if (installed > 0) {
      const waiting = pending.length > 0 ? `, ${pending.length} need a choice` : "";
      const broke = failed > 0 ? `, ${failed} failed` : "";
      get().toast(
        "success",
        `Installed ${installed} mod${installed === 1 ? "" : "s"}${waiting}${broke}.`,
      );
    }
  },

  async confirmFomod(name, selection) {
    const staged = get().queue[0];
    if (!staged) return;
    set({ busy: { label: name, done: 0, total: 1 } });
    try {
      await ipc.confirmInstallFomod(staged.stagingId, name, selection);
      set((state) => ({ queue: state.queue.slice(1) }));
      await get().refresh();
      get().toast("success", `Installed ${name}.`);
    } catch (error) {
      get().toast("error", String(error));
    } finally {
      set({ busy: null });
    }
  },

  enqueueStaged(staged) {
    set((state) => ({ queue: [...state.queue, staged] }));
  },

  async confirmInstall(name, typeOverride, variants) {
    const staged = get().queue[0];
    if (!staged) return;
    set({ busy: { label: name, done: 0, total: 1 } });
    try {
      await ipc.confirmInstall(staged.stagingId, name, typeOverride, variants);
      set((state) => ({ queue: state.queue.slice(1) }));
      await get().refresh();
      get().toast("success", `Installed ${name}.`);
    } catch (error) {
      get().toast("error", String(error));
    } finally {
      set({ busy: null });
    }
  },

  async cancelInstall() {
    const staged = get().queue[0];
    set((state) => ({ queue: state.queue.slice(1) }));
    if (staged) await discard(staged);
  },

  /** Drop every archive still waiting to be asked about. */
  async skipRemaining() {
    const remaining = get().queue;
    set({ queue: [] });
    await Promise.all(remaining.map(discard));
  },

  async chooseGame(path) {
    try {
      await ipc.setGameRoot(path);
      await get().refresh();
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  async foundArchives(archives) {
    const fresh = archives.filter((a) => !get().found.some((f) => f.key === a.key));
    if (fresh.length === 0) return;
    const settings = await ipc.watchSettings().catch(() => null);
    if (settings?.autoInstall) {
      await ipc.markSeen(fresh.map((a) => a.key)).catch(() => {});
      await get().stagePaths(fresh.map((a) => a.path));
    } else {
      set((state) => ({ found: [...state.found, ...fresh] }));
    }
  },

  async answerFound(install) {
    const found = get().found;
    set({ found: [] });
    await ipc.markSeen(found.map((a) => a.key)).catch(() => {});
    if (install) await get().stagePaths(found.map((a) => a.path));
  },

  toast(kind, message) {
    const id = ++toastSeq;
    set((state) => ({ toasts: [...state.toasts, { id, kind, message }] }));
    // Errors stay until dismissed; anything else fades on its own.
    if (kind !== "error") {
      setTimeout(() => get().dismissToast(id), 3500);
    }
  },

  dismissAllToasts() {
    set({ toasts: [] });
  },

  dismissToast(id) {
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },
}));

/** Throw away an extracted-but-uncommitted archive. */
async function discard(staged: StagedInstall): Promise<void> {
  try {
    await ipc.cancelInstall(staged.stagingId);
  } catch {
    // The staged copy is disposable; a failure here is not worth a toast.
  }
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

function looksLikeArchive(path: string): boolean {
  return /\.(zip|7z|rar)$/i.test(path);
}

/** Mods matching the current search, in load order. */
/** Something about the mod wants attention. */
export function hasProblem(mod: ModView): boolean {
  return (
    mod.warnings.length > 0 ||
    mod.missingRequirements.length > 0 ||
    mod.misordered.length > 0 ||
    mod.componentTypes.includes("unknown") ||
    mod.fileConflicts?.redundant === true
  );
}

/** Mods matching the search and the list's filters, in the chosen order. */
export function visibleMods(
  snapshot: AppSnapshot | null,
  search: string,
  view: ListView = DEFAULT_LIST_VIEW,
): ModView[] {
  if (!snapshot) return [];
  const needle = search.trim().toLowerCase();
  const shown = snapshot.mods.filter((m) => {
    // Tags and notes are searched too, so a tag works as a filter.
    if (
      needle &&
      !m.name.toLowerCase().includes(needle) &&
      !m.tags.some((tag) => tag.toLowerCase().includes(needle)) &&
      !(m.notes ?? "").toLowerCase().includes(needle)
    ) {
      return false;
    }
    if (view.type && !m.componentTypes.includes(view.type)) return false;
    if (view.tag && !m.tags.includes(view.tag)) return false;
    switch (view.filter) {
      case "enabled":
        return m.enabled;
      case "disabled":
        return !m.enabled;
      case "updates":
        return m.latestVersion !== null;
      case "problems":
        return hasProblem(m);
      case "pending":
        return m.enabled !== m.deployed;
      default:
        return true;
    }
  });
  if (view.sort === "order") return view.descending ? shown.reverse() : shown;
  const key = (m: ModView): string | number => {
    switch (view.sort) {
      case "name":
        return m.name.toLowerCase();
      case "size":
        return m.sizeBytes;
      case "installed":
        return m.installedAt;
      default:
        return m.componentTypes[0] ?? "";
    }
  };
  const sorted = [...shown].sort((a, b) => {
    const [x, y] = [key(a), key(b)];
    const by = x < y ? -1 : x > y ? 1 : a.name.localeCompare(b.name);
    return view.descending ? -by : by;
  });
  return sorted;
}
