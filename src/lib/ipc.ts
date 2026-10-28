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
  | "splash"
  | "rootBinary"
  | "genericPak"
  | "gameRootOverlay"
  | "configTweak"
  | "reshadePreset"
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
  /** Set only when Nexus reports a version different from the installed one. */
  latestVersion: string | null;
  nexusModId: number | null;
  /** Loose files shared with other enabled mods; null when none. */
  fileConflicts: FileConflicts | null;
  /** What this enabled mod needs and the game will not have, e.g. "UE4SS". */
  missingRequirements: string[];
  /** Its folder in the library, which pictures are read from. */
  folder: string;
  /** The mod's picture, relative to `folder`. */
  image: string | null;
  notes: string | null;
  tags: string[];
  /** What the load order rules this mod breaks ask of it. */
  misordered: string[];
}

/** UE4SS at a glance. */
export interface Ue4ssStatus {
  /** Installed as a mod and enabled, installed but off, by hand, or not at all. */
  state: "managed" | "disabled" | "manual" | "missing";
  modId: number | null;
  version: string | null;
  /** Enabled mods that do nothing without UE4SS. */
  neededBy: string[];
  projectUrl: string;
}

/** The flags Mod Organizer 2 shows beside a mod. */
export interface FileConflicts {
  /** Paths it provides over a mod earlier in the load order. */
  wins: number;
  /** Paths a mod later in the load order takes from it. */
  loses: number;
  /** Every file it has is taken by later mods. */
  redundant: boolean;
}

/** What the game created under the virtual file system. */
export interface OverwriteView {
  path: string;
  files: number;
  bytes: number;
}

export interface UpdateCheckReport {
  checked: number;
  outdated: number;
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
  /** Null until an API key has been validated. */
  nexus: NexusAccount | null;
  updateChannel: UpdateChannel;
  /** Whether the manager publishes a Discord presence. */
  discordRpc: boolean;
  deploymentMode: DeploymentMode;
  /** Every profile, the active one marked. */
  profiles: ProfileView[];
  overwrite: OverwriteView;
  ue4ss: Ue4ssStatus;
  /** The game changed build since the user last acknowledged it. */
  gameUpdate: GameUpdate | null;
}

/** A named set of enabled mods and a load order. */
export interface ProfileView {
  id: number;
  name: string;
  active: boolean;
  enabledCount: number;
}

/** How enabled mods reach the game: hard links in the folder, or the virtual
    file system when the game is started from the manager. */
export type DeploymentMode = "hardlink" | "virtual";

export type UpdateChannel = "stable" | "nightly";

/** One entry of the virtual file tree, like Mod Organizer 2's Data tab. */
export interface VfsEntry {
  name: string;
  isDir: boolean;
  /** Game-relative path, for navigating into a folder. */
  path: string;
  /** Winning provider: a mod's name, "Game", "Overwrite" or the UE4SS list. */
  providedBy: string;
  /** "mod" | "game" | "overwrite" | "generated". */
  origin: string;
  modId: number | null;
  /** Hidden providers of the same path, the next in line first. */
  shadowedBy: string[];
  /** The real file the winning version comes from. */
  source: string | null;
}

export interface VfsListing {
  dir: string;
  entries: VfsEntry[];
}

export interface AvailableUpdate {
  version: string;
  notes: string | null;
  date: string | null;
}

export interface UpdateInfo {
  currentVersion: string;
  available: AvailableUpdate | null;
  channel: UpdateChannel;
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
  /** The user has to be asked: a type, or which version to install. */
  needsConfirmation: boolean;
  /** The versions the archive offers, when it holds several. */
  variants: VariantChoice | null;
  /** What was chosen when this mod was installed before. */
  rememberedVariants: string[];
  /** The archive's FOMOD installer, when it has one. */
  fomod: FomodView | null;
  /** Pictures in the archive, relative to the staging folder, previews first. */
  previews: string[];
}

export type FomodGroupKind =
  | "selectExactlyOne"
  | "selectAtMostOne"
  | "selectAtLeastOne"
  | "selectAll"
  | "selectAny";

