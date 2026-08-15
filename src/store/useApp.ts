import { create } from "zustand";

import {
  ipc,
  type AppSnapshot,
  type ModTypeId,
  type ModView,
  type StagedInstall,
} from "../lib/ipc";

export type View = "mods" | "order" | "settings";

interface Toast {
  id: number;
  kind: "info" | "error" | "success";
  message: string;
}

interface AppStore {
  ready: boolean;
  view: View;
  snapshot: AppSnapshot | null;
  selection: number[];
  search: string;
  busy: string | null;
  staged: StagedInstall | null;
  toasts: Toast[];

  init: () => Promise<void>;
  refresh: () => Promise<void>;
  setView: (view: View) => void;
  setSearch: (search: string) => void;
  setSelection: (ids: number[]) => void;

  toggle: (ids: number[], enabled: boolean) => Promise<void>;
  reorder: (ids: number[]) => Promise<void>;
  applyChanges: () => Promise<void>;
  uninstall: (id: number) => Promise<void>;

  stagePaths: (paths: string[]) => Promise<void>;
  confirmInstall: (name: string, typeOverride?: ModTypeId) => Promise<void>;
  cancelInstall: () => Promise<void>;

  chooseGame: (path: string) => Promise<void>;
  toast: (kind: Toast["kind"], message: string) => void;
  dismissToast: (id: number) => void;
}

let toastSeq = 0;

export const useApp = create<AppStore>((set, get) => ({
  ready: false,
  view: "mods",
  snapshot: null,
  selection: [],
  search: "",
  busy: null,
  staged: null,
  toasts: [],

  async init() {
    await get().refresh();
    set({ ready: true });
  },

  async refresh() {
    try {
      const snapshot = await ipc.snapshot();
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

  setView: (view) => set({ view }),
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

  async applyChanges() {
    const snapshot = get().snapshot;
    if (!snapshot?.game) {
      get().toast("error", "Select the Stellar Blade folder first.");
      return;
    }
    set({ busy: "Applying changes…" });
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

  async uninstall(id) {
    set({ busy: "Removing…" });
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
    for (const path of paths) {
      set({ busy: `Reading ${basename(path)}…` });
      try {
        const staged = looksLikeArchive(path)
          ? await ipc.stageArchive(path)
          : await ipc.stageFolder(path);

        // Only stop and ask when the detector genuinely could not tell.
        if (staged.needsConfirmation) {
          set({ staged, busy: null });
          return;
        }
        await ipc.confirmInstall(staged.stagingId, staged.suggestedName);
        await get().refresh();
        get().toast("success", `Installed ${staged.suggestedName}.`);
      } catch (error) {
        get().toast("error", `${basename(path)}: ${error}`);
      } finally {
        set({ busy: null });
      }
    }
  },

  async confirmInstall(name, typeOverride) {
    const staged = get().staged;
    if (!staged) return;
    set({ busy: "Installing…" });
    try {
      await ipc.confirmInstall(staged.stagingId, name, typeOverride);
      set({ staged: null });
      await get().refresh();
      get().toast("success", `Installed ${name}.`);
    } catch (error) {
      get().toast("error", String(error));
    } finally {
      set({ busy: null });
    }
  },

  async cancelInstall() {
    const staged = get().staged;
    set({ staged: null });
    if (staged) {
      try {
        await ipc.cancelInstall(staged.stagingId);
      } catch {
        // The staged copy is disposable; a failure here is not worth a toast.
      }
    }
  },

  async chooseGame(path) {
    try {
      await ipc.setGameRoot(path);
      await get().refresh();
    } catch (error) {
      get().toast("error", String(error));
    }
  },

  toast(kind, message) {
    const id = ++toastSeq;
    set((state) => ({ toasts: [...state.toasts, { id, kind, message }] }));
    // Errors stay until dismissed; anything else fades on its own.
    if (kind !== "error") {
      setTimeout(() => get().dismissToast(id), 3500);
    }
  },

  dismissToast(id) {
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },
}));

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

function looksLikeArchive(path: string): boolean {
  return /\.(zip|7z|rar)$/i.test(path);
}

/** Mods matching the current search, in load order. */
export function visibleMods(snapshot: AppSnapshot | null, search: string): ModView[] {
  if (!snapshot) return [];
  const needle = search.trim().toLowerCase();
  if (!needle) return snapshot.mods;
  return snapshot.mods.filter((m) => m.name.toLowerCase().includes(needle));
}
