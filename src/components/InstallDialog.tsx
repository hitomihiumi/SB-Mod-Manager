import { useEffect, useState } from "react";

import { formatBytes } from "../lib/format";
import {
  MOD_TYPE_META,
  SELECTABLE_TYPES,
  type ModTypeId,
  type StagedInstall,
  type VariantChoice,
} from "../lib/ipc";
import { useApp } from "../store/useApp";
import { FomodWizard } from "./FomodWizard";
import { Gallery, Preview } from "./Preview";
import { Badge, Button, Dialog } from "./ui";

/**
 * Shown only when an install cannot go ahead on its own: detection could not
 * decide where the files belong, the archive holds several versions of the
 * mod and one has to be chosen, or it brings its own FOMOD installer.
 * Everything else installs without interrupting the user.
 */
export function InstallDialog() {
  const queue = useApp((s) => s.queue);
  const confirmInstall = useApp((s) => s.confirmInstall);
  const cancelInstall = useApp((s) => s.cancelInstall);
  const skipRemaining = useApp((s) => s.skipRemaining);

  const staged = queue[0] ?? null;

  const [name, setName] = useState("");
  const [type, setType] = useState<ModTypeId>("genericPak");
  const [chosen, setChosen] = useState<string[]>([]);

  useEffect(() => {
    if (staged) {
      setName(staged.suggestedName);
      const detected = staged.components[0]?.modType;
      setType(detected && detected !== "unknown" ? detected : "genericPak");
      // What was picked last time, or else the first option.
      const first = staged.variants?.options[0]?.label;
      setChosen(
        staged.rememberedVariants.length > 0 ? staged.rememberedVariants : first ? [first] : [],
      );
    }
  }, [staged]);

  if (!staged) return null;
  if (staged.fomod) {
    return <FomodWizard key={staged.stagingId} staged={staged} queueLength={queue.length} />;
  }

  const variants = staged.variants;
  const many = queue.length > 1 ? ` (1 of ${queue.length})` : "";

  return (
    <Dialog
      open
      title={variants ? `Which version?${many}` : `Where do these files belong?${many}`}
      onClose={() => void cancelInstall()}
      footer={
        <>
          {queue.length > 1 && (
            <Button variant="ghost" onClick={() => void skipRemaining()}>
              Skip all {queue.length}
            </Button>
          )}
          <div className="flex-1" />
          <Button variant="ghost" onClick={() => void cancelInstall()}>
            Skip
          </Button>
          <Button
            variant="primary"
            disabled={!name.trim() || (variants !== null && chosen.length === 0)}
            onClick={() =>
              void (variants
                ? confirmInstall(name.trim(), undefined, chosen)
                : confirmInstall(name.trim(), type))
            }
          >
            Install
          </Button>
        </>
      }
    >
      {variants ? (
        <VariantBody
          staged={staged}
          variants={variants}
          name={name}
          setName={setName}
          chosen={chosen}
          setChosen={setChosen}
        />
      ) : (
        <TypeBody staged={staged} name={name} setName={setName} type={type} setType={setType} />
      )}
    </Dialog>
  );
}

/**
 * Pick which of an archive's versions to install. Options that all clash are
 * a radio choice; otherwise several can be ticked, with a warning when the
 * ticked ones put files in the same place.
 */