export type FomodPluginType = "required" | "recommended" | "optional" | "couldBeUsable" | "notUsable";

export interface FomodPlugin {
  name: string;
  description: string;
  /** Relative to the installer's root. */
  image: string | null;
}

export interface FomodGroup {
  name: string;
  kind: FomodGroupKind;
  plugins: FomodPlugin[];
}

export interface FomodStep {
  name: string;
  groups: FomodGroup[];
}

export interface FomodInstaller {
  name: string;
  /** The header image, relative to the installer's root. */
  image: string | null;
  steps: FomodStep[];
}

/** A FOMOD installer ready to run. */
export interface FomodView {
  /** The folder holding fomod/, relative to the staging folder. */
  root: string;
  installer: FomodInstaller;
  /** What to tick when the wizard opens, as [step, group, option]. */
  selection: [number, number, number][];
  /** True when selection is the user's choice from last time. */
  remembered: boolean;
}

/** The wizard's state for a selection. */
export interface FomodWizardState {
  visible: boolean[];
  /** [step][group][option]. */
  kinds: FomodPluginType[][][];
  /** What stops the install, e.g. "Colour: choose one option". */
  problems: string[];
}

/** One version of a mod an archive offers. */
export interface VariantOption {
  label: string;
  folder: string;
  modTypes: ModTypeId[];
  fileCount: number;
  /** A picture in the option's folder, relative to the staging folder. */
  preview: string | null;
}

export interface VariantChoice {
  options: VariantOption[];
  /** Every option clashes with every other, so exactly one makes sense. */
  exclusive: boolean;
}

export interface ApplyReport {
  deployed: string[];
  removed: string[];
  failed: { name: string; reason: string }[];
}

export interface NexusAccount {
  name: string;
  isPremium: boolean;
  userId: number;
}

export interface RateLimit {
  hourlyRemaining: number | null;
  hourlyLimit: number | null;
  dailyRemaining: number | null;
  dailyLimit: number | null;
  resetAfterSecs: number | null;
}

export type DownloadState =
  | "queued"
  | "running"
  | "done"
  | "failed"
  | "cancelled"
  /** A free account has to press "Mod Manager Download" on the mod page. */
  | "needsUserAction";

export interface QueueItem {
  id: number;
  modId: number;
  fileId: number;
  name: string;
  fileName: string;
  state: DownloadState;
  bytesDone: number;
  bytesTotal: number | null;
  error: string | null;
  collection: string | null;
}

export interface Claimant {
  modId: number;
  name: string;
  priority: number;
  /** True for the one the game actually loads. */
  wins: boolean;
}

export interface Conflict {
  /** An asset path, or a chunk id when the container had no directory index. */
  asset: string;
  named: boolean;
  claimants: Claimant[];
}

export interface ModConflictCount {
  modId: number;
  /** Assets this mod provides that a later mod replaces. */
  losing: number;
  /** Assets this mod takes from an earlier one. */
  winning: number;
}

export interface ConflictReport {
  conflicts: Conflict[];
  overridden: ModConflictCount[];
  /** Mods whose containers could not be read, so the picture is incomplete. */
  unreadable: string[];
}

export interface PlannedMod {
  modId: number;
  fileId: number;
  name: string;
  version: string | null;
  optional: boolean;
  /** True when this mod is already installed, so it can be skipped. */
  installed: boolean;
}

export interface ManualMod {
  name: string;
  url: string | null;
  reason: string;
}

export interface CollectionPlan {
  name: string;
  slug: string;
  revision: number;
  mods: PlannedMod[];
  /** Entries hosted somewhere the API cannot reach. */
  manual: ManualMod[];
}

export interface QueuedFile {
  modId: number;
  fileId: number;
  name: string;
  version: string | null;
}

export type UpscalerComponent =
  | "dlssSuperResolution"
  | "dlssFrameGeneration"
  | "dlssRayReconstruction"
  | "fsrDx12"
  | "fsrVulkan";

export interface UpscalerFile {
  component: UpscalerComponent;
  /** Relative to the game root. */
  path: string;
  version: string | null;
}

