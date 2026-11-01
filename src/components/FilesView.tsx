import clsx from "clsx";
import { Plus, X } from "lucide-react";
import { useEffect } from "react";

import { ipc } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { useExplorer, type ExplorerKind, type ExplorerTab } from "../store/useExplorer";
import { isAsking } from "./Ask";
import { ConflictsPane } from "./files/ConflictsPane";
import { GamePane } from "./files/GamePane";
import { LibraryPane } from "./files/LibraryPane";

const KINDS: { id: ExplorerKind; label: string; hint: string }[] = [
  { id: "game", label: "Game", hint: "The game folder as the enabled mods make it" },
  { id: "library", label: "Library", hint: "The mod files on disk, which can be moved and renamed" },
  { id: "conflicts", label: "Conflicts", hint: "Paths more than one mod provides" },
];

const titleOf = (tab: ExplorerTab) =>
  tab.kind === "conflicts"
    ? "Conflicts"
    : tab.dir.split("/").pop() || (tab.kind === "game" ? "Game" : "Library");

/**
 * The explorer. Three views of the same files, in as many tabs as are wanted:
 * the game folder the mods add up to, the mod library as it lies on disk
 * (where files can be moved and renamed), and the paths mods fight over.
 * Files open in other programs from any of them.
 *
 * In the library, a drag can end on a folder, on a folder of the path above
 * the list, or on another library tab, so a file can be carried between tabs.
 */
export function FilesView() {
  const gameRoot = useApp((s) => s.snapshot?.game?.root);
  const focus = useApp((s) => s.conflictFocus);
  const tabs = useExplorer((s) => s.tabs);
  const activeId = useExplorer((s) => s.activeId);
  const drop = useExplorer((s) => s.drop);
  const { openTab, closeTab, activate, show, setModsDir } = useExplorer.getState();
  const active = tabs.find((t) => t.id === activeId) ?? tabs[0]!;

  // Where the mods' own files are, so a file seen in the game tree can be found among them.
  useEffect(() => {
    ipc
      .folders()
      .then((folders) => setModsDir(folders.mods))
      .catch(() => {});
  }, [gameRoot, setModsDir]);

  // Arriving from a mod's flag in the list lands on its conflicts.
  useEffect(() => {
    if (!focus) return;
    const existing = useExplorer.getState().tabs.find((t) => t.kind === "conflicts");
    if (existing) useExplorer.getState().activate(existing.id);
    else useExplorer.getState().openTab("conflicts");
  }, [focus]);

  // Ctrl+T opens a tab beside this one, Ctrl+W closes it.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || isAsking()) return;
      if (document.querySelector('[role="dialog"], [role="menu"]')) return;
      if ((event.target as HTMLElement | null)?.closest("input, textarea, select")) return;
      const key = event.key.toLowerCase();
      const state = useExplorer.getState();
      const current = state.tabs.find((t) => t.id === state.activeId);
      if (key === "t" && current) state.openTab(current.kind, current.dir);
      else if (key === "w" && current) state.closeTab(current.id);
      else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div role="tablist" aria-label="Explorer tabs" className="flex shrink-0 items-end gap-1 overflow-x-auto px-5 pt-1">
        {tabs.map((tab) => {
          const current = tab.id === active.id;
          // Another library tab takes a drag: the files go to the folder it shows.
          const takesDrop = tab.kind === "library" && tab.dir !== "";
          return (
            <div
              key={tab.id}
              data-drop-dir={takesDrop ? tab.dir : undefined}
              data-drop-key={takesDrop ? `tab-${tab.id}` : undefined}
              onAuxClick={(event) => event.button === 1 && closeTab(tab.id)}
              className={clsx(
                "group flex h-8 max-w-52 shrink-0 items-center gap-1 pr-1 pl-3 transition-colors duration-150 ease-quiet",
                current ? "text-ink shadow-[inset_0_-1px_var(--color-ink)]" : "text-ink-muted hover:text-ink-soft",
                drop?.key === `tab-${tab.id}` && "bg-hover",
              )}
            >
              <button
                role="tab"
                aria-selected={current}
                onClick={() => activate(tab.id)}
                title={tab.dir ? `${tab.kind}: ${tab.dir}` : tab.kind}
                className="min-w-0 truncate text-[12.5px]"
              >
                {titleOf(tab)}
              </button>
              <button
                aria-label={`Close ${titleOf(tab)}`}
                onClick={() => closeTab(tab.id)}
                className="rounded-sm p-0.5 text-ink-muted opacity-0 transition-[opacity,color] duration-150 ease-quiet group-hover:opacity-100 hover:text-ink focus-visible:opacity-100"
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
        <button
          aria-label="New tab"
          title="New tab beside this one (Ctrl+T)"
          onClick={() => openTab(active.kind, active.dir)}
          className="mb-0.5 ml-1 rounded-sm p-1.5 text-ink-muted transition-colors duration-150 ease-quiet hover:text-ink"
        >
          <Plus size={14} />
        </button>
      </div>

      <div className="flex shrink-0 items-center gap-5 px-5 pt-3 pb-1 text-[13px]">
        {KINDS.map((kind) => (
          <button
            key={kind.id}
            onClick={() => show(active.id, kind.id, kind.id === active.kind ? active.dir : "")}
            aria-pressed={active.kind === kind.id}
            title={kind.hint}
            className={clsx(
              "font-light transition-colors duration-150 ease-quiet",
              active.kind === kind.id ? "text-ink" : "text-ink-muted hover:text-ink-soft",
            )}
          >
            {kind.label}
          </button>
        ))}
      </div>

      {/* Keyed by tab, so switching tabs eases in rather than swapping. */}
      <div key={active.id} className="flex min-h-0 flex-1 animate-fade flex-col">
        {active.kind === "game" && <GamePane tab={active} />}
        {active.kind === "library" && <LibraryPane tab={active} />}
        {active.kind === "conflicts" && <ConflictsPane tab={active} />}
      </div>
    </div>
  );
}
