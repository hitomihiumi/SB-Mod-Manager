import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import { ExternalLink, FileCog, ImageOff, Plus, X } from "lucide-react";
import { useEffect, useState } from "react";

import { formatBytes, formatDate } from "../lib/format";
import { ipc, modPageUrl, MOD_TYPE_META, type ConfigFile, type ModView } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { ConfigEditor } from "./ConfigEditor";
import { Preview } from "./Preview";
import { Badge } from "./ui";

/**
 * Everything about one mod beside the list, like Mod Organizer 2's mod
 * information window: its picture, the user's notes and tags, and its
 * settings files to edit.
 */
export function ModDetails({ mod, onClose }: { mod: ModView; onClose: () => void }) {
  const refresh = useApp((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const [images, setImages] = useState<string[]>([]);
  const [files, setFiles] = useState<ConfigFile[]>([]);
  const [notes, setNotes] = useState(mod.notes ?? "");
  const [tag, setTag] = useState("");
  const [editing, setEditing] = useState<string | null>(null);

  const loadFiles = () => {
    ipc
      .configFiles({ kind: "mod", modId: mod.id })
      .then(setFiles)
      .catch(() => setFiles([]));
  };

  useEffect(() => {
    setNotes(mod.notes ?? "");
    setTag("");
    ipc
      .modImages(mod.id)
      .then(setImages)
      .catch(() => setImages([]));
    loadFiles();
    // Reloaded for each mod shown; notes follow the mod, not every refresh.
  }, [mod.id]);

  async function save(action: () => Promise<void>) {
    try {
      await action();
      await refresh();
    } catch (error) {
      toast("error", String(error));
    }
  }

  function addTag() {
    const next = tag.trim();
    if (!next) return;
    setTag("");
    void save(() => ipc.setModTags(mod.id, [...mod.tags, next]));
  }

  return (
    <aside className="flex w-80 shrink-0 flex-col border-l border-line bg-surface">
      <div className="flex items-start gap-2 border-b border-line px-3 py-2.5">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-[13px] font-semibold" title={mod.name}>
            {mod.name}
          </h3>
          <p className="text-[11px] text-ink-muted">
            {mod.version ? `${mod.version} · ` : ""}
            {formatBytes(mod.sizeBytes)} · installed {formatDate(mod.installedAt)}
          </p>
        </div>
        <button onClick={onClose} className="text-ink-muted hover:text-ink" aria-label="Close details">
          <X size={14} />
        </button>
      </div>

      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-3">
        <section>
          <Preview
            folder={mod.folder}
            path={mod.image}
            alt={mod.name}
            className="h-44 w-full"
          />
          {images.length > 0 && (
            <div className="mt-1.5 flex gap-1.5 overflow-x-auto pb-1">
              {images.map((image) => (
                <button
                  key={image}
                  title={image}
                  onClick={() => void save(() => ipc.setModImage(mod.id, image))}
                  className={clsx(
                    "shrink-0 rounded-md border-2",
                    mod.image === image ? "border-accent" : "border-transparent",
                  )}
                >
                  <Preview folder={mod.folder} path={image} alt={image} className="size-12" />
                </button>
              ))}
              <button
                title="No picture"
                onClick={() => void save(() => ipc.setModImage(mod.id, null))}
                className={clsx(
                  "grid size-[52px] shrink-0 place-items-center rounded-md border-2 text-ink-muted",
                  mod.image === null ? "border-accent" : "border-line",
                )}
              >
                <ImageOff size={14} />
              </button>
            </div>
          )}
        </section>

        <section className="flex flex-wrap items-center gap-1.5">
          {mod.componentTypes.map((type) => (
            <Badge key={type} tone="accent" title={MOD_TYPE_META[type].hint}>
              {MOD_TYPE_META[type].label}
            </Badge>
          ))}
          <Badge>{mod.source}</Badge>
          {mod.nexusModId !== null && (
            <button
              onClick={() => void openUrl(modPageUrl(mod.nexusModId!))}
              className="inline-flex items-center gap-1 text-[11px] text-accent-soft hover:underline"
            >
              Nexus page <ExternalLink size={10} />
            </button>
          )}
        </section>

        <section>
          <h4 className="mb-1.5 text-[12px] font-medium">Tags</h4>
          <div className="flex flex-wrap gap-1.5">
            {mod.tags.map((each) => (
              <span
                key={each}
                className="inline-flex items-center gap-1 rounded-full border border-line bg-base px-2 py-0.5 text-[11px]"
              >
                {each}
                <button
                  aria-label={`Remove tag ${each}`}
                  onClick={() =>
                    void save(() => ipc.setModTags(mod.id, mod.tags.filter((t) => t !== each)))
                  }
                  className="text-ink-muted hover:text-danger"
                >
                  <X size={10} />
                </button>
              </span>
            ))}
            <span className="inline-flex items-center gap-1">
              <input
                value={tag}
                onChange={(event) => setTag(event.target.value)}
                onKeyDown={(event) => event.key === "Enter" && addTag()}
                placeholder="Add a tag"
                aria-label="New tag"
                className="h-6 w-24 rounded-full border border-line bg-base px-2 text-[11px] focus:border-accent-soft focus:outline-none"
              />
              {tag.trim() && (
                <button onClick={addTag} aria-label="Add tag" className="text-ink-muted hover:text-ink">
                  <Plus size={12} />
                </button>
              )}
            </span>
          </div>
        </section>

        <section>
          <h4 className="mb-1.5 text-[12px] font-medium">Notes</h4>
          <textarea
            value={notes}
            onChange={(event) => setNotes(event.target.value)}
            onBlur={() => {
              if (notes !== (mod.notes ?? "")) void save(() => ipc.setModNotes(mod.id, notes));
            }}
            placeholder="What it needs, what it clashes with, which options you picked…"
            rows={4}
            className="w-full resize-y rounded-md border border-line bg-base px-2.5 py-2 text-[12px] leading-relaxed focus:border-accent-soft focus:outline-none"
          />
        </section>

        <section>
          <h4 className="mb-1.5 text-[12px] font-medium">Settings files</h4>
          {files.length === 0 ? (
            <p className="text-[11px] text-ink-muted">This mod has none.</p>
          ) : (
            <ul className="space-y-1">
              {files.map((file) => (
                <li key={file.path}>
                  <button
                    onClick={() => setEditing(file.path)}
                    title={file.path}
                    className="flex w-full items-center gap-2 rounded-md border border-line px-2 py-1.5 text-left hover:border-line-strong"
                  >
                    <FileCog size={13} className="shrink-0 text-ink-muted" />
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px]">
                      {file.path.split("/").pop()}
                    </span>
                    {file.edited && <Badge tone="warn">Edited</Badge>}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>

      {editing && (
        <ConfigEditor
          scope={{ kind: "mod", modId: mod.id }}
          path={editing}
          title={`${editing.split("/").pop()} — ${mod.name}`}
          onClose={(saved) => {
            setEditing(null);
            if (saved) {
              loadFiles();
              void refresh();
            }
          }}
        />
      )}
    </aside>
  );
}