export interface UpscalerEntry {
  component: UpscalerComponent;
  label: string;
  installed: UpscalerFile[];
  installedVersion: string | null;
  latestVersion: string | null;
  /** False when the graphics card cannot run this DLL at all. */
  supported: boolean;
  blockedReason: string | null;
  /** Set when the manager put the current file there. */
  managedVersion: string | null;
  note: string | null;
}

export interface Adapter {
  name: string;
  vendor: "nvidia" | "amd" | "intel" | "other";
  memoryBytes: number;
}

export interface UpscalerStatus {
  adapter: Adapter | null;
  entries: UpscalerEntry[];
}

/** Proxy DLL names the RTX 20/30 frame generation package can load under. */
export type FramegenProxy = "version" | "winmm" | "dbghelp" | "dinput8" | "dxgi" | "d3d12";

export type FramegenCheckState = "pass" | "warn" | "fail" | "info";

export interface FramegenCheck {
  id: string;
  state: FramegenCheckState;
  title: string;
  detail: string;
}

export type FramegenVerdict =
  | "compatible"
  | "compatibleWithWarnings"
  | "notNeeded"
  | "incompatible";

export interface FramegenAssessment {
  verdict: FramegenVerdict;
  checks: FramegenCheck[];
}

export interface FramegenInstall {
  modId: number;
  version: string;
  proxy: FramegenProxy;
  enabled: boolean;
}

export interface FramegenStatus {
  adapter: Adapter | null;
  /** Null until a game folder has been chosen. */
  assessment: FramegenAssessment | null;
  proxy: FramegenProxy;
  installed: FramegenInstall | null;
  /** Null when GitHub could not be asked. */
  latestVersion: string | null;
  projectUrl: string;
}

export interface FramegenProgress {
  stage: "download" | "verify";
  done: number;
  total: number | null;
}

export interface Folders {
  library: string;
  mods: string;
  backups: string;
  downloads: string;
  /** What the game created while running under the virtual file system. */
  overwrite: string;
  game: string | null;
  /** False when the library is on a different drive from the game, which
      means mods are copied instead of hard-linked and stored twice. */
  sameVolumeAsGame: boolean;
  libraryBytes: number;
}

export interface LibraryMoveReport {
  folders: Folders;
  redeployed: ApplyReport;
}

