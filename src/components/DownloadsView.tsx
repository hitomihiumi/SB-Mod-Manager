import clsx from "clsx";
import {
  CheckCircle2,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc, type DownloadState, type QueueItem, type RateLimit } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Empty } from "./ui";

export function DownloadsView() {
  const downloads = useApp((s) => s.downloads);
  const snapshot = useApp((s) => s.snapshot);
  const refreshDownloads = useApp((s) => s.refreshDownloads);
  const cancelDownload = useApp((s) => s.cancelDownload);
  const clearFinished = useApp((s) => s.clearFinishedDownloads);
  const addNxmLink = useApp((s) => s.addNxmLink);

  const [link, setLink] = useState("");
  const [limit, setLimit] = useState<RateLimit | null>(null);

  const hasKey = snapshot?.nexus != null;
  const finished = downloads.filter((d) => !isOpen(d.state)).length;

  useEffect(() => {
    if (!hasKey) return;
    // Cheap and only informational, so a failure is not worth a toast.
    ipc.nexusRateLimit().then(setLimit, () => setLimit(null));
  }, [hasKey, downloads.length]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line px-3">
        <h1 className="text-[13px] font-semibold">Downloads</h1>
        {limit?.dailyRemaining != null && (
          <span className="text-[11px] text-ink-muted" title="Nexus API allowance">
            {limit.dailyRemaining} requests left today
          </span>
        )}
        <div className="ml-auto flex items-center gap-1.5">
          <Button size="sm" variant="ghost" onClick={() => void refreshDownloads()}>
            <RefreshCw size={13} /> Refresh
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={finished === 0}
            onClick={() => void clearFinished()}
          >
            <Trash2 size={13} /> Clear finished
          </Button>
        </div>
      </header>

      {!hasKey && (
        <p className="border-b border-line bg-warn/10 px-3 py-2 text-[12px] text-warn">
          Add your Nexus API key in Settings before downloading — even a free
          account needs one to ask for a file.
        </p>
      )}

      <form
        className="flex shrink-0 items-center gap-2 border-b border-line px-3 py-2.5"
        onSubmit={(event) => {
          event.preventDefault();
          const url = link.trim();
          if (!url) return;
          setLink("");
          void addNxmLink(url);
        }}
      >
        <input
          value={link}
          onChange={(event) => setLink(event.target.value)}
          placeholder="Paste an nxm:// link"
          spellCheck={false}
          className="h-8 min-w-0 flex-1 rounded-md border border-line-strong bg-base px-2.5 font-mono text-[12px] outline-none focus:border-accent"
        />
        <Button type="submit" size="sm" variant="outline" disabled={!link.trim()}>
          Add
        </Button>
      </form>

      {downloads.length === 0 ? (
        <Empty
          icon={<Download size={20} />}
          title="Nothing downloading"
          hint="Press “Mod Manager Download” on a Nexus mod page and it lands here, then installs itself. Links can also be pasted above."
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          {downloads.map((item) => (
            <DownloadRow key={item.id} item={item} onCancel={() => void cancelDownload(item.id)} />
          ))}
        </div>
      )}
    </div>
  );
}

function DownloadRow({ item, onCancel }: { item: QueueItem; onCancel: () => void }) {
  const total = item.bytesTotal ?? 0;
  const percent = total > 0 ? Math.min(100, (item.bytesDone / total) * 100) : 0;

  return (
    <div className="border-b border-line/70 px-3 py-2.5">
      <div className="flex items-center gap-2">
        <StateIcon state={item.state} />
        <span className="min-w-0 flex-1 truncate text-[13px]">{item.name}</span>
        {item.collection && <Badge tone="accent">{item.collection}</Badge>}
        <StateBadge state={item.state} />
        {isOpen(item.state) && (
          <button
            onClick={onCancel}
            aria-label={`Cancel ${item.name}`}
            className="shrink-0 text-ink-muted hover:text-danger"
          >
            <X size={14} />
          </button>
        )}
      </div>

      <p className="mt-0.5 truncate pl-6 font-mono text-[11px] text-ink-muted">
        {item.fileName}
      </p>

      {item.state === "running" && (
        <div className="mt-1.5 flex items-center gap-2 pl-6">
          <div className="h-1 min-w-0 flex-1 overflow-hidden rounded-full bg-hover">
            <div
              className="h-full rounded-full bg-accent transition-[width]"
              style={{ width: total > 0 ? `${percent}%` : "35%" }}
            />
          </div>
          <span className="shrink-0 text-[11px] tabular-nums text-ink-muted">
            {formatBytes(item.bytesDone)}
            {total > 0 ? ` / ${formatBytes(total)}` : ""}
          </span>
        </div>
      )}

      {item.state === "needsUserAction" && (
        <p className="mt-1 flex items-center gap-1.5 pl-6 text-[11px] text-warn">
          <ExternalLink size={11} />
          The mod page is open — press “Mod Manager Download” there and this
          continues on its own.
        </p>
      )}

      {item.error && (
        <p className="mt-1 pl-6 text-[11px] break-words text-danger">{item.error}</p>
      )}
    </div>
  );
}

function StateIcon({ state }: { state: DownloadState }) {
  if (state === "running") {
    return <Loader2 size={14} className="shrink-0 animate-spin text-accent" />;
  }
  if (state === "done") return <CheckCircle2 size={14} className="shrink-0 text-ok" />;
  return (
    <Download
      size={14}
      className={clsx(
        "shrink-0",
        state === "failed" ? "text-danger" : "text-ink-muted",
      )}
    />
  );
}

const STATE_META: Record<DownloadState, { label: string; tone: "neutral" | "accent" | "warn" | "danger" }> = {
  queued: { label: "Queued", tone: "neutral" },
  running: { label: "Downloading", tone: "accent" },
  done: { label: "Installed", tone: "neutral" },
  failed: { label: "Failed", tone: "danger" },
  cancelled: { label: "Cancelled", tone: "neutral" },
  needsUserAction: { label: "Waiting for you", tone: "warn" },
};

function StateBadge({ state }: { state: DownloadState }) {
  const meta = STATE_META[state];
  return <Badge tone={meta.tone}>{meta.label}</Badge>;
}

function isOpen(state: DownloadState): boolean {
  return state === "queued" || state === "running" || state === "needsUserAction";
}
