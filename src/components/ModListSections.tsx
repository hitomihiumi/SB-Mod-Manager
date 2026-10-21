import { open, save } from "@tauri-apps/plugin-dialog";
import { Download, FileDown, FileUp, Hand, Check } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc, type ImportPlan, type UpdatableMod } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Checkbox, Dialog, Section } from "./ui";

const FILTERS = [{ name: "Mod list", extensions: ["json"] }];

/** Share a profile as a file, or set one up from a file someone shared. */
export function ModListSection() {
  const snapshot = useApp((s) => s.snapshot);
  const toast = useApp((s) => s.toast);
  const [importing, setImporting] = useState<{ path: string; plan: ImportPlan } | null>(null);

  const active = snapshot?.profiles.find((p) => p.active)?.name ?? "profile";

  async function exportList() {
    const path = await save({
      title: "Save the mod list",
      defaultPath: `${active}.sbmm.json`,
      filters: FILTERS,
    });
    if (!path) return;
    try {
      const count = await ipc.exportModList(path);
      toast("success", `Saved ${count} mods from “${active}”.`);
    } catch (error) {
      toast("error", String(error));
    }
  }

  async function importList() {
    const path = await open({ title: "Open a mod list", filters: FILTERS });
    if (typeof path !== "string") return;
    try {
      setImporting({ path, plan: await ipc.planImport(path) });
    } catch (error) {
      toast("error", String(error));
    }
  }

  return (
    <Section
      title="Mod list"
      hint="Save the active profile — which mods, in what order, which are on, where each came from, and your load order rules — to share or keep. Opening one sets it up as a new profile."
    >
      <div className="flex gap-2">
        <Button variant="outline" onClick={() => void exportList()}>
          <FileDown size={13} /> Save “{active}”…
        </Button>
        <Button variant="outline" onClick={() => void importList()}>
          <FileUp size={13} /> Open a mod list…
        </Button>
      </div>
      {importing && (
        <ImportDialog
          path={importing.path}
          plan={importing.plan}
          onClose={() => setImporting(null)}
        />
      )}
    </Section>
  );
}

const STATUS = {
  installed: { label: "Installed", tone: "accent" as const },
  download: { label: "Download", tone: "warn" as const },
  manual: { label: "By hand", tone: "danger" as const },
};

function ImportDialog({
  path,
  plan,
  onClose,
}: {
  path: string;
  plan: ImportPlan;
  onClose: () => void;
}) {
  const refresh = useApp((s) => s.refresh);
  const refreshDownloads = useApp((s) => s.refreshDownloads);
  const toast = useApp((s) => s.toast);
  const [name, setName] = useState(plan.profile);
  const [working, setWorking] = useState(false);
  const [manual, setManual] = useState<string[] | null>(null);

  const count = (status: string) => plan.entries.filter((e) => e.status === status).length;

  async function run() {
    setWorking(true);
    try {
      const outcome = await ipc.importModList(path, name.trim());
      await refresh();
      await refreshDownloads();
      toast(
        "success",
        `Profile “${outcome.profile}” is active: ${outcome.matched} installed mods put in place` +
          (outcome.downloads.length > 0
            ? `, ${outcome.downloads.length} queued for download — each takes its place when it arrives.`
            : "."),
      );
      if (outcome.manual.length > 0) setManual(outcome.manual);
      else onClose();
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
    }
  }

  if (manual) {
    return (
      <Dialog
        open
        title="Still to get by hand"
        width="480px"
        onClose={onClose}
        footer={
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        }
      >
        <p className="mb-2 text-[12px] text-ink-muted">
          These are not on Nexus, or could not be found there. Install them yourself, then switch
          them on in the new profile.
        </p>
        <ul className="space-y-1">
          {manual.map((entry) => (
            <li key={entry} className="flex items-center gap-2 text-[12px]">
              <Hand size={12} className="text-ink-muted" /> {entry}
            </li>
          ))}
        </ul>
      </Dialog>
    );
  }

  return (
    <Dialog
      open
      title="Open a mod list"
      width="560px"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={working || !name.trim()} onClick={() => void run()}>
            {working ? "Importing…" : "Import as a new profile"}
          </Button>
        </>
      }
    >
      <label className="mb-1.5 block text-[12px] font-medium">Profile name</label>
      <input
        value={name}
        onChange={(event) => setName(event.target.value)}
        className="mb-3 h-8 w-full rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
      />
      <p className="mb-2 text-[12px] text-ink-muted">
        {plan.entries.length} mods: {count("installed")} already installed, {count("download")} to
        download from Nexus, {count("manual")} to get by hand
        {plan.rules > 0 ? ` · ${plan.rules} load order rule${plan.rules === 1 ? "" : "s"}` : ""}.
      </p>
      <ul className="max-h-72 divide-y divide-line overflow-y-auto rounded-md border border-line">
        {plan.entries.map((entry, index) => {
          const status = STATUS[entry.status];
          return (
            <li key={`${entry.name}:${index}`} className="flex items-center gap-2 px-2.5 py-1.5">
              <span className="w-6 text-[11px] text-ink-muted tabular-nums">{index + 1}</span>
              <span className={`min-w-0 flex-1 truncate text-[12px] ${entry.enabled ? "" : "text-ink-muted"}`}>
                {entry.name}
                {entry.version ? <span className="text-ink-muted"> {entry.version}</span> : null}
              </span>
              {!entry.enabled && <span className="text-[10px] text-ink-muted">off</span>}
              <Badge tone={status.tone}>{status.label}</Badge>
            </li>
          );
        })}
      </ul>
      <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">
        The new profile becomes active; your current one is left as it is. Downloads need a Nexus
        API key; with a free account each one opens its page for you to click download.
      </p>
    </Dialog>
  );
}