export const ipc = {
  snapshot: () => invoke<AppSnapshot>("snapshot"),
  discoverGames: () => invoke<GameInstall[]>("discover_games"),
  setGameRoot: (path: string) => invoke<GameInstall>("set_game_root", { path }),

  stageArchive: (path: string) => invoke<StagedInstall>("stage_archive", { path }),
  stageFolder: (path: string) => invoke<StagedInstall>("stage_folder", { path }),
  confirmInstall: (
    stagingId: string,
    name: string,
    typeOverride?: ModTypeId,
    variants?: string[],
  ) =>
    invoke<number>("confirm_install", {
      stagingId,
      name,
      typeOverride: typeOverride ?? null,
      variants: variants ?? null,
    }),
  cancelInstall: (stagingId: string) => invoke<void>("cancel_install", { stagingId }),
  fomodState: (stagingId: string, selection: [number, number, number][]) =>
    invoke<FomodWizardState>("fomod_state", { stagingId, selection }),
  confirmInstallFomod: (
    stagingId: string,
    name: string,
    selection: [number, number, number][],
  ) => invoke<number>("confirm_install_fomod", { stagingId, name, selection }),
  /** A picture from a staged install or an installed mod, as a data: URL. */
  previewImage: (folder: string, path: string) =>
    invoke<string>("preview_image", { folder, path }),

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
  setLibraryRoot: (path: string) => invoke<LibraryMoveReport>("set_library_root", { path }),

  setNexusKey: (apiKey: string) => invoke<NexusAccount | null>("set_nexus_key", { apiKey }),
  nexusRateLimit: () => invoke<RateLimit>("nexus_rate_limit"),

  addNxmLink: (url: string) => invoke<void>("add_nxm_link", { url }),
  checkModUpdates: () => invoke<UpdateCheckReport>("check_mod_updates"),
  downloadQueue: () => invoke<QueueItem[]>("download_queue"),
  cancelDownload: (id: number) => invoke<void>("cancel_download", { id }),
  clearFinishedDownloads: () => invoke<void>("clear_finished_downloads"),

  conflicts: () => invoke<ConflictReport>("conflicts"),
  indexModAssets: () => invoke<number>("index_mod_assets"),

  resolveCollection: (link: string) => invoke<CollectionPlan>("resolve_collection", { link }),
  installCollection: (slug: string, files: QueuedFile[]) =>
    invoke<number>("install_collection", { slug, files }),

  upscalerStatus: () => invoke<UpscalerStatus>("upscaler_status"),
  updateUpscaler: (component: UpscalerComponent) =>
    invoke<string>("update_upscaler", { component }),
  restoreUpscaler: (component: UpscalerComponent) =>
    invoke<boolean>("restore_upscaler", { component }),

  framegenStatus: (proxy?: FramegenProxy) =>
    invoke<FramegenStatus>("framegen_status", { proxy: proxy ?? null }),
  installFramegen: (proxy: FramegenProxy) => invoke<string>("install_framegen", { proxy }),
  removeFramegen: () => invoke<boolean>("remove_framegen"),

  checkForUpdate: () => invoke<UpdateInfo>("check_for_update"),
  installUpdate: () => invoke<void>("install_update"),
  setUpdateChannel: (channel: UpdateChannel) =>
    invoke<void>("set_update_channel", { channel }),
  setDiscordRpc: (enabled: boolean) => invoke<void>("set_discord_rpc", { enabled }),
  setDeploymentMode: (mode: DeploymentMode) =>
    invoke<ApplyReport>("set_deployment_mode", { mode }),
  launchGame: () => invoke<string>("launch_game"),
  vfsTree: (dir: string) => invoke<VfsListing>("vfs_tree", { dir }),
  vfsConflicts: (modId: number | null) => invoke<VfsEntry[]>("vfs_conflicts", { modId }),
  createModFromOverwrite: (name: string) => invoke<number>("create_mod_from_overwrite", { name }),
  clearOverwrite: () => invoke<number>("clear_overwrite"),
  /** Null when GitHub could not be asked. */
  ue4ssLatest: () => invoke<string | null>("ue4ss_latest"),
  installUe4ss: () => invoke<string>("install_ue4ss"),
  reshadeView: () => invoke<ReshadeView>("reshade_view"),
  setReshadePreset: (path: string) => invoke<void>("set_reshade_preset", { path }),
  setProfileReshadePreset: (profileId: number, path: string | null) =>
    invoke<void>("set_profile_reshade_preset", { profileId, path }),
  duplicateReshadePreset: (path: string, name: string) =>
    invoke<string>("duplicate_reshade_preset", { path, name }),
  renameReshadePreset: (path: string, name: string) =>
    invoke<string>("rename_reshade_preset", { path, name }),
  deleteReshadePreset: (path: string) => invoke<void>("delete_reshade_preset", { path }),
  /** Write the diagnostics zip to `path`; resolves to where it went. */
  collectDiagnostics: (path: string) => invoke<string>("collect_diagnostics", { path }),
  createProfile: (name: string, copyEnabled: boolean) =>
    invoke<number>("create_profile", { name, copyEnabled }),
  renameProfile: (id: number, name: string) => invoke<void>("rename_profile", { id, name }),
  deleteProfile: (id: number) => invoke<void>("delete_profile", { id }),
  switchProfile: (id: number) => invoke<ApplyReport>("switch_profile", { id }),
  /** What a profile's shortcut runs, as one line. */
  profileCommand: (profileId: number, launch: boolean) =>
    invoke<string>("profile_command", { profileId, launch }),
  /** Resolves to the shortcut's path. */
  createProfileShortcut: (profileId: number, place: ShortcutPlace, launch: boolean) =>
    invoke<string>("create_profile_shortcut", { profileId, place, launch }),
  /** Messages about command-line requests, each handed over once. */
  takeNotices: () => invoke<Notice[]>("take_notices"),
  acknowledgeGameUpdate: () => invoke<void>("acknowledge_game_update"),
  repairDrift: () => invoke<RepairReport>("repair_drift"),
  restorePoints: () => invoke<RestorePoint[]>("restore_points"),
  createRestorePoint: (label: string) => invoke<number>("create_restore_point", { label }),
  deleteRestorePoint: (id: number) => invoke<void>("delete_restore_point", { id }),
  restoreToPoint: (id: number) => invoke<RestoreReport>("restore_to_point", { id }),
  saves: () => invoke<Saves>("saves"),
  backUpSaves: () => invoke<SaveBackup | null>("back_up_saves"),
  restoreSaves: (file: string) => invoke<void>("restore_saves", { file }),
  deleteSaveBackup: (file: string) => invoke<void>("delete_save_backup", { file }),
  /** Null goes back to where the game keeps its saves. */
  setSavesDir: (dir: string | null) => invoke<void>("set_saves_dir", { dir }),
  setBackupSavesOnLaunch: (on: boolean) => invoke<void>("set_backup_saves_on_launch", { on }),
  configFiles: (scope: ConfigScope) => invoke<ConfigFile[]>("config_files", { scope }),
  readConfig: (scope: ConfigScope, path: string) =>
    invoke<ConfigText>("read_config", { scope, path }),
  writeConfig: (scope: ConfigScope, path: string, text: string) =>
    invoke<void>("write_config", { scope, path, text }),
  setModNotes: (modId: number, notes: string) => invoke<void>("set_mod_notes", { modId, notes }),
  setModTags: (modId: number, tags: string[]) => invoke<void>("set_mod_tags", { modId, tags }),
  modImages: (modId: number) => invoke<string[]>("mod_images", { modId }),
  setModImage: (modId: number, image: string | null) =>
    invoke<void>("set_mod_image", { modId, image }),
  modContents: (modId: number) => invoke<ModContents>("mod_contents", { modId }),
  orderRules: () => invoke<OrderRule[]>("order_rules"),
  /** Adds the rule and sorts the list by all rules; resolves to how many mods moved. */
  addOrderRule: (before: number, after: number) =>
    invoke<number>("add_order_rule", { before, after }),
  deleteOrderRule: (id: number) => invoke<void>("delete_order_rule", { id }),
  sortByRules: () => invoke<number>("sort_by_rules"),
  nexusView: (modId: number) => invoke<NexusView | null>("nexus_view", { modId }),
  refreshNexusDetails: (modId: number, force: boolean) =>
    invoke<NexusView | null>("refresh_nexus_details", { modId, force }),
  storageReport: () => invoke<StorageReport>("storage_report"),
  /** Resolves to the bytes freed. */
  cleanStorage: (what: Cleanup) => invoke<number>("clean_storage", { what }),
  watchSettings: () => invoke<WatchSettings>("watch_settings"),
  setWatchDir: (dir: string | null) => invoke<void>("set_watch_dir", { dir }),
  setWatchAutoInstall: (on: boolean) => invoke<void>("set_watch_auto_install", { on }),
  markSeen: (keys: string[]) => invoke<void>("mark_seen", { keys }),
  /** Resolves to how many mods were written. */
  exportModList: (path: string) => invoke<number>("export_mod_list", { path }),
  planImport: (path: string) => invoke<ImportPlan>("plan_import", { path }),
  importModList: (path: string, profile: string) =>
    invoke<ImportOutcome>("import_mod_list", { path, profile }),
  updatableMods: () => invoke<UpdatableMod[]>("updatable_mods"),
  /** Queue the newest files; each replaces its old version on install. */
  updateMods: (ids: number[]) => invoke<UpdatesQueued>("update_mods", { ids }),
  /** All installed mods when `ids` is null. */
  redetectMods: (ids: number[] | null) => invoke<RedetectReport>("redetect_mods", { ids }),
};

