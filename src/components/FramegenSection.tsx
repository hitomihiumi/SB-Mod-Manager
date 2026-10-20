import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import { AlertTriangle, CheckCircle2, Cpu, ExternalLink, Info, Trash2, XCircle, Zap } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import {
  ipc,
  type FramegenCheckState,
  type FramegenProgress,
  type FramegenProxy,
  type FramegenStatus,
} from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Section } from "./ui";

/** Upstream's order: try them top to bottom, one at a time. */
const PROXIES: { id: FramegenProxy; label: string }[] = [
  { id: "version", label: "version.dll (recommended)" },
  { id: "winmm", label: "winmm.dll" },
  { id: "dbghelp", label: "dbghelp.dll" },
  { id: "dinput8", label: "dinput8.dll" },
  { id: "dxgi", label: "dxgi.dll (last resort)" },
  { id: "d3d12", label: "d3d12.dll (last resort)" },
];

const VERDICT: Record<string, { tone: "accent" | "warn" | "danger" | "neutral"; label: string }> = {
  compatible: { tone: "accent", label: "Compatible" },
  compatibleWithWarnings: { tone: "warn", label: "Compatible, read the notes" },
  notNeeded: { tone: "neutral", label: "Not needed on this card" },
  incompatible: { tone: "danger", label: "Not compatible" },
};

/**
 * DLSS Frame Generation on RTX 20 and 30 cards, through the community
 * `dlssg_for_sm86` proxy.
 *
 * Third-party code in the graphics path, so it is marked experimental, sits
 * behind an explicit acknowledgement, and is only offered when the
 * compatibility check passes. It installs through the same deployment record
 * as any mod, so Remove puts the folder back exactly.
 */
