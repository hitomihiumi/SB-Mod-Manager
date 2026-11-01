import { open as pickFile } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { AppWindow, Copy, ExternalLink, FolderOpen } from "lucide-react";

import { ipc } from "../../lib/ipc";
import { useApp } from "../../store/useApp";
import type { MenuItem } from "../ContextMenu";

/** Programs picked before, newest first, so the next file is one click away. */
const RECENT_KEY = "sbmm.openWith";
const KEEP = 6;

const programName = (path: string) =>
  path.split(/[\\/]/).pop()?.replace(/\.(exe|bat|cmd|com)$/i, "") ?? path;

function recentPrograms(): string[] {
  try {
    const stored = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((p): p is string => typeof p === "string") : [];
  } catch {
    return [];
  }
}

function remember(program: string) {
  try {
    const next = [program, ...recentPrograms().filter((p) => p !== program)].slice(0, KEEP);
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    // Storage can be unavailable; the menu just does not remember.
  }
}

const fail = (error: unknown) => useApp.getState().toast("error", String(error));

/** Open a file or folder with the system's program for it, or with `program`. */
export async function openFile(path: string, program?: string) {
  try {
    await ipc.openPath(path, program);
    if (program) remember(program);
  } catch (error) {
    fail(error);
  }
}

/** Ask for a program, then open the file with it. */
export async function openWithChosen(path: string) {
  try {
    const picked = await pickFile({
      title: "Open with…",
      multiple: false,
      filters: [{ name: "Programs", extensions: ["exe", "bat", "cmd", "com"] }],
    });
    if (typeof picked === "string") await openFile(path, picked);
  } catch (error) {
    fail(error);
  }
}

export function reveal(path: string) {
  revealItemInDir(path).catch(fail);
}

export async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    useApp.getState().toast("success", "Path copied.");
  } catch (error) {
    fail(error);
  }
}

/** What can be done with a real file, wherever it is listed. */
export function fileMenu(path: string): MenuItem[] {
  const recent = recentPrograms();
  return [
    { label: "Open", icon: <ExternalLink size={13} />, onSelect: () => void openFile(path) },
    {
      label: "Open with",
      icon: <AppWindow size={13} />,
      items: [
        ...recent.map((program) => ({
          label: programName(program),
          onSelect: () => void openFile(path, program),
        })),
        ...(recent.length > 0 ? (["separator"] as const) : []),
        { label: "Choose a program…", onSelect: () => void openWithChosen(path) },
      ],
    },
    { label: "Show in folder", icon: <FolderOpen size={13} />, onSelect: () => reveal(path) },
    { label: "Copy path", icon: <Copy size={13} />, onSelect: () => void copyText(path) },
  ];
}