export type ShortcutPlace = "desktop" | "startMenu";

/** What detecting installed mods again changed. */
export interface RedetectReport {
  changed: string[];
  unchanged: number;
  unsure: string[];
  applied: ApplyReport | null;
}

/** One entry of a mod list being imported. */
export interface ImportEntry {
  name: string;
  version: string | null;
  enabled: boolean;
  status: "installed" | "download" | "manual";
  modId: number | null;
  nexusModId: number | null;
  nexusFileId: number | null;
}

export interface ImportPlan {
  /** The profile it was exported from. */
  profile: string;
  entries: ImportEntry[];
  rules: number;
}

export interface ImportOutcome {
  profileId: number;
  profile: string;
  matched: number;
  /** Queued; each takes its place when it arrives. */
  downloads: ImportEntry[];
  /** To fetch by hand. */
  manual: string[];
  applied: ApplyReport;
}

export interface UpdatableMod {
  id: number;
  name: string;
  nexusModId: number;
  nexusFileId: number | null;
  version: string | null;
  latest: string;
}

export interface UpdatesQueued {
  queued: number;
  current: string[];
  failed: [string, string][];
}

/** Where the library's disk space goes. */
export interface StorageReport {
  total: number;
  parts: {
    /** `mods`, `downloads`, `saves` and `vfs` can be cleaned. */
    key: string;
    label: string;
    path: string;
    bytes: number;
    files: number;
    cleanableBytes: number;
    cleanableFiles: number;
  }[];
  /** Not enabled in any profile, largest first. */
  unusedMods: { id: number; name: string; bytes: number }[];
}

