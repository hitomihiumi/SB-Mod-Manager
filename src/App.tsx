import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import { AlertTriangle, Download, Loader2, Play, Wrench, X } from "lucide-react";
import { useEffect, useState } from "react";

import { AskHost, isAsking } from "./components/Ask";
import { CollectionsView } from "./components/CollectionsView";
import { ContextMenuHost } from "./components/ContextMenu";
import { ConflictsView } from "./components/ConflictsView";
import { DownloadsView } from "./components/DownloadsView";
import { FilesView } from "./components/FilesView";
import { InstallDialog } from "./components/InstallDialog";
import { LoadOrderView } from "./components/LoadOrderView";
import { ModsView } from "./components/ModsView";
import { ReshadeView } from "./components/ReshadeView";
import { SettingsView } from "./components/SettingsView";
import { SetupWizard } from "./components/SetupWizard";
import { TitleBar } from "./components/TitleBar";
import { Button, Dialog } from "./components/ui";
import { ipc, type FoundArchive, type QueueItem, type StagedInstall } from "./lib/ipc";
import { useApp, type View } from "./store/useApp";

// Words only: the screens are named plainly, and Ctrl+1…8 reach them in this order.
const NAV: { id: View; label: string }[] = [
  { id: "mods", label: "Mods" },
  { id: "order", label: "Load order" },
  { id: "conflicts", label: "Conflicts" },
  { id: "files", label: "Files" },
  { id: "downloads", label: "Downloads" },
  { id: "collections", label: "Collections" },
  { id: "reshade", label: "ReShade" },
  { id: "settings", label: "Settings" },
];

