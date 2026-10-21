import { Check, Copy, Monitor, Play, SquareMenu, UserCog } from "lucide-react";
import { useEffect, useState } from "react";

import { ipc, type ProfileView, type ShortcutPlace } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Dialog } from "./ui";

/**
 * Make a shortcut that starts the game with a profile, as Mod Organizer 2's
 * shortcuts do, or one that opens the manager on it — and show the command
 * behind it, for launchers that take a command line instead.
 */
export function ShortcutDialog({
  profiles,
  initial,
  onClose,
}: {
  profiles: ProfileView[];
  initial: number;
  onClose: () => void;
}) {
  const toast = useApp((s) => s.toast);
  const [profileId, setProfileId] = useState(initial);
  const [launch, setLaunch] = useState(true);
  const [command, setCommand] = useState("");
  const [copied, setCopied] = useState(false);
  const [working, setWorking] = useState(false);

  useEffect(() => {
    let live = true;
    setCopied(false);
    ipc
      .profileCommand(profileId, launch)
      .then((line) => live && setCommand(line))
      .catch((error) => live && setCommand(String(error)));
    return () => {
      live = false;
    };
  }, [profileId, launch]);

  async function create(place: ShortcutPlace) {
    setWorking(true);
    try {
      const path = await ipc.createProfileShortcut(profileId, place, launch);
      toast("success", `Shortcut created: ${path}`);
      onClose();
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
    }
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
    } catch (error) {
      toast("error", `Could not copy: ${error}`);
    }
  }

  const choices = [
    {
      value: true,
      icon: Play,
      label: "Start the game",
      hint: "Makes the profile active and starts Stellar Blade without showing the manager, which closes once the game is on its way. If something goes wrong, the manager opens and says what.",
    },
    {
      value: false,
      icon: UserCog,
      label: "Open the manager",
      hint: "Opens the manager with the profile active, or switches a manager that is already open.",
    },
  ];

  return (
    <Dialog
      open
      title="Profile shortcut"
      width="520px"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
          <div className="flex-1" />
          <Button variant="outline" disabled={working} onClick={() => void create("startMenu")}>
            <SquareMenu size={13} /> Add to Start menu
          </Button>
          <Button variant="primary" disabled={working} onClick={() => void create("desktop")}>
            <Monitor size={13} /> Put on desktop
          </Button>
        </>
      }
    >
      <label className="mb-1.5 block text-[12px] font-medium">Profile</label>
      <select
        value={profileId}
        onChange={(event) => setProfileId(Number(event.target.value))}
        className="mb-4 h-8 w-full rounded-md border border-line bg-base px-2 text-[12px] focus:border-accent-soft focus:outline-none"
      >
        {profiles.map((profile) => (
          <option key={profile.id} value={profile.id}>
            {profile.name}
          </option>
        ))}
      </select>

      <label className="mb-1.5 block text-[12px] font-medium">The shortcut will</label>
      <div className="mb-4 space-y-1.5">
        {choices.map((choice) => {
          const Icon = choice.icon;
          const selected = launch === choice.value;
          return (
            <button
              key={choice.label}
              role="radio"
              aria-checked={selected}
              onClick={() => setLaunch(choice.value)}
              className={
                "flex w-full items-start gap-2.5 rounded-md border px-2.5 py-2 text-left transition-colors " +
                (selected ? "border-accent bg-accent/10" : "border-line hover:border-line-strong")
              }
            >
              <Icon
                size={14}
                className={"mt-0.5 shrink-0 " + (selected ? "text-accent-soft" : "text-ink-muted")}
              />
              <span className="min-w-0">
                <span className="block text-[13px] font-medium">{choice.label}</span>
                <span className="block text-[11px] leading-relaxed text-ink-muted">{choice.hint}</span>
              </span>
            </button>
          );
        })}
      </div>

      <label className="mb-1.5 block text-[12px] font-medium">
        Command line{" "}
        <span className="font-normal text-ink-muted">· for Steam, Playnite or another launcher</span>
      </label>
      <div className="flex items-start gap-1.5">
        <code
          aria-label="Command line"
          className="min-w-0 flex-1 rounded-md border border-line bg-base px-2.5 py-1.5 font-mono text-[11px] leading-relaxed break-all text-ink-soft select-all"
        >
          {command}
        </code>
        <Button variant="outline" onClick={() => void copy()} title="Copy">
          {copied ? <Check size={13} /> : <Copy size={13} />}
        </Button>
      </div>
      <p className="mt-2 text-[11px] leading-relaxed text-ink-muted">
        The shortcut names the profile, so renaming the profile breaks it — make a new one after.
      </p>
    </Dialog>
  );
}