function VariantBody({
  staged,
  variants,
  name,
  setName,
  chosen,
  setChosen,
}: {
  staged: StagedInstall;
  variants: VariantChoice;
  name: string;
  setName: (name: string) => void;
  chosen: string[];
  setChosen: (chosen: string[]) => void;
}) {
  const remembered = staged.rememberedVariants.length > 0;
  const pictured = variants.options.some((option) => option.preview);
  const toggle = (label: string) =>
    setChosen(
      variants.exclusive
        ? [label]
        : chosen.includes(label)
          ? chosen.filter((c) => c !== label)
          : [...chosen, label],
    );

  return (
    <>
      <p className="mb-4 text-[12px] leading-relaxed text-ink-muted">
        This archive holds several versions of the mod that put files in the same places, so only
        one of them would take effect.{" "}
        {variants.exclusive ? "Pick the one to install." : "Pick which to install."} The choice is
        remembered, and the next version of this mod installs the same way.
      </p>

      {!pictured && <Gallery folder={staged.stagingId} paths={staged.previews} />}

      <label className="mb-1.5 block text-[12px] font-medium">Name</label>
      <input
        value={name}
        onChange={(event) => setName(event.target.value)}
        className="mb-4 h-8 w-full rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
      />

      <label className="mb-1.5 flex items-center gap-2 text-[12px] font-medium">
        Version
        {remembered && <Badge tone="accent">Your choice last time</Badge>}
      </label>
      <div className="space-y-1.5">
        {variants.options.map((option) => {
          const selected = chosen.includes(option.label);
          return (
            <button
              key={option.folder}
              onClick={() => toggle(option.label)}
              role={variants.exclusive ? "radio" : "checkbox"}
              aria-checked={selected}
              className={
                "flex w-full items-center gap-2.5 rounded-md border px-2.5 py-2 text-left transition-colors " +
                (selected ? "border-accent bg-accent/10" : "border-line hover:border-line-strong")
              }
            >
              <span
                className={
                  "grid size-4 shrink-0 place-items-center border " +
                  (variants.exclusive ? "rounded-full " : "rounded ") +
                  (selected ? "border-accent bg-accent" : "border-line-strong")
                }
              >
                {selected && <span className="size-1.5 rounded-full bg-accent-ink" />}
              </span>
              {pictured && (
                <Preview
                  folder={staged.stagingId}
                  path={option.preview}
                  alt={option.label}
                  className="h-12 w-20 shrink-0"
                />
              )}
              <span className="min-w-0 flex-1 truncate text-[13px]">{option.label}</span>
              {option.modTypes.map((t) => (
                <Badge key={t} tone="accent">
                  {MOD_TYPE_META[t].label}
                </Badge>
              ))}
              <span className="text-[11px] text-ink-muted">
                {option.fileCount} file{option.fileCount === 1 ? "" : "s"}
              </span>
            </button>
          );
        })}
      </div>
      {!variants.exclusive && chosen.length > 1 && (
        <p className="mt-2 text-[11px] leading-relaxed text-warn">
          Some of the ticked versions replace the same files; the one further down the load order
          wins.
        </p>
      )}
      <p className="mt-3 text-[11px] text-ink-muted">{formatBytes(staged.sizeBytes)} in all</p>
    </>
  );
}

/** Choose a type for an archive the detector could not place. */
function TypeBody({
  staged,
  name,
  setName,
  type,
  setType,
}: {
  staged: StagedInstall;
  name: string;
  setName: (name: string) => void;
  type: ModTypeId;
  setType: (type: ModTypeId) => void;
}) {
  const fileCount = staged.components.reduce((sum, c) => sum + c.files.length, 0);
  const notes = staged.components.flatMap((c) => c.notes);
  const targetDir = MOD_TYPE_META[type].hint;

  return (
    <>
      <p className="mb-4 text-[12px] leading-relaxed text-ink-muted">
        This archive has no marker the detector recognises — no pak files, no{" "}
        <code className="font-mono text-ink-soft">LogicMods</code> folder, no UE4SS script. Pick
        the type and it will be remembered for archives shaped like this one.
      </p>

      <Gallery folder={staged.stagingId} paths={staged.previews} />

      <label className="mb-1.5 block text-[12px] font-medium">Name</label>
      <input
        value={name}
        onChange={(event) => setName(event.target.value)}
        className="mb-4 h-8 w-full rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
      />

      <label className="mb-1.5 block text-[12px] font-medium">Type</label>
      <div className="mb-2 grid grid-cols-2 gap-1.5">
        {SELECTABLE_TYPES.map((candidate) => (
          <button
            key={candidate}
            onClick={() => setType(candidate)}
            className={
              "rounded-md border px-2.5 py-2 text-left transition-colors " +
              (type === candidate
                ? "border-accent bg-accent/10"
                : "border-line hover:border-line-strong")
            }
          >
            <div className="text-[12px] font-medium">{MOD_TYPE_META[candidate].label}</div>
            <div className="truncate font-mono text-[10px] text-ink-muted">
              {MOD_TYPE_META[candidate].hint}
            </div>
          </button>
        ))}
      </div>

      <div className="mt-4 rounded-md border border-line bg-base p-3">
        <div className="mb-2 flex items-center gap-2">
          <Badge tone="accent">{MOD_TYPE_META[type].label}</Badge>
          <span className="text-[11px] text-ink-muted">
            {fileCount} file{fileCount === 1 ? "" : "s"} · {formatBytes(staged.sizeBytes)}
          </span>
        </div>
        <p className="mb-2 font-mono text-[11px] text-ink-soft">→ {targetDir}</p>
        {notes.length > 0 && (
          <ul className="mb-2 space-y-0.5">
            {notes.map((note) => (
              <li key={note} className="text-[11px] leading-relaxed text-ink-muted">
                {note}
              </li>
            ))}
          </ul>
        )}
        <ul className="max-h-32 space-y-0.5 overflow-y-auto">
          {staged.components
            .flatMap((c) => c.files)
            .slice(0, 40)
            .map((file) => (
              <li key={file.source} className="truncate font-mono text-[11px] text-ink-muted">
                {file.source}
              </li>
            ))}
        </ul>
      </div>
    </>
  );
}
