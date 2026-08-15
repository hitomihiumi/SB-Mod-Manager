/**
 * Typed wrappers over the Tauri command surface.
 *
 * These interfaces mirror the serde-serialised DTOs in `crates/sbmm-app/src/dto.rs`;
 * keep the two in step when either changes.
 */
import { invoke } from "@tauri-apps/api/core";

export type ModTypeId =
  | "ue4ssFramework"
  | "logicMod"
  | "ue4ssLua"
  | "ue4ssDll"
  | "movie"
  | "rootBinary"
  | "genericPak"
  | "gameRootOverlay"
  | "unknown";

export type Confidence = "high" | "medium" | "askUser";

export type ChangeKind = "enable" | "disable" | "reorder";

export interface GameInstall {
  root: string;
  source: "steam" | "epic" | "manual";
  executable: string | null;
}

export interface ModView {
  id: number;
  name: string;
  version: string | null;
  modType: ModTypeId;
  componentTypes: ModTypeId[];
  groupId: number | null;
  enabled: boolean;
  deployed: boolean;
  priority: number;
  sizeBytes: number;
  source: string;
  installedAt: string;
  warnings: string[];
}

export interface GroupView {
  id: number;
  name: string;
  color: string | null;
  collapsed: boolean;
  sortIndex: number;
}

export interface PendingChange {
  modId: number;
  name: string;
  kind: ChangeKind;
}

export interface AppSnapshot {
  game: GameInstall | null;
  mods: ModView[];
  groups: GroupView[];
  pending: PendingChange[];
  drift: string[];
  autoApply: boolean;
}

export interface ComponentFile {
  source: string;
  target: string;
}

export interface DetectedComponent {
  modType: ModTypeId;
  confidence: Confidence;
  files: ComponentFile[];
  targetSubdir: string;
  ue4ssModName: string | null;
  pakSets: { base: string; hadPSuffix: boolean }[];
  warnings: string[];
  notes: string[];
}

export interface StagedInstall {
  stagingId: string;
  suggestedName: string;
  components: DetectedComponent[];
  sizeBytes: number;
  needsConfirmation: boolean;
}

export interface ApplyReport {
  deployed: string[];
  removed: string[];
  failed: { name: string; reason: string }[];
}

export interface Folders {
  mods: string;
  backups: string;
  game: string | null;
}

export const ipc = {
  snapshot: () => invoke<AppSnapshot>("snapshot"),
  discoverGames: () => invoke<GameInstall[]>("discover_games"),
  setGameRoot: (path: string) => invoke<GameInstall>("set_game_root", { path }),

  stageArchive: (path: string) => invoke<StagedInstall>("stage_archive", { path }),
  stageFolder: (path: string) => invoke<StagedInstall>("stage_folder", { path }),
  confirmInstall: (stagingId: string, name: string, typeOverride?: ModTypeId) =>
    invoke<number>("confirm_install", { stagingId, name, typeOverride: typeOverride ?? null }),
  cancelInstall: (stagingId: string) => invoke<void>("cancel_install", { stagingId }),

  setEnabled: (modIds: number[], enabled: boolean) =>
    invoke<void>("set_enabled", { modIds, enabled }),
  setOrder: (modIds: number[]) => invoke<void>("set_order", { modIds }),
  apply: () => invoke<ApplyReport>("apply"),
  uninstall: (modId: number) => invoke<void>("uninstall", { modId }),
  renameMod: (modId: number, name: string) => invoke<void>("rename_mod", { modId, name }),

  createGroup: (name: string, color?: string) =>
    invoke<number>("create_group", { name, color: color ?? null }),
  deleteGroup: (groupId: number) => invoke<void>("delete_group", { groupId }),
  setGroupCollapsed: (groupId: number, collapsed: boolean) =>
    invoke<void>("set_group_collapsed", { groupId, collapsed }),
  assignGroup: (modIds: number[], groupId: number | null) =>
    invoke<void>("assign_group", { modIds, groupId }),

  setAutoApply: (enabled: boolean) => invoke<void>("set_auto_apply", { enabled }),
  folders: () => invoke<Folders>("folders"),
};

/** Display names and accent colours for each payload type. */
export const MOD_TYPE_META: Record<ModTypeId, { label: string; hint: string }> = {
  genericPak: { label: "Pak", hint: "SB/Content/Paks/~mods" },
  logicMod: { label: "Logic", hint: "SB/Content/Paks/LogicMods" },
  ue4ssLua: { label: "Lua", hint: "UE4SS Lua mod" },
  ue4ssDll: { label: "C++", hint: "UE4SS native mod" },
  ue4ssFramework: { label: "UE4SS", hint: "UE4SS runtime" },
  rootBinary: { label: "Root", hint: "SB/Binaries/Win64" },
  movie: { label: "Movie", hint: "SB/Content/Movies" },
  gameRootOverlay: { label: "Overlay", hint: "Laid onto the game folder" },
  unknown: { label: "Unknown", hint: "Needs a type before it can install" },
};

export const SELECTABLE_TYPES: ModTypeId[] = [
  "genericPak",
  "logicMod",
  "ue4ssLua",
  "ue4ssDll",
  "ue4ssFramework",
  "rootBinary",
  "movie",
  "gameRootOverlay",
];