export type Cleanup = "orphans" | "downloads" | "saveBackups" | "logs";

/** A mod archive that appeared in the watched downloads folder. */
export interface FoundArchive {
  key: string;
  path: string;
  name: string;
  bytes: number;
}

export interface WatchSettings {
  dir: string | null;
  autoInstall: boolean;
}

/** What one mod changes. */
export interface ModContents {
  /** Paths it puts in the game folder (the first 500). */
  files: string[];
  fileCount: number;
  /** Its paks have been read; without that `areas` is empty, not "none". */
  indexed: boolean;
  assetCount: number;
  areas: { area: string; count: number; examples: string[] }[];
  clashes: { modId: number; name: string; shared: number; enabled: boolean; wins: boolean }[];
}

export interface OrderRule {
  id: number;
  beforeId: number;
  before: string;
  afterId: number;
  after: string;
  broken: boolean;
}

export interface NexusDetails {
  modId: number;
  name: string;
  author: string | null;
  version: string | null;
  summary: string | null;
  endorsements: number;
  downloads: number;
  pictureUrl: string | null;
  updatedAt: string | null;
  adult: boolean;
  category: string | null;
  tags: string[];
}

export interface RequirementView {
  /** Null for something hosted off Nexus. */
  modId: number | null;
  name: string;
  url: string;
  notes: string | null;
  state: "enabled" | "disabled" | "missing" | "external";
  installedAs: number | null;
}

export interface NexusView {
  nexusModId: number;
  details: NexusDetails | null;
  fetchedAt: string | null;
  requirements: RequirementView[];
}

/** Whose settings files: one mod's, or the game's own. */
/**
 * Whose settings files: a mod's, the game's own (in %LOCALAPPDATA%), or INI
 * files in the game folder itself (ReShade presets).
 */
export type ConfigScope =
  | { kind: "mod"; modId: number }
  | { kind: "game" }
  | { kind: "install" };

/** ReShade's DLL beside the executable. */
export interface ReshadeInstall {
  /** The name it loads under, e.g. `dxgi.dll`. */
  dll: string;
  version: string | null;
  providedBy: string;
  modId: number | null;
}

/** One ReShade preset. */
export interface ReshadePreset {
  /** Game-relative, `/`-separated. */
  path: string;
  name: string;
  /** Folder relative to the executable's; empty beside it. */
  folder: string;
  techniques: string[];
  /** Effect files it uses that ReShade will not find. */
  missingEffects: string[];
  /** Techniques that do not name their file (older presets). */
  unchecked: number;
  /** ReShade starts with it. */
  active: boolean;
  bytes: number;
  providedBy: string;
  origin: "mod" | "game" | "overwrite";
  modId: number | null;
  /** Where to edit it; null for one in overwrite. */
  source: { scope: ConfigScope; path: string } | null;
}

