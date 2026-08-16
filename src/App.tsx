import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import clsx from "clsx";
import {
  AlertTriangle,
  Boxes,
  CheckCircle2,
  Download,
  Layers,
  ListOrdered,
  Loader2,
  Settings,
  Swords,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";

import { CollectionsView } from "./components/CollectionsView";
import { ConflictsView } from "./components/ConflictsView";
import { DownloadsView } from "./components/DownloadsView";
import { InstallDialog } from "./components/InstallDialog";
import { LoadOrderView } from "./components/LoadOrderView";
import { ModsView } from "./components/ModsView";
import { SettingsView } from "./components/SettingsView";
import { SetupWizard } from "./components/SetupWizard";
import { TitleBar } from "./components/TitleBar";
import { Button } from "./components/ui";
import type { QueueItem } from "./lib/ipc";
import { useApp, type View } from "./store/useApp";

const NAV: { id: View; label: string; icon: typeof Layers }[] = [
  { id: "mods", label: "Mods", icon: Layers },
  { id: "order", label: "Load order", icon: ListOrdered },
  { id: "conflicts", label: "Conflicts", icon: Swords },
  { id: "downloads", label: "Downloads", icon: Download },
  { id: "collections", label: "Collections", icon: Boxes },
  { id: "settings", label: "Settings", icon: Settings },
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
      if (event.payload.type === "over") setDropping(true);
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
      listen<[string, string]>("download-install-failed", (event) => {
        const [name, reason] = event.payload;
        toast("error", name ? `${name}: ${reason}` : reason);
      }),
      listen<[string, string]>("nxm-rejected", (event) => toast("error", event.payload[1])),
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
        <div className="flex min-h-0 flex-1">
          <nav className="flex w-14 shrink-0 flex-col items-center gap-1 border-r border-line bg-surface py-2">
            {NAV.map((item) => {
              const Icon = item.icon;
              const count = item.id === "downloads" ? active : 0;
              return (
                <button
                  key={item.id}
                  onClick={() => setView(item.id)}
                  title={item.label}
                  aria-label={item.label}
                  aria-current={view === item.id}
                  className={clsx(
                    "relative grid size-10 place-items-center rounded-lg transition-colors",
                    view === item.id
                      ? "bg-hover text-ink"
                      : "text-ink-muted hover:bg-hover/60 hover:text-ink-soft",
                  )}
                >
                  {view === item.id && (
                    <span className="absolute top-1/2 -left-2 h-5 w-[3px] -translate-y-1/2 rounded-r bg-accent" />
                  )}
                  <Icon size={17} />
                  {count > 0 && (
                    <span className="absolute top-1 right-1 grid h-4 min-w-4 place-items-center rounded-full bg-accent px-1 text-[9px] font-bold text-accent-ink tabular-nums">
                      {count}
                    </span>
                  )}
                </button>
              );
            })}
          </nav>

          <main className="flex min-w-0 flex-1 flex-col">
            {view === "mods" && <ModsView onAdd={pickFiles} />}
            {view === "order" && <LoadOrderView />}
            {view === "conflicts" && <ConflictsView />}
            {view === "downloads" && <DownloadsView />}
            {view === "collections" && <CollectionsView />}
            {view === "settings" && <SettingsView />}
            <ApplyBar />
          </main>
        </div>
      )}

      {dropping && (
        <div className="pointer-events-none fixed inset-0 z-40 grid place-items-center bg-base/80">
          <div className="rounded-xl border-2 border-dashed border-accent px-8 py-6 text-center">
            <p className="text-[14px] font-semibold">Drop to install</p>
            <p className="mt-1 text-[12px] text-ink-muted">.zip, .7z, .rar or a mod folder</p>
          </div>
        </div>
      )}

      {busy && (
        <div className="fixed inset-x-0 bottom-0 z-40 flex items-center gap-2 border-t border-line bg-surface px-4 py-2 text-[12px] text-ink-soft">
          <Loader2 size={13} className="animate-spin" />
          <span className="truncate">{busy.label}</span>
          {busy.total > 1 && (
            <span className="ml-auto shrink-0 tabular-nums text-ink-muted">
              {busy.done + 1} of {busy.total}
            </span>
          )}
        </div>
      )}

      <InstallDialog />
      <Toasts />
    </div>
  );
}

/** The staged-changes bar. Hidden entirely when auto-apply is on and nothing is queued. */
function ApplyBar() {
  const snapshot = useApp((s) => s.snapshot);
  const applyChanges = useApp((s) => s.applyChanges);

  const pending = snapshot?.pending ?? [];
  const drift = snapshot?.drift ?? [];
  if (pending.length === 0 && drift.length === 0) return null;

  return (
    <div className="flex h-11 shrink-0 items-center gap-3 border-t border-line bg-raised px-3">
      {drift.length > 0 && (
        <span className="flex items-center gap-1.5 text-[12px] text-warn">
          <AlertTriangle size={13} />
          {drift.length} deployed file{drift.length === 1 ? "" : "s"} changed outside the manager
        </span>
      )}
      {pending.length > 0 && (
        <>
          <span className="text-[12px] text-ink-soft">
            {pending.length} pending change{pending.length === 1 ? "" : "s"}
          </span>
          <span className="truncate text-[11px] text-ink-muted">
            {pending
              .slice(0, 3)
              .map((p) => `${p.kind} ${p.name}`)
              .join(", ")}
            {pending.length > 3 ? "…" : ""}
          </span>
        </>
      )}
      <div className="ml-auto" />
      <Button variant="primary" size="sm" onClick={() => void applyChanges()}>
        <CheckCircle2 size={13} /> Apply
      </Button>
    </div>
  );
}

function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismissToast);

  if (toasts.length === 0) return null;
  return (
    <div className="fixed right-4 bottom-14 z-50 flex w-80 flex-col gap-2">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          className={clsx(
            "flex items-start gap-2 rounded-lg border px-3 py-2.5 shadow-lg",
            toast.kind === "error"
              ? "border-danger/40 bg-danger/10"
              : toast.kind === "success"
                ? "border-ok/40 bg-ok/10"
                : "border-line bg-raised",
          )}
        >
          <p className="min-w-0 flex-1 text-[12px] leading-relaxed break-words">
            {toast.message}
          </p>
          <button
            onClick={() => dismiss(toast.id)}
            className="shrink-0 text-ink-muted hover:text-ink"
            aria-label="Dismiss"
          >
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
