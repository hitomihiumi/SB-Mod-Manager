import clsx from "clsx";
import { AlertTriangle, Check, ChevronLeft, ChevronRight, Lock, Wand2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import {
  ipc,
  type FomodGroup,
  type FomodPluginType,
  type FomodWizardState,
  type StagedInstall,
} from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Badge, Button, Dialog } from "./ui";
import { Preview } from "./Preview";

type Pick = [number, number, number];
const key = ([s, g, p]: Pick) => `${s}:${g}:${p}`;
const unkey = (k: string) => k.split(":").map(Number) as Pick;

const KIND_LABEL: Partial<Record<FomodPluginType, string>> = {
  required: "Required",
  recommended: "Recommended",
  couldBeUsable: "May not work",
  notUsable: "Not available",
};

/**
 * A FOMOD installer as a wizard, the way Mod Organizer 2 runs one: a page per
 * step, option groups ticked by their rules, and beside them the picture and
 * description of the option under the pointer. Which steps appear, what each
 * option is and what still needs answering all come from the manager, so the
 * script's conditions are evaluated in one place.
 */
export function FomodWizard({ staged, queueLength }: { staged: StagedInstall; queueLength: number }) {
  const confirmFomod = useApp((s) => s.confirmFomod);
  const cancelInstall = useApp((s) => s.cancelInstall);
  const skipRemaining = useApp((s) => s.skipRemaining);
  const toast = useApp((s) => s.toast);

  const fomod = staged.fomod!;
  const installer = fomod.installer;
  const [name, setName] = useState(staged.suggestedName);
  const [picked, setPicked] = useState<Set<string>>(() => new Set(fomod.selection.map(key)));
  const [state, setState] = useState<FomodWizardState | null>(null);
  const [page, setPage] = useState(0);
  const [focus, setFocus] = useState<Pick | null>(null);

  // The manager evaluates the script for every change of answers.
  useEffect(() => {
    let live = true;
    ipc
      .fomodState(staged.stagingId, [...picked].map(unkey))
      .then((next) => {
        if (!live) return;
        setState(next);
        // An option the script now makes required is ticked for the user.
        const required = next.kinds.flatMap((groups, s) =>
          next.visible[s]
            ? groups.flatMap((kinds, g) =>
                kinds.flatMap((kind, p) => (kind === "required" ? [key([s, g, p])] : [])),
              )
            : [],
        );
        if (required.some((k) => !picked.has(k))) {
          setPicked((current) => new Set([...current, ...required]));
        }
      })
      .catch((error) => toast("error", String(error)));
    return () => {
      live = false;
    };
  }, [picked, staged.stagingId, toast]);

  const visibleSteps = useMemo(
    () => installer.steps.map((_, s) => s).filter((s) => state?.visible[s] ?? s === 0),
    [installer.steps, state],
  );
  const stepIndex = visibleSteps[Math.min(page, visibleSteps.length - 1)] ?? 0;
  const step = installer.steps[stepIndex];
  const last = page >= visibleSteps.length - 1;
  const stepProblems = (state?.problems ?? []).filter((problem) =>
    step?.groups.some((group) => problem.startsWith(`${group.name}:`)),
  );

  function toggle(g: number, p: number) {
    const group = step!.groups[g]!;
    const kind = state?.kinds[stepIndex]?.[g]?.[p];
    if (kind === "required" || kind === "notUsable" || group.kind === "selectAll") return;
    const k = key([stepIndex, g, p]);
    setPicked((current) => {
      const next = new Set(current);
      const radio = group.kind === "selectExactlyOne" || group.kind === "selectAtMostOne";
      if (radio) {
        const was = next.has(k);
        group.plugins.forEach((_, other) => next.delete(key([stepIndex, g, other])));
        // At-most-one lets the only pick be taken back; exactly-one does not.
        if (!was || group.kind === "selectExactlyOne") next.add(k);
      } else if (next.has(k)) {
        next.delete(k);
      } else {
        next.add(k);
      }
      return next;
    });
  }

  const shown = focus && focus[0] === stepIndex ? focus : null;
  const focusPlugin = shown ? step?.groups[shown[1]]?.plugins[shown[2]] : null;
  const pictureRoot = fomod.root ? `${fomod.root}/` : "";

  return (
    <Dialog
      open
      width="880px"
      title={`${installer.name || staged.suggestedName}${queueLength > 1 ? ` (1 of ${queueLength})` : ""}`}
      onClose={() => void cancelInstall()}
      footer={
        <>
          {queueLength > 1 && (
            <Button variant="ghost" onClick={() => void skipRemaining()}>
              Skip all {queueLength}
            </Button>
          )}
          <span className="flex-1 text-[11px] text-ink-muted">
            Step {Math.min(page, visibleSteps.length - 1) + 1} of {visibleSteps.length}
          </span>
          <Button variant="ghost" onClick={() => void cancelInstall()}>
            Skip
          </Button>
          <Button variant="outline" disabled={page === 0} onClick={() => setPage(page - 1)}>
            <ChevronLeft size={13} /> Back
          </Button>
          {last ? (
            <Button
              variant="primary"
              disabled={!state || state.problems.length > 0 || !name.trim()}
              onClick={() => void confirmFomod(name.trim(), [...picked].map(unkey))}
            >
              <Wand2 size={13} /> Install
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={stepProblems.length > 0}
              onClick={() => setPage(page + 1)}
            >
              Next <ChevronRight size={13} />
            </Button>
          )}
        </>
      }
    >
      <div className="mb-3 flex items-center gap-3">
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          aria-label="Mod name"
          className="h-8 min-w-0 flex-1 rounded-md border border-line bg-base px-2.5 text-[13px] focus:border-accent-soft focus:outline-none"
        />
        {fomod.remembered && <Badge tone="accent">Your answers last time</Badge>}
      </div>

      <div className="grid grid-cols-[1fr_300px] gap-4">
        <div className="min-h-72 space-y-4">
          <h3 className="text-[14px] font-semibold">{step?.name}</h3>
          {step?.groups.map((group, g) => (
            <GroupView
              key={`${stepIndex}:${g}`}
              group={group}
              kinds={state?.kinds[stepIndex]?.[g] ?? []}
              isPicked={(p) => picked.has(key([stepIndex, g, p]))}
              onToggle={(p) => toggle(g, p)}
              onFocus={(p) => setFocus([stepIndex, g, p])}
            />
          ))}
          {stepProblems.map((problem) => (
            <p key={problem} className="flex items-center gap-1.5 text-[11px] text-warn">
              <AlertTriangle size={12} /> {problem}
            </p>
          ))}
        </div>

        {/* What the option under the pointer is, as Mod Organizer shows it. */}
        <aside className="space-y-2">
          <Preview
            folder={staged.stagingId}
            path={
              focusPlugin?.image
                ? pictureRoot + focusPlugin.image
                : installer.image
                  ? pictureRoot + installer.image
                  : null
            }
            alt={focusPlugin?.name ?? installer.name}
            className="h-52 w-full"
          />
          <div className="max-h-48 overflow-y-auto text-[12px] leading-relaxed whitespace-pre-line text-ink-soft">
            {focusPlugin ? (
              <>
                <div className="mb-1 font-medium text-ink">{focusPlugin.name}</div>
                {focusPlugin.description || "No description."}
              </>
            ) : (
              <span className="text-ink-muted">
                Point at an option to see its picture and description.
              </span>
            )}
          </div>
        </aside>
      </div>
    </Dialog>
  );
}

