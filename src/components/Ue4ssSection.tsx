import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Download, ExternalLink, Puzzle } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Section } from "./ui";

/** Install or update UE4SS, with the outcome toasted and the list refreshed. */
export function useInstallUe4ss() {
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [installing, setInstalling] = useState(false);

  async function install() {
    setInstalling(true);
    try {
      toast("success", await ipc.installUe4ss());
      await refresh();
    } catch (error) {
      toast("error", String(error));
    } finally {
      setInstalling(false);
    }
  }
  return { install, installing };
}

const STATE_LABEL = {
  managed: { tone: "accent", text: "Installed" },
  disabled: { tone: "warn", text: "Installed, switched off" },
  manual: { tone: "neutral", text: "Installed by hand" },
  missing: { tone: "danger", text: "Not installed" },
} as const;

/**
 * UE4SS: whether the game will load it, which mods need it, and the Stellar
 * Blade build to install or update to.
 */
export function Ue4ssSection() {
  const snapshot = useApp((s) => s.snapshot);
  const { install, installing } = useInstallUe4ss();
  const [latest, setLatest] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    void ipc.ue4ssLatest().then(setLatest).catch(() => setLatest(null));
  }, []);

  const status = snapshot?.ue4ss;
  if (!status) return null;
  const label = STATE_LABEL[status.state];
  const outdated =
    status.state === "managed" && latest != null && status.version !== null && latest !== status.version;
  const canInstall = latest != null && (status.state !== "managed" || outdated);

  return (
    <Section
      title="UE4SS"
      hint="The script loader Lua, C++ and Blueprint logic mods run on. Stellar Blade needs its own build of it (Chrisr0's fork of RE-UE4SS), which is what is installed here."
    >
      <div className="flex flex-wrap items-center gap-2">
        <Puzzle size={15} className="text-ink-muted" />
        <Badge tone={label.tone}>{label.text}</Badge>
        {status.version && <span className="text-[12px] text-ink-soft">version {status.version}</span>}
        {outdated && <Badge tone="warn">{latest} available</Badge>}
        <span className="flex-1" />
        <Button size="sm" variant="ghost" onClick={() => void openUrl(status.projectUrl)}>
          <ExternalLink size={12} /> Project page
        </Button>
        <Button
          size="sm"
          variant={canInstall ? "primary" : "outline"}
          disabled={installing || !canInstall}
          onClick={() => void install()}
        >
          <Download size={12} />
          {installing
            ? "Installing…"
            : latest === undefined
              ? "Checking…"
              : latest === null
                ? "Can't check"
                : status.state === "managed"
                  ? outdated
                    ? `Update to ${latest}`
                    : "Up to date"
                  : `Install ${latest}`}
        </Button>
      </div>

      {status.state === "manual" && (
        <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">
          A UE4SS the manager did not install is in the game folder. Installing here sets it aside
          and puts it back if the managed one is removed.
        </p>
      )}
      {status.state === "disabled" && (
        <p className="mt-2 text-[11px] leading-relaxed text-warn">
          The UE4SS mod is switched off in the mod list, so the game will not load it.
        </p>
      )}
      {status.neededBy.length > 0 && (
        <p
          className={
            status.state === "managed" || status.state === "manual"
              ? "mt-2 text-[11px] text-ink-muted"
              : "mt-2 flex items-start gap-1.5 text-[11px] text-danger"
          }
        >
          {status.state !== "managed" && status.state !== "manual" && (
            <AlertTriangle size={12} className="mt-px shrink-0" />
          )}
          Needed by {status.neededBy.join(", ")}.
        </p>
      )}
      {latest === null && (
        <p className="mt-2 text-[11px] text-ink-muted">
          Could not reach GitHub to see what is published.
        </p>
      )}
    </Section>
  );
}

/**
 * Over the mod list when enabled mods need UE4SS and the game will not load
 * it — otherwise they are on, in place, and silently do nothing.
 */
export function Ue4ssBanner() {
  const status = useApp((s) => s.snapshot?.ue4ss);
  const { install, installing } = useInstallUe4ss();
  if (!status || status.neededBy.length === 0) return null;
  if (status.state === "managed" || status.state === "manual") return null;

  const count = status.neededBy.length;
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-danger/30 bg-danger/10 px-3 py-2 text-[12px]">
      <AlertTriangle size={14} className="shrink-0 text-danger" />
      <span className="min-w-0 flex-1">
        {count} enabled mod{count === 1 ? " needs" : "s need"} UE4SS,{" "}
        {status.state === "disabled" ? "which is switched off" : "which is not installed"} — until it
        is, {count === 1 ? "it does" : "they do"} nothing.
      </span>
      {status.state === "missing" && (
        <Button size="sm" variant="primary" disabled={installing} onClick={() => void install()}>
          <Download size={12} /> {installing ? "Installing…" : "Install UE4SS"}
        </Button>
      )}
    </div>
  );
}
