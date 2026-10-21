import { Copy, FilePlus2, Pencil, Trash2, UserCog } from "lucide-react";
import { useState } from "react";

import { ipc, type ProfileView } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Dialog } from "./ui";

type Editing =
  | { kind: "new" }
  | { kind: "duplicate"; from: ProfileView }
  | { kind: "rename"; profile: ProfileView }
  | { kind: "delete"; profile: ProfileView };

/**
 * Pick the active profile, and create, duplicate, rename or delete one.
 *
 * Switching reconciles the game folder with the other profile straight away,
 * so the list the user sees and what the game loads never disagree.
 */
export function ProfileSwitcher() {
  const snapshot = useApp((s) => s.snapshot);
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [name, setName] = useState("");
  const [working, setWorking] = useState(false);

  const profiles = snapshot?.profiles ?? [];
  const active = profiles.find((p) => p.active) ?? null;

  async function run(action: () => Promise<string | void>) {
    setWorking(true);
    try {
      const message = await action();
      await refresh();
      if (message) toast("success", message);
      setEditing(null);
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
    }
  }

  function open(next: Editing) {
    setName(
      next.kind === "rename"
        ? next.profile.name
        : next.kind === "duplicate"
          ? `${next.from.name} (copy)`
          : "",
    );
    setEditing(next);
  }

  async function switchTo(id: number) {
    const target = profiles.find((p) => p.id === id);
    await run(async () => {
      const report = await ipc.switchProfile(id);
      if (report.failed.length > 0) {
        throw new Error(
          `Switched, but ${report.failed.length} mod(s) could not be deployed: ` +
            report.failed.map((f) => `${f.name} — ${f.reason}`).join("; "),
        );
      }
      return `Profile “${target?.name}” is active.`;
    });
  }

  function confirm() {
    if (!editing) return;
    if (editing.kind === "delete") {
      void run(async () => {
        await ipc.deleteProfile(editing.profile.id);
        return `Profile “${editing.profile.name}” deleted.`;
      });
    } else if (editing.kind === "rename") {
      void run(() => ipc.renameProfile(editing.profile.id, name));
    } else {
      const copy = editing.kind === "duplicate";
      void run(async () => {
        const id = await ipc.createProfile(name, copy);
        const report = await ipc.switchProfile(id);
        if (report.failed.length > 0) {
          throw new Error(`Created, but ${report.failed.length} mod(s) could not be deployed.`);
        }
        return `Profile “${name.trim()}” created and active.`;
      });
    }
  }

  const titles: Record<Editing["kind"], string> = {
    new: "New profile",
    duplicate: "Duplicate profile",
    rename: "Rename profile",
    delete: "Delete profile",
  };

  return (
    <div className="flex items-center gap-1">
      <UserCog size={14} className="text-ink-muted" />
      <select
        value={active?.id ?? ""}
        disabled={working}
        onChange={(event) => void switchTo(Number(event.target.value))}
        title="Active profile"
        className="no-drag h-8 w-32 rounded-md xl:w-44 border border-line bg-base px-2 text-[12px] focus:border-accent-soft focus:outline-none"
      >
        {profiles.map((profile) => (
          <option key={profile.id} value={profile.id}>
            {profile.name} ({profile.enabledCount})
          </option>
        ))}
      </select>
      <Button size="sm" title="New profile" disabled={working} onClick={() => open({ kind: "new" })}>
        <FilePlus2 size={13} />
      </Button>
      {active && (
        <>
          <Button
            size="sm"
            title="Duplicate this profile"
            disabled={working}
            onClick={() => open({ kind: "duplicate", from: active })}
          >
            <Copy size={13} />
          </Button>
          <Button
            size="sm"
            title="Rename this profile"
            disabled={working}
            onClick={() => open({ kind: "rename", profile: active })}
          >
            <Pencil size={13} />
          </Button>
        </>
      )}
      {profiles.length > 1 && (
        <Button
          size="sm"
          variant="danger"
          title="Delete a profile"
          disabled={working}
          onClick={() => {
            const other = profiles.find((p) => !p.active);
            if (other) open({ kind: "delete", profile: other });
          }}
        >
          <Trash2 size={13} />
        </Button>
      )}

      <Dialog
        open={editing !== null}
        title={editing ? titles[editing.kind] : ""}
        onClose={() => setEditing(null)}
        width="420px"
        footer={
          <>
            <Button variant="ghost" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button
              variant={editing?.kind === "delete" ? "danger" : "primary"}
              disabled={working || (editing?.kind !== "delete" && name.trim() === "")}
              onClick={confirm}
            >
              {editing?.kind === "delete" ? "Delete" : editing?.kind === "rename" ? "Rename" : "Create"}
            </Button>
          </>
        }
      >
        {editing?.kind === "delete" ? (
          <div className="space-y-2 text-[12px]">
            <p className="text-ink-muted">
              The active profile cannot be deleted. Pick which other profile to remove; installed
              mods stay installed.
            </p>
            <select
              value={editing.profile.id}
              onChange={(event) => {
                const profile = profiles.find((p) => p.id === Number(event.target.value));
                if (profile) setEditing({ kind: "delete", profile });
              }}
              className="h-8 w-full rounded-md border border-line bg-base px-2 text-[12px]"
            >
              {profiles
                .filter((p) => !p.active)
                .map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
            </select>
          </div>
        ) : (
          <div className="space-y-2">
            <input
              autoFocus
              value={name}
              onChange={(event) => setName(event.target.value)}
              onKeyDown={(event) => event.key === "Enter" && name.trim() && confirm()}
              placeholder="Profile name"
              className="h-8 w-full rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
            />
            <p className="text-[11px] leading-relaxed text-ink-muted">
              {editing?.kind === "new" &&
                "Keeps the current load order with every mod disabled. It becomes active straight away."}
              {editing?.kind === "duplicate" &&
                "A copy of the current profile, with the same mods enabled. It becomes active straight away."}
              {editing?.kind === "rename" && "Only the name changes."}
            </p>
          </div>
        )}
      </Dialog>
    </div>
  );
}
