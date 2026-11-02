import { create } from "zustand";

/** What a tab of the explorer shows. */
export type ExplorerKind = "game" | "library" | "conflicts";

export interface ExplorerTab {
  id: number;
  kind: ExplorerKind;
  /** Game-relative for `game`, mods-folder-relative for `library`. */
  dir: string;
  /** Folders visited before this one in the tab, for Back. */
  back: string[];
  /** Folders left with Back, for Forward. */
  forward: string[];
}

/** Cut moves what is pasted; Copy leaves the original. */
export type ClipMode = "cut" | "copy";

interface ExplorerState {
  tabs: ExplorerTab[];
  activeId: number;
  nextId: number;
  /** Library paths set aside with Cut or Copy, waiting for Paste in some folder. */
  clipboard: string[];
  clipMode: ClipMode;
  /** Goes up whenever the library changed from outside a pane, so panes reload. */
  revision: number;
  /** The mods folder, to tell where a real file belongs in the library. */
  modsDir: string | null;
  /** The folder a dragged selection is over right now, and which element says so. */
  drop: { dir: string; key: string } | null;

  /** A new tab, which becomes the active one. */
  openTab: (kind: ExplorerKind, dir?: string) => void;
  closeTab: (id: number) => void;
  activate: (id: number) => void;
  /** Point a tab somewhere else, in the same or another view. */
  show: (id: number, kind: ExplorerKind, dir?: string) => void;
  setClipboard: (paths: string[], mode?: ClipMode) => void;
  /** Go to another folder in the same view, remembering where it came from. */
  navigate: (id: number, dir: string) => void;
  goBack: (id: number) => void;
  goForward: (id: number) => void;
  bump: () => void;
  setModsDir: (dir: string) => void;
  setDrop: (drop: { dir: string; key: string } | null) => void;
}

/**
 * Tabs live outside the screen so that going to another screen and coming
 * back finds them where they were left.
 */
export const useExplorer = create<ExplorerState>((set) => ({
  tabs: [{ id: 1, kind: "game", dir: "", back: [], forward: [] }],
  activeId: 1,
  nextId: 2,
  clipboard: [],
  clipMode: "cut",
  revision: 0,
  modsDir: null,
  drop: null,

  openTab: (kind, dir = "") =>
    set((s) => ({
      tabs: [...s.tabs, { id: s.nextId, kind, dir, back: [], forward: [] }],
      activeId: s.nextId,
      nextId: s.nextId + 1,
    })),

  closeTab: (id) =>
    set((s) => {
      const index = s.tabs.findIndex((t) => t.id === id);
      if (index === -1) return s;
      const rest = s.tabs.filter((t) => t.id !== id);
      // The last tab never goes: closing it starts over at the game folder.
      if (rest.length === 0) {
        return {
          tabs: [{ id: s.nextId, kind: "game", dir: "", back: [], forward: [] }],
          activeId: s.nextId,
          nextId: s.nextId + 1,
        };
      }
      const activeId = s.activeId === id ? rest[Math.min(index, rest.length - 1)]!.id : s.activeId;
      return { tabs: rest, activeId };
    }),

  activate: (activeId) => set({ activeId }),

  show: (id, kind, dir = "") =>
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id !== id
          ? t
          : t.kind === kind
            ? t.dir === dir
              ? t
              : { ...t, dir, back: [...t.back, t.dir].slice(-50), forward: [] }
            : { ...t, kind, dir, back: [], forward: [] },
      ),
    })),

  navigate: (id, dir) => {
    const tab = useExplorer.getState().tabs.find((t) => t.id === id);
    if (tab) useExplorer.getState().show(id, tab.kind, dir);
  },

  goBack: (id) =>
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === id && t.back.length > 0
          ? { ...t, dir: t.back[t.back.length - 1]!, back: t.back.slice(0, -1), forward: [t.dir, ...t.forward] }
          : t,
      ),
    })),

  goForward: (id) =>
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === id && t.forward.length > 0
          ? { ...t, dir: t.forward[0]!, back: [...t.back, t.dir], forward: t.forward.slice(1) }
          : t,
      ),
    })),

  bump: () => set((s) => ({ revision: s.revision + 1 })),

  setClipboard: (clipboard, clipMode = "cut") => set({ clipboard, clipMode }),
  setModsDir: (modsDir) => set({ modsDir }),
  setDrop: (drop) => set({ drop }),
}));

/** The folder above `dir`, or the top. */
export function parentDir(dir: string): string {
  const cut = dir.lastIndexOf("/");
  return cut === -1 ? "" : dir.slice(0, cut);
}