export interface ReshadeView {
  installed: ReshadeInstall | null;
  config: string | null;
  configProvidedBy: string | null;
  /** The preset ReShade starts with. */
  active: string | null;
  activeFound: boolean;
  presets: ReshadePreset[];
  /** Effect files ReShade can find. */
  effects: number;
  /** The preset the active profile asks for. */
  profilePreset: string | null;
  downloadUrl: string;
}

export interface ConfigFile {
  path: string;
  bytes: number;
  /** Differs from the copy it was installed (or first found) as. */
  edited: boolean;
}

export interface ConfigText {
  text: string;
  /** As installed or first found; null when no copy was kept. */
  original: string | null;
}

/** What identifies an installed build of the game. */
export interface GameBuild {
  /** Steam's build id, when Steam installed the game. */
  buildId: string | null;
  exeSize: number | null;
  /** Seconds since the Unix epoch. */
  exeModified: number | null;
}

export interface GameUpdate {
  previous: GameBuild;
  current: GameBuild;
  /** Deployed files the update replaced or removed. */
  drifted: number;
}

export interface RepairReport {
  relinked: number;
  displaced: number;
  adopted: number;
  recorded: number;
  failed: ApplyReport["failed"];
}

export interface RestorePoint {
  id: number;
  /** UTC, ISO 8601. */
  createdAt: string;
  label: string;
  automatic: boolean;
  gameBuild: string | null;
  enabled: number;
  /** Mods it had enabled that are no longer installed. */
  uninstalled: number;
}

export interface RestoreReport {
  applied: ApplyReport;
  uninstalled: number;
}

export interface SaveBackup {
  file: string;
  /** Seconds since the Unix epoch. */
  createdAt: number;
  kind: "auto" | "manual" | "before-restore";
  bytes: number;
}

export interface Saves {
  dir: string | null;
  customDir: boolean;
  exists: boolean;
  files: number;
  backupOnLaunch: boolean;
  backups: SaveBackup[];
}

/** The outcome of a request from the command line or a shortcut. */
export interface Notice {
  kind: "success" | "error";
  message: string;
}

/** The game's Nexus domain, mirroring `NEXUS_DOMAIN` in `sbmm-game`. */
const NEXUS_DOMAIN = "stellarblade";

export function modPageUrl(nexusModId: number): string {
  return `https://www.nexusmods.com/${NEXUS_DOMAIN}/mods/${nexusModId}?tab=files`;
}

/** Display names and accent colours for each payload type. */
export const MOD_TYPE_META: Record<ModTypeId, { label: string; hint: string }> = {
  genericPak: { label: "Pak", hint: "SB/Content/Paks/~mods" },
  logicMod: { label: "Logic", hint: "SB/Content/Paks/LogicMods" },
  ue4ssLua: { label: "Lua", hint: "UE4SS Lua mod" },
  ue4ssDll: { label: "C++", hint: "UE4SS native mod" },
  ue4ssFramework: { label: "UE4SS", hint: "UE4SS runtime" },
  rootBinary: { label: "Root", hint: "SB/Binaries/Win64" },
  movie: { label: "Movie", hint: "SB/Content/Movies" },
  splash: { label: "Splash", hint: "SB/Content/Splash — the startup image" },
  gameRootOverlay: { label: "Overlay", hint: "Laid onto the game folder" },
  configTweak: {
    label: "Engine tweak",
    hint: "Engine.ini and the like — merged into the game's settings in %LOCALAPPDATA%\\SB\\Saved\\Config",
  },
  reshadePreset: {
    label: "ReShade preset",
    hint: "SB/Binaries/Win64/reshade-presets — picked on the ReShade screen",
  },
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
  "splash",
  "gameRootOverlay",
  "configTweak",
  "reshadePreset",
];
