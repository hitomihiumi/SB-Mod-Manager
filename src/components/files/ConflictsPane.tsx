import { ExternalLink, File as FileIcon, FolderOpen, Swords, X, Loader2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { ipc, type VfsEntry } from "../../lib/ipc";
import { useApp } from "../../store/useApp";
import { useExplorer, type ExplorerTab } from "../../store/useExplorer";
import { Badge, Empty } from "../ui";
import { openContextMenu } from "../ContextMenu";
import { fileMenu, openFile, reveal } from "./fileMenu";
import { OriginTag } from "./GamePane";
import { RowButton } from "./shared";

/**
 * Every path more than one provider claims — or only one mod's, when arriving
 * from that mod's flag in the mod list. Clicking a path shows it in the tree.
 */
export function ConflictsPane({ tab }: { tab: ExplorerTab }) {
  const snapshot = useApp((s) => s.snapshot);
  const toast = useApp((s) => s.toast);
  const focus = useApp((s) => s.conflictFocus);
  const setFocus = useApp((s) => s.setConflictFocus);
  const show = useExplorer((s) => s.show);
  const [conflicts, setConflicts] = useState<VfsEntry[] | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(
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

  useEffect(() => {
    void load(focus?.modId ?? null);
  }, [focus, load, snapshot?.mods, snapshot?.deploymentMode]);

  /** Show a conflicting file where it lives in the tree. */
  const browseTo = (path: string) =>
    show(tab.id, "game", path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "");

  return (
    <>
      <div className="flex shrink-0 items-center gap-2 px-5 py-1.5 text-[12px]">
        {focus ? (
          <span className="flex items-center gap-1.5 bg-surface px-2 py-0.5 text-ink-soft">
            Only “{focus.name}”
            <button
              onClick={() => setFocus(null)}
              title="Show every conflict"
              className="text-ink-muted transition-colors duration-150 ease-quiet hover:text-ink"
            >
              <X size={12} />
            </button>
          </span>
        ) : (
          <span className="text-ink-muted">Every path more than one provider claims</span>
        )}
        {loading && <Loader2 size={13} className="animate-spin text-ink-muted" />}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {conflicts && conflicts.length === 0 ? (
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
          <ul>
            {conflicts?.map((entry) => {
              const folder = entry.path.includes("/")
                ? entry.path.slice(0, entry.path.lastIndexOf("/") + 1)
                : "";
              const loses = focus !== null && entry.providedBy !== focus.name;
              return (
                <li
                  key={entry.path}
                  onClick={() => browseTo(entry.path)}
                  onContextMenu={(event) =>
                    entry.source && openContextMenu(event, fileMenu(entry.source))
                  }
                  title="Show in the tree"
                  className="group flex cursor-pointer items-center gap-2.5 px-5 py-2 text-[13px] transition-colors duration-150 ease-quiet hover:bg-surface"
                >
                  <FileIcon size={15} className="shrink-0 text-ink-muted" />
                  <span className="min-w-0 flex-1 truncate">
                    <span className="text-ink-muted">{folder}</span>
                    {entry.name}
                  </span>
                  {focus !== null && <Badge tone={loses ? "warn" : "accent"}>{loses ? "Loses" : "Wins"}</Badge>}
                  <OriginTag entry={entry} />
                  <span
                    className="max-w-60 shrink-0 truncate text-[11px] text-ink-muted"
                    title={entry.shadowedBy.join(", ")}
                  >
                    hides {entry.shadowedBy.join(", ")}
                  </span>
                  {entry.source && (
                    <>
                      <RowButton label="Open" onClick={() => void openFile(entry.source!)}>
                        <ExternalLink size={13} />
                      </RowButton>
                      <RowButton label="Show the real file" onClick={() => reveal(entry.source!)}>
                        <FolderOpen size={13} />
                      </RowButton>
                    </>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </>
  );
}
