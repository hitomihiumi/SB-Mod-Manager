import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  AlertTriangle,
  Copy,
  ExternalLink,
  FileCog,
  MoreHorizontal,
  Palette,
  Pencil,
  Pin,
  PinOff,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { formatBytes } from "../lib/format";
import { ipc, type ReshadePreset, type ReshadeView as View } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { askConfirm, askText } from "./Ask";
import { ConfigEditor } from "./ConfigEditor";
import { openContextMenu, type MenuItem } from "./ContextMenu";
import { Badge, Button, Empty } from "./ui";

/**
 * ReShade presets: every one beside the executable, who put it there, which
 * effects it switches on and which of them ReShade will not find — and the
 * one ReShade starts with, for the game and for each profile.
 */
export function ReshadeView() {
  const snapshot = useApp((s) => s.snapshot);
  const toast = useApp((s) => s.toast);
  const [view, setView] = useState<View | null>(null);
  const [filter, setFilter] = useState("");
  const [editing, setEditing] = useState<ReshadePreset | null>(null);

  const load = useCallback(() => {
    ipc
      .reshadeView()
      .then(setView)
      .catch((error) => toast("error", String(error)));
  }, [toast]);
  // Mods coming and going change the presets there are.
  useEffect(load, [load, snapshot]);

  const profile = snapshot?.profiles.find((p) => p.active) ?? null;

  const presets = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const all = view?.presets ?? [];
    if (!needle) return all;
    return all.filter(
      (p) =>
        p.name.toLowerCase().includes(needle) ||
        p.providedBy.toLowerCase().includes(needle) ||
        p.techniques.some((t) => t.toLowerCase().includes(needle)),
    );
  }, [view, filter]);

  async function run(action: () => Promise<unknown>, done?: string) {
    try {
      await action();
      if (done) toast("success", done);
    } catch (error) {
      toast("error", String(error));
    }
    load();
  }

  const use = (preset: ReshadePreset) =>
    run(() => ipc.setReshadePreset(preset.path), `ReShade starts with ${preset.name} now.`);

  const pin = (preset: ReshadePreset | null) =>
    profile &&
    run(
      () => ipc.setProfileReshadePreset(profile.id, preset?.path ?? null),
      preset
        ? `${profile.name} starts ReShade with ${preset.name}.`
        : `${profile.name} leaves ReShade's choice alone.`,
    );

  async function duplicate(preset: ReshadePreset) {
    const name = await askText({
      title: "Duplicate preset",
      label: "Name of the copy",
      initial: `${preset.name} (mine)`,
      confirmLabel: "Duplicate",
    });
    if (name) await run(() => ipc.duplicateReshadePreset(preset.path, name), `Made ${name}.`);
  }

  async function rename(preset: ReshadePreset) {
    const name = await askText({
      title: "Rename preset",
      initial: preset.name,
      confirmLabel: "Rename",
    });
    if (name && name !== preset.name) await run(() => ipc.renameReshadePreset(preset.path, name));
  }

  async function remove(preset: ReshadePreset) {
    const sure = await askConfirm({
      title: "Delete preset",
      message: `Delete ${preset.name} from the game folder? This cannot be undone.`,
      confirmLabel: "Delete",
      danger: true,
    });
    if (sure) await run(() => ipc.deleteReshadePreset(preset.path), `Deleted ${preset.name}.`);
  }

  function menu(preset: ReshadePreset): MenuItem[] {
    const own = preset.origin === "game";
    const pinned = view?.profilePreset === preset.path;
    return [
      {
        label: "Start ReShade with this",
        icon: <Play size={13} />,
        disabled: preset.active,
        onSelect: () => void use(preset),
      },
      {
        label: pinned ? `Stop using for ${profile?.name ?? "this profile"}` : `Use for ${profile?.name ?? "this profile"}`,
        icon: pinned ? <PinOff size={13} /> : <Pin size={13} />,
        disabled: !profile,
        onSelect: () => void pin(pinned ? null : preset),
      },
      "separator",
      {
        label: "Edit",
        icon: <FileCog size={13} />,
        disabled: !preset.source,
        onSelect: () => setEditing(preset),
      },
      { label: "Duplicate", icon: <Copy size={13} />, onSelect: () => void duplicate(preset) },
      {
        label: "Rename",
        icon: <Pencil size={13} />,
        disabled: !own,
        onSelect: () => void rename(preset),
      },
      "separator",
      {
        label: "Delete",
        icon: <Trash2 size={13} />,
        danger: true,
        disabled: !own || preset.active,
        onSelect: () => void remove(preset),
      },
    ];
  }

  if (!snapshot?.game) {
    return (
      <Empty
        icon={<Palette size={20} />}
        title="No game folder yet"
        hint="Choose where Stellar Blade is installed in Settings, and its ReShade presets show up here."
      />
    );
  }

  const pinnedMissing =
    view?.profilePreset != null && !view.presets.some((p) => p.path === view.profilePreset);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-line px-3">
        <h1 className="text-[13px] font-semibold">ReShade</h1>
        {view && (
          <span className="text-[11px] text-ink-muted">
            {view.presets.length} preset{view.presets.length === 1 ? "" : "s"} · {view.effects}{" "}
            effect file{view.effects === 1 ? "" : "s"} found
          </span>
        )}
        <div className="ml-auto flex items-center gap-1.5">
          <input
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Filter by name, mod or effect"
            className="h-8 w-56 rounded-md border border-line bg-base px-2.5 text-[12px] placeholder:text-ink-muted focus:border-accent-soft focus:outline-none"
          />
          <Button size="sm" variant="ghost" onClick={load}>
            <RefreshCw size={13} /> Refresh
          </Button>
        </div>
      </header>

      {view && (
        <div className="shrink-0 space-y-2 border-b border-line px-3 py-2.5 text-[12px]">
          {view.installed ? (
            <div className="flex flex-wrap items-center gap-2">
              <Palette size={14} className="text-ink-muted" />
              <Badge tone="accent">Installed</Badge>
              <span className="text-ink-soft">
                {view.installed.version ? `ReShade ${view.installed.version}` : "ReShade"} as{" "}
                <span className="font-mono">{view.installed.dll}</span>, from{" "}
                {view.installed.providedBy}
              </span>
            </div>
          ) : (
            <div className="flex flex-wrap items-center gap-2 text-danger">
              <AlertTriangle size={14} className="shrink-0" />
              <span className="min-w-0 flex-1">
                ReShade is not beside the game executable, so presets do nothing. Run its installer
                on SB-Win64-Shipping.exe (DirectX 10/11/12), or install a ReShade archive as a mod.
              </span>
              <Button size="sm" variant="outline" onClick={() => void openUrl(view.downloadUrl)}>
                <ExternalLink size={12} /> reshade.me
              </Button>
            </div>
          )}

          {view.active && !view.activeFound && (
            <p className="flex items-start gap-1.5 text-warn">
              <AlertTriangle size={13} className="mt-px shrink-0" />
              ReShade starts with {view.active.split("/").pop()}, which is not there — it will
              start an empty preset of that name. Pick one below.
            </p>
          )}

          {profile && (
            <label className="flex flex-wrap items-center gap-2 text-ink-soft">
              <Pin size={13} className="text-ink-muted" />
              <span>
                Profile <span className="font-medium text-ink">{profile.name}</span> starts ReShade
                with
              </span>
              <select
                value={view.profilePreset ?? ""}
                onChange={(event) => {
                  const chosen = view.presets.find((p) => p.path === event.target.value) ?? null;
                  void pin(chosen);
                }}
                className="h-7 max-w-72 rounded-md border border-line bg-base px-2 text-[12px] focus:border-accent-soft focus:outline-none"
              >
                <option value="">whatever ReShade last used</option>
                {pinnedMissing && (
                  <option value={view.profilePreset ?? ""}>
                    {view.profilePreset?.split("/").pop()} (missing)
                  </option>
                )}
                {view.presets.map((p) => (
                  <option key={p.path} value={p.path}>
                    {p.folder ? `${p.folder}/${p.name}` : p.name}
                  </option>
                ))}
              </select>
              <span className="text-[11px] text-ink-muted">
                — set on switching to it and before every launch.
              </span>
            </label>
          )}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {view && view.presets.length === 0 ? (
          <Empty
            icon={<Palette size={20} />}
            title="No presets yet"
            hint="Preset archives install like any mod — they are recognised and put in reshade-presets on their own. Presets saved from ReShade's overlay in game show up here too."
          />
        ) : (
          <ul className="divide-y divide-line">
            {presets.map((preset) => (
              <PresetRow
                key={preset.path}
                preset={preset}
                pinned={view?.profilePreset === preset.path}
                onUse={() => void use(preset)}
                onEdit={() => setEditing(preset)}
                onMenu={(event) => openContextMenu(event, menu(preset))}
              />
            ))}
          </ul>
        )}
      </div>

      {editing?.source && (
        <ConfigEditor
          scope={editing.source.scope}
          path={editing.source.path}
          title={`${editing.name} — ReShade preset`}
          onClose={(saved) => {
            setEditing(null);
            if (saved) load();
          }}
        />
      )}
    </div>
  );
}