export default function App() {
  const ready = useApp((s) => s.ready);
  const init = useApp((s) => s.init);
  const snapshot = useApp((s) => s.snapshot);
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const stagePaths = useApp((s) => s.stagePaths);
  const busy = useApp((s) => s.busy);
  const downloads = useApp((s) => s.downloads);

  const [dropping, setDropping] = useState(false);
  const [shortcuts, setShortcuts] = useState(false);

  // Anything still in flight, so the rail can say how many.
  const active = downloads.filter(
    (d) => d.state === "queued" || d.state === "running" || d.state === "needsUserAction",
  ).length;

  useEffect(() => {
    void init();
  }, [init]);

  // Dropping archives onto the window is the primary way to install.
  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      // The window reports every drag, including a row being dragged inside it.
      // Only a drag that carries files is an install, so only that shows the
      // overlay; "over" fires continuously and says nothing about the content.
      if (event.payload.type === "enter") setDropping(event.payload.paths.length > 0);
      else if (event.payload.type === "leave") setDropping(false);
      else if (event.payload.type === "drop") {
        setDropping(false);
        void stagePaths(event.payload.paths);
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [stagePaths]);

  // Downloads run in the background, so the window is told about them rather
  // than polling for changes.
  useEffect(() => {
    const { patchDownload, refreshDownloads, refresh, toast, setView } = useApp.getState();

    const subscriptions = [
      listen<QueueItem>("download-progress", (event) => patchDownload(event.payload)),
      listen("downloads-changed", () => {
        void refreshDownloads();
        void refresh();
      }),
      listen<QueueItem>("download-needs-action", (event) => {
        void refreshDownloads();
        setView("downloads");
        toast(
          "info",
          `${event.payload.name}: press “Mod Manager Download” on the page that just opened.`,
        );
      }),
      // A download holding several versions waits in the same queue as a
      // dropped archive that needs a choice.
      listen<StagedInstall>("download-needs-choice", (event) => {
        useApp.getState().enqueueStaged(event.payload);
      }),
      listen<[string, string]>("download-install-failed", (event) => {
        const [name, reason] = event.payload;
        toast("error", name ? `${name}: ${reason}` : reason);
      }),
      listen<[string, string]>("nxm-rejected", (event) => toast("error", event.payload[1])),
      // A mod archive finished downloading into the watched folder.
      listen<FoundArchive[]>("watch-found", (event) => {
        void useApp.getState().foundArchives(event.payload);
      }),
      // A shortcut or a second start asked for a profile or the game.
      listen("notices", () => void useApp.getState().takeNotices()),
      // A collection link from the browser goes to the collections screen,
      // which looks it up on arrival.
      listen<{ slug: string }>("nxm-collection", (event) => {
        useApp.getState().setPendingCollection(event.payload.slug);
        setView("collections");
      }),
    ];

    return () => {
      for (const subscription of subscriptions) void subscription.then((off) => off());
    };
  }, []);

  // Shortcuts that work on every screen. Ones about the mod list live with
  // the list; none of them fire while a dialog or a question has the keys.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isAsking() || document.querySelector('[role="dialog"]')) return;
      const target = event.target as HTMLElement | null;
      const typing = !!target?.closest("input, textarea, select, [contenteditable=true]");
      const ctrl = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      const state = useApp.getState();

      if (event.key === "F5") {
        event.preventDefault();
        void state.refresh();
      } else if (event.key === "F1" || (!typing && event.key === "?")) {
        event.preventDefault();
        setShortcuts(true);
      } else if (ctrl && key === "o") {
        event.preventDefault();
        void pickFiles();
      } else if (ctrl && key === "s") {
        event.preventDefault();
        if ((state.snapshot?.pending.length ?? 0) > 0) void state.applyChanges();
      } else if (ctrl && /^[1-9]$/.test(event.key)) {
        const item = NAV[Number(event.key) - 1];
        if (item) {
          event.preventDefault();
          state.setView(item.id);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  async function pickFiles() {
    const picked = await open({
      multiple: true,
      title: "Select mod archives",
      filters: [{ name: "Mod archives", extensions: ["zip", "7z", "rar"] }],
    });
    if (Array.isArray(picked)) await stagePaths(picked);
    else if (typeof picked === "string") await stagePaths([picked]);
  }

  return (
    <div className="flex h-full flex-col">
      <TitleBar />

      {!ready ? (
        <div className="grid flex-1 place-items-center text-ink-muted">
          <Loader2 size={18} className="animate-spin" />
        </div>
      ) : !snapshot?.game ? (
        <SetupWizard />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          <nav className="flex h-11 shrink-0 items-center gap-6 px-5" aria-label="Screens">
            {NAV.map((item, index) => {
              const count = item.id === "downloads" ? active : 0;
              const current = view === item.id;
              return (
                <button
                  key={item.id}
                  onClick={() => setView(item.id)}
                  title={`${item.label} (Ctrl+${index + 1})`}
                  aria-current={current ? "page" : undefined}
                  className={clsx(
                    "flex items-baseline gap-1.5 pt-0.5 text-[14px] font-light transition-colors duration-150 ease-quiet",
                    current ? "text-ink" : "text-ink-muted hover:text-ink-soft",
                  )}
                >
                  {item.label}
                  {count > 0 && <span className="font-mono text-[10.5px] text-ink-soft">{count}</span>}
                </button>
              );
            })}
            <span className="flex-1" />
            <button
              onClick={() => setShortcuts(true)}
              title="Keyboard shortcuts (F1)"
              aria-label="Keyboard shortcuts"
              className="font-mono text-[12px] text-ink-muted transition-colors duration-150 ease-quiet hover:text-ink"
            >
              ?
            </button>
            <PlayButton />
          </nav>

          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            {/* Keyed by screen, so each one eases in rather than appearing. */}
            <div key={view} className="flex min-h-0 flex-1 animate-rise flex-col">
              {view === "mods" && <ModsView onAdd={pickFiles} />}
              {view === "order" && <LoadOrderView />}
              {view === "conflicts" && <ConflictsView />}
              {view === "files" && <FilesView />}
              {view === "downloads" && <DownloadsView />}
              {view === "collections" && <CollectionsView />}
              {view === "reshade" && <ReshadeView />}
              {view === "settings" && <SettingsView />}
            </div>
            <ApplyBar />
          </main>
        </div>
      )}

      {dropping && (
        <div className="pointer-events-none fixed inset-0 z-40 grid animate-fade place-items-center bg-base/85">
          <div className="border border-dashed border-ink-muted px-8 py-6 text-center">
            <p className="text-[15px] font-light">Drop to install</p>
            <p className="mt-1 text-[12px] text-ink-muted">.zip, .7z, .rar or a mod folder</p>
          </div>
        </div>
      )}

      {busy && (
        <div className="fixed inset-x-0 bottom-0 z-40 flex animate-rise items-center gap-2 bg-raised px-5 py-2.5 text-[12px] text-ink-soft">
          <Loader2 size={13} className="animate-spin" />
          <span className="truncate">{busy.label}</span>
          {busy.total > 1 && (
            <span className="ml-auto shrink-0 tabular-nums text-ink-muted">
              {busy.done + 1} of {busy.total}
            </span>
          )}
        </div>
      )}

      <FoundBar />
      <InstallDialog />
      {shortcuts && <ShortcutsDialog onClose={() => setShortcuts(false)} />}
      <AskHost />
      <ContextMenuHost />
      <Toasts />
    </div>
  );
}

/** The staged-changes bar. Hidden entirely when auto-apply is on and nothing is queued. */
function ApplyBar() {
  const snapshot = useApp((s) => s.snapshot);
  const applyChanges = useApp((s) => s.applyChanges);
  const repairDrift = useApp((s) => s.repairDrift);

  const pending = snapshot?.pending ?? [];
  const drift = snapshot?.drift ?? [];
  if (pending.length === 0 && drift.length === 0) return null;

  return (
    <div className="flex h-12 shrink-0 animate-bar items-center gap-4 bg-surface px-5">
      {drift.length > 0 && (
        <span className="flex items-center gap-1.5 text-[12px] text-warn">
          <AlertTriangle size={13} />
          {drift.length} deployed file{drift.length === 1 ? "" : "s"} changed outside the manager
        </span>
      )}
      {pending.length > 0 && (
        <>
          <span className="text-[13px] whitespace-nowrap text-ink">
            {pending.length} change{pending.length === 1 ? "" : "s"} not applied
          </span>
          <span className="min-w-0 truncate text-[12px] text-ink-muted">
            {pending
              .slice(0, 3)
              .map((p) => `${p.kind} ${p.name}`)
              .join(", ")}
            {pending.length > 3 ? "…" : ""}
          </span>
        </>
      )}
      <div className="ml-auto" />
      {drift.length > 0 && (
        <Button
          variant="outline"
          size="sm"
          title="Put the mods' files back. Whatever replaced them is kept: settings edits join the mod, anything else is restored when the mod is disabled."
          onClick={() => void repairDrift()}
        >
          <Wrench size={13} /> Repair
        </Button>
      )}
      {pending.length > 0 && (
        <Button variant="primary" onClick={() => void applyChanges()}>
          Apply
        </Button>
      )}
    </div>
  );
}

/** Mod archives the browser just finished downloading, offered for install. */
function FoundBar() {
  const found = useApp((s) => s.found);
  const answerFound = useApp((s) => s.answerFound);
  if (found.length === 0) return null;
  const names = found.map((a) => a.name);
  return (
    <div className="fixed bottom-14 left-1/2 z-40 flex w-[min(640px,90vw)] -translate-x-1/2 animate-rise items-center gap-3 bg-hover px-4 py-3">
      <Download size={15} className="shrink-0 text-ink-soft" />
      <p className="min-w-0 flex-1 text-[12px] leading-relaxed">
        {found.length === 1 ? "New mod archive" : `${found.length} new mod archives`} in your
        downloads folder:{" "}
        <span className="text-ink-soft" title={names.join("\n")}>
          {names.slice(0, 2).join(", ")}
          {names.length > 2 ? ` and ${names.length - 2} more` : ""}
        </span>
      </p>
      <Button size="sm" onClick={() => void answerFound(false)}>
        Ignore
      </Button>
      <Button size="sm" variant="primary" onClick={() => void answerFound(true)}>
        Install
      </Button>
    </div>
  );
}

function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismissToast);
  const dismissAll = useApp((s) => s.dismissAllToasts);

  if (toasts.length === 0) return null;
  return (
    <div className="fixed right-4 bottom-14 z-50 flex max-h-[70vh] w-80 flex-col gap-2">
      {toasts.length > 1 && (
        <button
          onClick={dismissAll}
          className="self-end bg-hover px-2.5 py-1 text-[11px] text-ink-soft transition-colors duration-150 ease-quiet hover:text-ink"
        >
          Dismiss all ({toasts.length})
        </button>
      )}
      <div className="flex min-h-0 flex-col gap-2 overflow-y-auto">
        {toasts.map((toast) => (
          <div
            key={toast.id}
            // Opaque and one layer up, so whatever is underneath never shows through.
            className="flex shrink-0 animate-rise items-start gap-3 bg-hover px-4 py-3"
          >
            <span
              className={clsx(
                "mt-[7px] size-1.5 shrink-0 rounded-full",
                toast.kind === "error" ? "bg-danger" : toast.kind === "success" ? "bg-ok" : "bg-ink-muted",
              )}
            />
            <p className="min-w-0 flex-1 text-[12px] leading-relaxed break-words">{toast.message}</p>
            <button
              onClick={() => dismiss(toast.id)}
              className="shrink-0 text-ink-muted transition-colors duration-150 ease-quiet hover:text-ink"
              aria-label="Dismiss"
            >
              <X size={13} />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

const SHORTCUTS: { group: string; keys: [string, string][] }[] = [
  {
    group: "Everywhere",
    keys: [
      ["Ctrl+O", "Add mods from archives"],
      ["Ctrl+S", "Apply pending changes"],
      [
        "Ctrl+1 … Ctrl+8",
        "Mods, Load order, Conflicts, Files, Downloads, Collections, ReShade, Settings",
      ],
      ["F5", "Re-read the mod list"],
      ["F1 or ?", "This list"],
    ],
  },
  {
    group: "Mod list",
    keys: [
      ["Ctrl+F or /", "Search"],
      ["↑ ↓, Home, End", "Move the selection (Shift extends it)"],
      ["Ctrl+A", "Select every mod shown"],
      ["Space", "Enable or disable the selection"],
      ["F2", "Rename the selected mod"],
      ["Ctrl+G", "Group the selection"],
      ["Delete", "Uninstall the selection"],
      ["Esc", "Clear the selection (or the search)"],
      ["Right click", "Everything else for a mod"],
    ],
  },
  {
    group: "Settings files",
    keys: [["Ctrl+S", "Save the open file"]],
  },
];

function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  return (
    <Dialog open title="Keyboard shortcuts" width="520px" onClose={onClose}>
      <div className="space-y-4">
        {SHORTCUTS.map((section) => (
          <section key={section.group}>
            <h3 className="mb-2 text-[12px] text-ink-muted">
              {section.group}
            </h3>
            <ul className="space-y-1">
              {section.keys.map(([keys, what]) => (
                <li key={keys} className="flex items-center gap-3 text-[12px]">
                  <kbd className="w-36 shrink-0 bg-base px-1.5 py-0.5 text-center font-mono text-[11px] text-ink-soft">
                    {keys}
                  </kbd>
                  <span className="text-ink-soft">{what}</span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </Dialog>
  );
}

/**
 * Start the game. Under the virtual file system this is the only way the game
 * sees its mods, so it stays in reach on every screen.
 */
function PlayButton() {
  const snapshot = useApp((s) => s.snapshot);
  const toast = useApp((s) => s.toast);
  const refresh = useApp((s) => s.refresh);
  const [starting, setStarting] = useState(false);
  const virtual = snapshot?.deploymentMode === "virtual";
  const label = virtual ? "Play (virtual file system)" : "Play";

  return (
    <button
      onClick={async () => {
        setStarting(true);
        try {
          toast("success", await ipc.launchGame());
          await refresh();
        } catch (error) {
          toast("error", String(error));
        } finally {
          setStarting(false);
        }
      }}
      disabled={starting}
      title={label}
      aria-label={label}
      className="group flex items-center gap-2.5 text-[14px] font-light text-ink disabled:opacity-50"
    >
      <span className="grid size-[26px] place-items-center rounded-full border border-ink-muted transition-[border-color,transform] duration-200 ease-quiet group-hover:border-ink group-active:scale-90">
        {starting ? (
          <Loader2 size={11} className="animate-spin" />
        ) : (
          <Play size={10} fill="currentColor" strokeWidth={0} className="translate-x-px" />
        )}
      </span>
      Play
      {virtual && <span className="font-mono text-[10.5px] text-ink-muted">virtual</span>}
    </button>
  );
}
