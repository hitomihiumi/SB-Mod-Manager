import clsx from "clsx";
import {
  ExternalLink,
  File as FileIcon,
  FolderOpen,
  Folder,
  Layers,
  Library,
  Loader2,
  Package,
  PenLine,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { ipc, type VfsEntry, type VfsListing } from "../../lib/ipc";
import { useApp } from "../../store/useApp";
import { useExplorer, type ExplorerTab } from "../../store/useExplorer";
import { Badge, Empty } from "../ui";
import { openContextMenu, type MenuItem } from "../ContextMenu";
import { Crumbs, pathInLibrary, RowButton } from "./shared";
import { fileMenu, openFile, reveal } from "./fileMenu";

/**
 * The game folder as the enabled mods make it, in the manner of Mod
 * Organizer 2's "Data" tab: each file attributed to the mod that provides it,
 * the game's own files where nothing touches them, and which mods are hidden
 * beneath the winner. Files open in other programs from here; to move one,
 * find it among the real files with "Show in library".
 */
export function GamePane({ tab }: { tab: ExplorerTab }) {
  const snapshot = useApp((s) => s.snapshot);
  const toast = useApp((s) => s.toast);
  const modsDir = useExplorer((s) => s.modsDir);
  const show = useExplorer((s) => s.show);
  const openTab = useExplorer((s) => s.openTab);
  const [listing, setListing] = useState<VfsListing | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(
    async (dir: string) => {
      setLoading(true);
      try {
        setListing(await ipc.vfsTree(dir));
      } catch (error) {
        toast("error", String(error));
      } finally {
        setLoading(false);
      }
    },
    [toast],
  );

  // Follows the folder, and the enabled mods and their order, which change what is in it.
  useEffect(() => {
    void load(tab.dir);
  }, [tab.dir, load, snapshot?.mods, snapshot?.deploymentMode]);

  const go = (dir: string) => show(tab.id, "game", dir);

  function menuFor(entry: VfsEntry): MenuItem[] {
    if (entry.isDir) {
      return [
        { label: "Open", icon: <Folder size={13} />, onSelect: () => go(entry.path) },
        { label: "Open in a new tab", icon: <ExternalLink size={13} />, onSelect: () => openTab("game", entry.path) },
      ];
    }
    if (!entry.source) return [];
    const inLibrary = pathInLibrary(entry.source, modsDir);
    return [
      ...fileMenu(entry.source),
      ...(inLibrary
        ? ([
            "separator",
            {
              label: "Show in library",
              icon: <Library size={13} />,
              onSelect: () => openTab("library", inLibrary.slice(0, inLibrary.lastIndexOf("/"))),
            },
          ] as MenuItem[])
        : []),
    ];
  }

  if (!snapshot?.game) {
    return (
      <Empty
        icon={<Folder size={22} />}
        title="No game folder"
        hint="Choose the Stellar Blade folder in Settings."
      />
    );
  }

  return (
    <>
      <div className="flex shrink-0 items-center gap-2 px-5 py-1.5">
        <Crumbs root="Game" dir={tab.dir} onGo={go} />
        {loading && <Loader2 size={13} className="animate-spin text-ink-muted" />}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {listing && listing.entries.length === 0 ? (
          <Empty
            icon={<Folder size={22} />}
            title="Nothing here"
            hint={tab.dir ? "This folder is empty in the virtual view." : "No mods are enabled yet."}
          />
        ) : (
          <ul>
            {listing?.entries.map((entry) => (
              <li
                key={entry.path}
                onClick={(event) => {
                  if (!entry.isDir) return;
                  // Ctrl-click or middle-click opens the folder beside this one.
                  if (event.ctrlKey || event.metaKey) openTab("game", entry.path);
                  else go(entry.path);
                }}
                onAuxClick={(event) => {
                  if (event.button === 1 && entry.isDir) openTab("game", entry.path);
                }}
                onDoubleClick={() => !entry.isDir && entry.source && void openFile(entry.source)}
                onContextMenu={(event) => {
                  const items = menuFor(entry);
                  if (items.length > 0) openContextMenu(event, items);
                }}
                className={clsx(
                  "group flex h-9 items-center gap-2.5 px-5 text-[13px] transition-colors duration-150 ease-quiet hover:bg-surface",
                  entry.isDir && "cursor-pointer",
                )}
              >
                {entry.isDir ? (
                  <Folder size={15} className="shrink-0 text-ink-soft" />
                ) : (
                  <FileIcon size={15} className="shrink-0 text-ink-muted" />
                )}
                <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                {!entry.isDir && <OriginTag entry={entry} />}
                {entry.shadowedBy.length > 0 && (
                  <Badge tone="warn" title={`Hides: ${entry.shadowedBy.join(", ")}`}>
                    +{entry.shadowedBy.length} hidden
                  </Badge>
                )}
                {!entry.isDir && entry.source && (
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
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

/** Who provides the winning version. */
export function OriginTag({ entry }: { entry: VfsEntry }) {
  if (entry.origin === "game") {
    return <span className="shrink-0 text-[11px] text-ink-muted">Game</span>;
  }
  const Icon = entry.origin === "overwrite" ? PenLine : entry.origin === "generated" ? Layers : Package;
  return (
    <span className="flex shrink-0 items-center gap-1 text-[11px] text-ink-soft">
      <Icon size={11} />
      <span className="max-w-40 truncate">{entry.providedBy}</span>
    </span>
  );
}