/** Update the mods Nexus has newer versions of, all at once. */
export function UpdateAllButton() {
  const snapshot = useApp((s) => s.snapshot);
  const refreshDownloads = useApp((s) => s.refreshDownloads);
  const setView = useApp((s) => s.setView);
  const toast = useApp((s) => s.toast);
  const [open_, setOpen] = useState(false);
  const [mods, setMods] = useState<UpdatableMod[]>([]);
  const [chosen, setChosen] = useState<number[]>([]);
  const [working, setWorking] = useState(false);

  const outdated = (snapshot?.mods ?? []).filter((m) => m.latestVersion !== null).length;

  useEffect(() => {
    if (!open_) return;
    ipc
      .updatableMods()
      .then((list) => {
        setMods(list);
        setChosen(list.map((m) => m.id));
      })
      .catch((error) => toast("error", String(error)));
  }, [open_]);

  if (outdated === 0) return null;

  async function run() {
    setWorking(true);
    try {
      const report = await ipc.updateMods(chosen);
      await refreshDownloads();
      const parts = [
        report.queued > 0 && `${report.queued} update${report.queued === 1 ? "" : "s"} queued`,
        report.current.length > 0 && `${report.current.length} already have the newest file`,
        report.failed.length > 0 &&
          `${report.failed.length} failed: ${report.failed.map(([n, r]) => `${n} — ${r}`).join("; ")}`,
      ].filter(Boolean);
      toast(report.failed.length > 0 ? "error" : "success", `${parts.join(", ")}.`);
      setOpen(false);
      if (report.queued > 0) setView("downloads");
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
    }
  }

  return (
    <>
      <Button size="sm" variant="outline" onClick={() => setOpen(true)} title="Update mods from Nexus">
        <Download size={13} /> Update {outdated}
      </Button>
      {open_ && (
        <Dialog
          open
          title="Update from Nexus"
          width="520px"
          onClose={() => setOpen(false)}
          footer={
            <>
              <Button variant="ghost" onClick={() => setOpen(false)}>
                Cancel
              </Button>
              <Button
                variant="primary"
                disabled={working || chosen.length === 0}
                onClick={() => void run()}
              >
                {working ? "Queueing…" : `Update ${chosen.length}`}
              </Button>
            </>
          }
        >
          <p className="mb-3 text-[12px] leading-relaxed text-ink-muted">
            Each mod's newest file of the same kind is downloaded and takes the old version's place:
            enabled where it was, at the same spot in every profile, with its rules, tags, notes and
            your settings edits.
          </p>
          <ul className="space-y-1">
            {mods.map((mod) => (
              <li key={mod.id} className="flex items-center gap-2.5">
                <Checkbox
                  checked={chosen.includes(mod.id)}
                  onChange={(next) =>
                    setChosen((current) =>
                      next ? [...current, mod.id] : current.filter((id) => id !== mod.id),
                    )
                  }
                  label={`Update ${mod.name}`}
                />
                <span className="min-w-0 flex-1 truncate text-[13px]">{mod.name}</span>
                <span className="text-[11px] text-ink-muted tabular-nums">
                  {mod.version ?? "?"} → <span className="text-ink-soft">{mod.latest}</span>
                </span>
                {chosen.includes(mod.id) && <Check size={12} className="text-ok" />}
              </li>
            ))}
          </ul>
        </Dialog>
      )}
    </>
  );
}