export function FramegenSection() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);

  const [status, setStatus] = useState<FramegenStatus | null>(null);
  const [proxy, setProxy] = useState<FramegenProxy | null>(null);
  const [loading, setLoading] = useState(false);
  const [working, setWorking] = useState(false);
  const [progress, setProgress] = useState<FramegenProgress | null>(null);
  const [understood, setUnderstood] = useState(false);

  const load = useCallback(async (choice: FramegenProxy | null) => {
    setLoading(true);
    try {
      const next = await ipc.framegenStatus(choice ?? undefined);
      setStatus(next);
      setProxy(next.proxy);
    } catch {
      setStatus(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (snapshot?.game) void load(null);
  }, [snapshot?.game?.root, load]);

  useEffect(() => {
    const off = listen<FramegenProgress>("framegen-progress", (event) => setProgress(event.payload));
    return () => void off.then((stop) => stop());
  }, []);

  async function run(action: () => Promise<string>) {
    setWorking(true);
    setProgress(null);
    try {
      toast("success", await action());
      await load(proxy);
      await refresh();
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
      setProgress(null);
    }
  }

  const assessment = status?.assessment ?? null;
  const installed = status?.installed ?? null;
  const verdict = assessment ? VERDICT[assessment.verdict] : null;
  const installable =
    assessment?.verdict === "compatible" || assessment?.verdict === "compatibleWithWarnings";
  const switching = installed !== null && proxy !== installed.proxy;
  const outdated =
    installed !== null &&
    status?.latestVersion != null &&
    status.latestVersion !== installed.version;

  return (
    <Section
      title="Frame Generation on RTX 20/30"
      hint="NVIDIA only runs DLSS Frame Generation on RTX 40 and newer. The community project dlssg_for_sm86 makes it run on RTX 20 and 30 cards through a proxy DLL beside the game."
    >
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <Badge tone="warn">Experimental</Badge>
        {verdict && <Badge tone={verdict.tone}>{verdict.label}</Badge>}
        {installed && (
          <Badge tone="accent">
            {installed.enabled ? "Installed" : "Installed, switched off"} · {installed.version}
          </Badge>
        )}
        {outdated && <Badge tone="warn">{status?.latestVersion} available</Badge>}
        <span className="flex-1" />
        {status && (
          <Button size="sm" variant="ghost" onClick={() => void openUrl(status.projectUrl)}>
            <ExternalLink size={12} /> Project page
          </Button>
        )}
      </div>

      {status?.adapter && (
        <p className="mb-3 flex items-center gap-1.5 text-[11px] text-ink-muted">
          <Cpu size={12} />
          {status.adapter.name}
        </p>
      )}

      {loading && !status ? (
        <p className="text-[12px] text-ink-muted">Checking compatibility…</p>
      ) : !assessment ? (
        <p className="text-[12px] text-ink-muted">Choose the game folder to check compatibility.</p>
      ) : (
        <>
          <ul className="space-y-1.5">
            {assessment.checks.map((check) => (
              <li key={check.id} className="flex gap-2 rounded-md border border-line bg-base px-2.5 py-2">
                <StateIcon state={check.state} />
                <div className="min-w-0">
                  <div className="text-[12px] font-medium">{check.title}</div>
                  <p className="text-[11px] leading-relaxed text-ink-muted">{check.detail}</p>
                </div>
              </li>
            ))}
          </ul>

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <label className="flex items-center gap-2 text-[12px]">
              <span className="text-ink-muted">Proxy DLL</span>
              <select
                value={proxy ?? "version"}
                disabled={working}
                onChange={(event) => {
                  const next = event.target.value as FramegenProxy;
                  setProxy(next);
                  void load(next);
                }}
                className="no-drag h-7 rounded-md border border-line-strong bg-base px-2 text-[12px]"
              >
                {PROXIES.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <p className="mt-1 text-[11px] leading-relaxed text-ink-muted">
            Only change this if the game does not load version.dll — frame generation then never
            appears in the game's settings. Try the names in order, one at a time.
          </p>

          {installable && (!installed || switching || outdated) && (
            <label className="mt-3 flex items-start gap-2 text-[12px] leading-relaxed">
              <input
                type="checkbox"
                checked={understood}
                onChange={(event) => setUnderstood(event.target.checked)}
                className="mt-0.5 size-4 accent-[var(--color-accent)]"
              />
              <span>
                I understand this is unofficial third-party code that hooks the game's graphics
                path, that it can crash the game or the driver, and that edits to dlssg_sm86.ini are
                replaced when it is reinstalled.
              </span>
            </label>
          )}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            {installable && (!installed || switching || outdated) && (
              <Button
                size="sm"
                variant="primary"
                disabled={working || !understood || proxy === null}
                onClick={() => proxy && run(() => ipc.installFramegen(proxy))}
              >
                <Zap size={12} />
                {working
                  ? "Working…"
                  : installed
                    ? switching
                      ? "Reinstall with this proxy"
                      : "Update"
                    : "Download and install"}
              </Button>
            )}
            {installed && (
              <Button
                size="sm"
                variant="danger"
                disabled={working}
                onClick={() =>
                  run(async () => {
                    await ipc.removeFramegen();
                    return "Frame generation package removed; the game folder is back as it was.";
                  })
                }
              >
                <Trash2 size={12} /> Remove
              </Button>
            )}
            {working && progress && (
              <span className="text-[11px] text-ink-muted">
                {progress.stage === "verify"
                  ? "Checking the download…"
                  : `Downloading ${formatBytes(progress.done)}${
                      progress.total ? ` of ${formatBytes(progress.total)}` : ""
                    }`}
              </span>
            )}
          </div>
          {status && status.latestVersion === null && (
            <p className="mt-2 text-[11px] text-ink-muted">
              Could not reach GitHub to see what is published.
            </p>
          )}
        </>
      )}
    </Section>
  );
}

function StateIcon({ state }: { state: FramegenCheckState }) {
  const className = clsx(
    "mt-0.5 shrink-0",
    state === "pass" && "text-accent-soft",
    state === "warn" && "text-warn",
    state === "fail" && "text-danger",
    state === "info" && "text-ink-muted",
  );
  if (state === "pass") return <CheckCircle2 size={14} className={className} />;
  if (state === "warn") return <AlertTriangle size={14} className={className} />;
  if (state === "fail") return <XCircle size={14} className={className} />;
  return <Info size={14} className={className} />;
}