function PresetRow({
  preset,
  pinned,
  onUse,
  onEdit,
  onMenu,
}: {
  preset: ReshadePreset;
  pinned: boolean;
  onUse: () => void;
  onEdit: () => void;
  onMenu: (event: React.MouseEvent) => void;
}) {
  const missing = preset.missingEffects.length;
  return (
    <li
      onContextMenu={onMenu}
      onDoubleClick={() => !preset.active && onUse()}
      className={clsx(
        "group flex items-center gap-3 px-3 py-2 hover:bg-hover/50",
        preset.active && "bg-accent-soft/5",
      )}
    >
      <button
        role="radio"
        aria-checked={preset.active}
        aria-label={`Start ReShade with ${preset.name}`}
        title={preset.active ? "ReShade starts with this preset" : "Start ReShade with this preset"}
        onClick={onUse}
        className={clsx(
          "grid size-4 shrink-0 place-items-center rounded-full border transition-colors",
          preset.active ? "border-accent" : "border-line-strong hover:border-ink-muted",
        )}
      >
        {preset.active && <span className="size-2 rounded-full bg-accent" />}
      </button>

      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13px] font-medium">{preset.name}</span>
          {preset.folder && (
            <span className="truncate font-mono text-[11px] text-ink-muted">{preset.folder}/</span>
          )}
          {preset.active && <Badge tone="accent">Active</Badge>}
          {pinned && (
            <Badge title="The active profile starts ReShade with this preset">Profile</Badge>
          )}
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-2 text-[11px] text-ink-muted">
          <span className="shrink-0">
            {preset.origin === "mod"
              ? preset.providedBy
              : preset.origin === "game"
                ? "Yours, in the game folder"
                : "Overwrite"}
          </span>
          <span>·</span>
          <span
            className="min-w-0 truncate"
            title={preset.techniques.join("\n") || "No effects switched on"}
          >
            {preset.techniques.length === 0
              ? "no effects switched on"
              : `${preset.techniques.length} effect${preset.techniques.length === 1 ? "" : "s"}: ${preset.techniques.join(", ")}`}
          </span>
        </div>
      </div>

      {missing > 0 && (
        <Badge
          tone="danger"
          title={`ReShade will not find ${preset.missingEffects.join(", ")}. Install the shader pack the preset's page names — until then those effects are left out.`}
        >
          {missing} effect file{missing === 1 ? "" : "s"} missing
        </Badge>
      )}
      {missing === 0 && preset.unchecked > 0 && (
        <Badge title="An older preset that does not name the files its effects are in, so they cannot be checked.">
          Not checked
        </Badge>
      )}
      <span className="w-14 shrink-0 text-right text-[11px] text-ink-muted">
        {formatBytes(preset.bytes)}
      </span>

      <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
        {!preset.active && (
          <Button size="sm" variant="outline" onClick={onUse}>
            <Play size={12} /> Use
          </Button>
        )}
        <Button size="sm" variant="ghost" disabled={!preset.source} onClick={onEdit}>
          <FileCog size={12} /> Edit
        </Button>
        <Button size="sm" variant="ghost" aria-label="More" onClick={onMenu}>
          <MoreHorizontal size={14} />
        </Button>
      </div>
    </li>
  );
}
