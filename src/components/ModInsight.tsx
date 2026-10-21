import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import { ArrowDownUp, Check, ExternalLink, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import { formatDate } from "../lib/format";
import {
  ipc,
  modPageUrl,
  type ModContents,
  type ModView,
  type NexusView,
  type OrderRule,
} from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button } from "./ui";

function Heading({ children, action }: { children: string; action?: React.ReactNode }) {
  return (
    <div className="mb-1.5 flex items-center gap-2">
      <h4 className="flex-1 text-[12px] font-medium">{children}</h4>
      {action}
    </div>
  );
}

const STATE: Record<string, { label: string; tone: "accent" | "warn" | "danger" | "neutral" }> = {
  enabled: { label: "Installed", tone: "accent" },
  disabled: { label: "Disabled", tone: "warn" },
  missing: { label: "Missing", tone: "danger" },
  external: { label: "Elsewhere", tone: "neutral" },
};

/**
 * What the mod's Nexus page says: author, summary and tags, and the mods it
 * needs, each checked against what is installed. Asked again when the
 * answer is a day old.
 */
export function NexusSection({ mod }: { mod: ModView }) {
  const toggle = useApp((s) => s.toggle);
  const toast = useApp((s) => s.toast);
  const [view, setView] = useState<NexusView | null>(null);
  const [loading, setLoading] = useState(false);

  async function load(force: boolean) {
    setLoading(true);
    try {
      setView(await ipc.refreshNexusDetails(mod.id, force));
    } catch (error) {
      // Offline or rate limited: whatever was fetched before still shows.
      setView(await ipc.nexusView(mod.id).catch(() => null));
      if (force) toast("error", String(error));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    setView(null);
    if (mod.nexusModId !== null) void load(false);
  }, [mod.id]);

  // Requirement states follow what is enabled.
  useEffect(() => {
    if (view) void ipc.nexusView(mod.id).then(setView).catch(() => {});
  }, [mod.enabled, mod.missingRequirements.join()]);

  if (mod.nexusModId === null) return null;
  const details = view?.details ?? null;

  return (
    <section>
      <Heading
        action={
          <button
            onClick={() => void load(true)}
            disabled={loading}
            title={view?.fetchedAt ? `Fetched ${formatDate(view.fetchedAt)}` : "Fetch from Nexus"}
            className="text-ink-muted hover:text-ink disabled:opacity-40"
            aria-label="Refresh from Nexus"
          >
            <RefreshCw size={12} className={clsx(loading && "animate-spin")} />
          </button>
        }
      >
        On Nexus
      </Heading>
      {!details ? (
        <p className="text-[11px] text-ink-muted">
          {loading ? "Asking Nexus…" : "Not fetched yet."}
        </p>
      ) : (
        <div className="space-y-2">
          <p className="text-[11px] text-ink-muted">
            {details.author ? `by ${details.author} · ` : ""}
            {details.endorsements.toLocaleString()} endorsements ·{" "}
            {details.downloads.toLocaleString()} downloads
            {details.updatedAt ? ` · updated ${formatDate(details.updatedAt)}` : ""}
          </p>
          {details.summary && (
            <p className="text-[12px] leading-relaxed text-ink-soft">{details.summary}</p>
          )}
          {(details.category || details.tags.length > 0) && (
            <div className="flex flex-wrap gap-1">
              {details.category && <Badge tone="accent">{details.category}</Badge>}
              {details.tags.map((tag) => (
                <Badge key={tag}>{tag}</Badge>
              ))}
            </div>
          )}
          {view && view.requirements.length > 0 && (
            <div>
              <p className="mb-1 text-[11px] font-medium text-ink-soft">Requires</p>
              <ul className="space-y-1">
                {view.requirements.map((req) => {
                  const state = STATE[req.state] ?? STATE.external!;
                  return (
                    <li
                      key={`${req.modId}:${req.name}`}
                      className="flex items-center gap-2 rounded-md border border-line px-2 py-1.5"
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[12px]" title={req.name}>
                          {req.name}
                        </span>
                        {req.notes && (
                          <span className="block text-[10px] text-ink-muted">{req.notes}</span>
                        )}
                      </span>
                      <Badge tone={state.tone}>{state.label}</Badge>
                      {req.state === "disabled" && req.installedAs !== null && (
                        <Button size="sm" onClick={() => void toggle([req.installedAs!], true)}>
                          Enable
                        </Button>
                      )}
                      {(req.state === "missing" || req.state === "external") && (req.modId !== null || req.url) && (
                        <button
                          title="Open its page"
                          onClick={() =>
                            void openUrl(req.modId !== null ? modPageUrl(req.modId) : req.url)
                          }
                          className="text-ink-muted hover:text-ink"
                        >
                          <ExternalLink size={12} />
                        </button>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

/** Where the mod's changes land, and which other mods change the same things. */
export function ContentsSection({ mod }: { mod: ModView }) {
  const snapshot = useApp((s) => s.snapshot);
  const [contents, setContents] = useState<ModContents | null>(null);

  useEffect(() => {
    ipc
      .modContents(mod.id)
      .then(setContents)
      .catch(() => setContents(null));
    // The clash picture changes with the order and the enabled set.
  }, [mod.id, snapshot]);

  if (!contents) return null;
  const others = contents.clashes;

  return (
    <section>
      <Heading>What it changes</Heading>
      <p className="mb-1.5 text-[11px] text-ink-muted">
        {contents.fileCount} file{contents.fileCount === 1 ? "" : "s"} in the game folder
        {contents.indexed
          ? ` · ${contents.assetCount} game asset${contents.assetCount === 1 ? "" : "s"} replaced`
          : contents.fileCount > 0
            ? " · its paks have not been read yet"
            : ""}
      </p>

      {contents.areas.length > 0 && (
        <ul className="mb-2 space-y-0.5">
          {contents.areas.map((area) => (
            <li
              key={area.area}
              title={area.examples.join("\n")}
              className="flex items-center gap-2 text-[11px]"
            >
              <span className="min-w-0 flex-1 truncate font-mono text-ink-soft">{area.area}</span>
              <span className="text-ink-muted tabular-nums">{area.count}</span>
            </li>
          ))}
        </ul>
      )}

      {others.length > 0 && (
        <ul className="mb-2 space-y-1">
          {others.map((clash) => (
            <li key={clash.modId} className="flex items-center gap-2 text-[11px]">
              <span
                className={clsx(
                  "shrink-0 rounded px-1 font-medium",
                  clash.wins ? "bg-ok/15 text-ok" : "bg-danger/15 text-danger",
                )}
              >
                {clash.wins ? "wins over" : "loses to"}
              </span>
              <span
                className={clsx("min-w-0 flex-1 truncate", !clash.enabled && "text-ink-muted")}
                title={clash.name}
              >
                {clash.name}
                {!clash.enabled && " (disabled)"}
              </span>
              <span className="text-ink-muted tabular-nums">{clash.shared}</span>
            </li>
          ))}
        </ul>
      )}

      {contents.files.length > 0 && (
        <details className="text-[11px]">
          <summary className="cursor-pointer text-ink-muted hover:text-ink-soft">
            Files ({contents.fileCount})
          </summary>
          <ul className="mt-1 max-h-40 space-y-0.5 overflow-y-auto">
            {contents.files.map((file) => (
              <li key={file} className="truncate font-mono text-ink-muted" title={file}>
                {file}
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}

/**
 * "This mod loads after that one" — kept whatever else moves, and the list
 * is sorted to obey them as soon as one is added.
 */
export function RulesSection({ mod }: { mod: ModView }) {
  const snapshot = useApp((s) => s.snapshot);
  const sortByRules = useApp((s) => s.sortByRules);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [rules, setRules] = useState<OrderRule[]>([]);
  const [side, setSide] = useState<"after" | "before">("after");
  const [other, setOther] = useState<number | "">("");

  useEffect(() => {
    ipc
      .orderRules()
      .then(setRules)
      .catch(() => setRules([]));
  }, [snapshot]);

  const mine = rules.filter((r) => r.beforeId === mod.id || r.afterId === mod.id);
  const others = (snapshot?.mods ?? []).filter((m) => m.id !== mod.id);

  return (
    <section>
      <Heading>Load order rules</Heading>
      {mine.length > 0 && (
        <ul className="mb-2 space-y-1">
          {mine.map((rule) => {
            const after = rule.afterId === mod.id;
            return (
              <li key={rule.id} className="flex items-center gap-2 text-[11px]">
                {rule.broken ? (
                  <Badge tone="warn">Broken</Badge>
                ) : (
                  <Check size={12} className="shrink-0 text-ok" />
                )}
                <span className="min-w-0 flex-1 truncate">
                  Loads {after ? "after" : "before"}{" "}
                  <span className="text-ink-soft">{after ? rule.before : rule.after}</span>
                </span>
                <button
                  aria-label="Delete rule"
                  onClick={async () => {
                    try {
                      await ipc.deleteOrderRule(rule.id);
                      await refresh();
                    } catch (error) {
                      toast("error", String(error));
                    }
                  }}
                  className="text-ink-muted hover:text-danger"
                >
                  <Trash2 size={11} />
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <div className="flex items-center gap-1.5">
        <select
          value={side}
          onChange={(event) => setSide(event.target.value as "after" | "before")}
          aria-label="Load this"
          className="h-7 rounded-md border border-line bg-base px-1.5 text-[11px]"
        >
          <option value="after">Load after</option>
          <option value="before">Load before</option>
        </select>
        <select
          value={other}
          onChange={(event) => setOther(event.target.value ? Number(event.target.value) : "")}
          aria-label="Which mod"
          className="h-7 min-w-0 flex-1 rounded-md border border-line bg-base px-1.5 text-[11px]"
        >
          <option value="">Choose a mod…</option>
          {others.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
        <Button
          size="sm"
          variant="outline"
          disabled={other === ""}
          onClick={() => {
            if (other === "") return;
            void sortByRules(
              side === "after"
                ? { before: other, after: mod.id }
                : { before: mod.id, after: other },
            );
            setOther("");
          }}
        >
          <ArrowDownUp size={12} /> Add
        </Button>
      </div>
      <p className="mt-1 text-[10px] leading-relaxed text-ink-muted">
        The mod loading later wins where both change the same thing.
      </p>
    </section>
  );
}