function GroupView({
  group,
  kinds,
  isPicked,
  onToggle,
  onFocus,
}: {
  group: FomodGroup;
  kinds: FomodPluginType[];
  isPicked: (p: number) => boolean;
  onToggle: (p: number) => void;
  onFocus: (p: number) => void;
}) {
  const radio = group.kind === "selectExactlyOne" || group.kind === "selectAtMostOne";
  const rule = {
    selectExactlyOne: "Choose one",
    selectAtMostOne: "Choose one or none",
    selectAtLeastOne: "Choose at least one",
    selectAll: "All of these are installed",
    selectAny: "Choose any",
  }[group.kind];

  return (
    <fieldset>
      <legend className="mb-1.5 flex items-center gap-2 text-[12px] font-medium">
        {group.name} <span className="font-normal text-ink-muted">· {rule}</span>
      </legend>
      <div className="space-y-1">
        {group.plugins.map((plugin, p) => {
          const kind = kinds[p] ?? "optional";
          const on = isPicked(p) || group.kind === "selectAll";
          const locked = kind === "required" || kind === "notUsable" || group.kind === "selectAll";
          return (
            <button
              key={`${plugin.name}:${p}`}
              onClick={() => onToggle(p)}
              onMouseEnter={() => onFocus(p)}
              onFocus={() => onFocus(p)}
              role={radio ? "radio" : "checkbox"}
              aria-checked={on}
              aria-disabled={locked}
              className={clsx(
                "flex w-full items-center gap-2.5 rounded-md border px-2.5 py-1.5 text-left transition-colors",
                on ? "border-accent bg-accent/10" : "border-line hover:border-line-strong",
                kind === "notUsable" && "opacity-50",
              )}
            >
              <span
                className={clsx(
                  "grid size-4 shrink-0 place-items-center border",
                  radio ? "rounded-full" : "rounded",
                  on ? "border-accent bg-accent" : "border-line-strong",
                )}
              >
                {on &&
                  (radio ? (
                    <span className="size-1.5 rounded-full bg-accent-ink" />
                  ) : (
                    <Check size={11} strokeWidth={3} className="text-accent-ink" />
                  ))}
              </span>
              <span className="min-w-0 flex-1 truncate text-[13px]">{plugin.name}</span>
              {locked && kind !== "notUsable" && <Lock size={11} className="text-ink-muted" />}
              {KIND_LABEL[kind] && (
                <Badge tone={kind === "notUsable" || kind === "couldBeUsable" ? "warn" : "accent"}>
                  {KIND_LABEL[kind]}
                </Badge>
              )}
            </button>
          );
        })}
      </div>
    </fieldset>
  );
}
