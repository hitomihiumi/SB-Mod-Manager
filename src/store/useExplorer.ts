import { create } from "zustand";

/** What a tab of the explorer shows. */
export type ExplorerKind = "game" | "library" | "conflicts";

export interface ExplorerTab {
  id: number;
  kind: ExplorerKind;
  /** Game-relative for `game`, mods-folder-relative for `library`. */
  dir: string;
}

interface ExplorerState {
  tabs: ExplorerTab[];
  activeId: number;
  nextId: number;
  /** Library paths set aside with Cut, waiting for Paste in some folder. */
  clipboard: string[];
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
  setClipboard: (paths: string[]) => void;
  setModsDir: (dir: string) => void;
  setDrop: (drop: { dir: string; key: string } | null) => void;
}

/**
 * Tabs live outside the screen so that going to another screen and coming
 * back finds them where they were left.
 */
export const useExplorer = create<ExplorerState>((set) => ({
  tabs: [{ id: 1, kind: "game", dir: "" }],
  activeId: 1,
  nextId: 2,
  clipboard: [],
  modsDir: null,
  drop: null,

  openTab: (kind, dir = "") =>
    set((s) => ({
      tabs: [...s.tabs, { id: s.nextId, kind, dir }],
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
        return { tabs: [{ id: s.nextId, kind: "game", dir: "" }], activeId: s.nextId, nextId: s.nextId + 1 };
      }
      const activeId = s.activeId === id ? rest[Math.min(index, rest.length - 1)]!.id : s.activeId;
      return { tabs: rest, activeId };
    }),

  activate: (activeId) => set({ activeId }),

  show: (id, kind, dir = "") =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, kind, dir } : t)) })),

  setClipboard: (clipboard) => set({ clipboard }),
  setModsDir: (modsDir) => set({ modsDir }),
  setDrop: (drop) => set({ drop }),
}));

/** The folder above `dir`, or the top. */
export function parentDir(dir: string): string {
  const cut = dir.lastIndexOf("/");
  return cut === -1 ? "" : dir.slice(0, cut);
}
