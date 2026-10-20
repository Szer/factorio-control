import { authExpired, captureAuthEpoch } from "./authApi";
import { z } from "zod";
import type { components } from "./api.generated";

export const statusSchema = z.object({
  state: z.enum([
    "stopped",
    "starting",
    "running",
    "paused",
    "stopping",
    "failed",
  ]),
  since: z.number(),
  active_save: z.string().nullable(),
  started_at: z.number().nullable(),
  binary_available: z.boolean(),
  version: z.string().nullable(),
  error: z.string().nullable(),
}) satisfies z.ZodType<components["schemas"]["Status"]>;
export type Status = z.infer<typeof statusSchema>;
export const saveSchema = z.object({
  name: z.string(),
  size_bytes: z.number(),
  modified_at: z.number(),
  revision: z.string(),
}) satisfies z.ZodType<components["schemas"]["Save"]>;
export type Save = z.infer<typeof saveSchema>;
const unsigned = z.number().int().min(0).max(4294967295);
export const settingsSchema = z.object({
  name: z.string().trim().min(1, "Give your server a name.").max(100),
  description: z.string().max(2000),
  tags: z.array(z.string()),
  max_players: unsigned.max(65535),
  visibility: z.object({ public: z.boolean(), lan: z.boolean() }),
  username: z.string(),
  password: z.string(),
  token: z.string(),
  game_password: z.string(),
  require_user_verification: z.boolean(),
  max_upload_in_kilobytes_per_second: unsigned,
  max_upload_slots: unsigned,
  minimum_latency_in_ticks: unsigned,
  max_heartbeats_per_second: unsigned,
  ignore_player_limit_for_returning_players: z.boolean(),
  allow_commands: z.union([z.boolean(), z.literal("admins-only")]),
  autosave_interval: unsigned.max(1440),
  autosave_slots: unsigned,
  afk_autokick_interval: unsigned,
  auto_pause: z.boolean(),
  auto_pause_when_players_connect: z.boolean(),
  only_admins_can_pause_the_game: z.boolean(),
  autosave_only_on_server: z.boolean(),
  non_blocking_saving: z.boolean(),
  minimum_segment_size: unsigned,
  minimum_segment_size_peer_count: unsigned,
  maximum_segment_size: unsigned,
  maximum_segment_size_peer_count: unsigned,
}) satisfies z.ZodType<components["schemas"]["ServerSettings"]>;
export type Settings = z.infer<typeof settingsSchema>;
export const settingsDocumentSchema = z.object({
  revision: z.string(),
  settings: settingsSchema,
}) satisfies z.ZodType<components["schemas"]["SettingsDocument"]>;
export type SettingsDocument = z.infer<typeof settingsDocumentSchema>;
export const rawSettingsSchema = z.object({
  revision: z.string(),
  settings: z.record(z.string(), z.unknown()),
}) satisfies z.ZodType<components["schemas"]["RawSettingsDocument"]>;
export type RawSettingsDocument = z.infer<typeof rawSettingsSchema>;
export const managerSchema = z.object({
  revision: z.string(),
  settings: z.object({
    poll_interval_seconds: z.number().int().min(1).max(60),
    ups_history_minutes: z.number().int().min(1).max(1440),
    enable_script_telemetry: z.boolean(),
    game_bind_address: z.string().default("0.0.0.0"),
    use_server_whitelist: z.boolean().default(false),
    game_port: z.number().int().min(1).max(65535).default(34197),
  }),
}) satisfies z.ZodType<components["schemas"]["ManagerSettingsDocument"]>;
export type ManagerDocument = z.infer<typeof managerSchema>;
export const selectionSchema = z.object({
  revision: z.string(),
  save: z.string().nullable(),
  load_latest: z.boolean().default(false),
}) satisfies z.ZodType<components["schemas"]["SaveSelectionDocument"]>;
export const adminsSchema = z.object({
  revision: z.string(),
  admins: z.array(z.string()),
}) satisfies z.ZodType<components["schemas"]["AdminsDocument"]>;
export type AdminsDocument = z.infer<typeof adminsSchema>;
export const logSchema = z.object({
  id: z.number(),
  timestamp: z.number(),
  level: z.enum(["info", "warn", "error"]),
  message: z.string(),
}) satisfies z.ZodType<components["schemas"]["LogEntry"]>;
export type LogEntry = z.infer<typeof logSchema>;
export const telemetrySchema = z.object({
  enabled: z.boolean(),
  available: z.boolean(),
  sampled_at: z.number().nullable(),
  players: z.array(z.string()).nullable(),
  research: z
    .array(
      z.object({
        force: z.string(),
        name: z.string(),
        level: z.number(),
        progress: z.number(),
      }),
    )
    .nullable(),
  ups: z.number().nullable(),
  target_ups: z.number().nullable(),
  history: z.array(
    z.object({
      timestamp: z.number(),
      ups: z.number().nullable(),
      target_ups: z.number().nullable(),
      state: z.enum(["active", "paused", "collecting", "unavailable"]),
    }),
  ),
  activity: z.array(
    z.object({
      id: z.number(),
      timestamp: z.number(),
      kind: z.enum([
        "started",
        "stopped",
        "joined",
        "left",
        "research_finished",
      ]),
      message: z.string(),
    }),
  ),
  error: z.string().nullable(),
}) satisfies z.ZodType<components["schemas"]["Snapshot"]>;
export type Telemetry = z.infer<typeof telemetrySchema>;
const modKind = z.enum([
  "boolean",
  "number",
  "string",
  "integer",
  "unsigned_integer",
  "color",
  "unsupported",
]);
export const modSettingsSchema = z.object({
  revision: z.string(),
  available: z.boolean(),
  settings: z.array(
    z.object({
      name: z.string(),
      kind: modKind,
      value: z.string(),
      components: z.array(
        z.object({ name: z.string(), kind: modKind, value: z.string() }),
      ),
    }),
  ),
}) satisfies z.ZodType<components["schemas"]["ModSettingsDocument"]>;
export type ModSettingsDocument = z.infer<typeof modSettingsSchema>;
export type ModSettingsChange = {
  value?: string;
  components?: Record<string, string>;
};
export const modsSchema = z.object({
  revision: z.string(),
  portal_authenticated: z.boolean(),
  mods: z.array(
    z.object({
      name: z.string(),
      version: z.string(),
      title: z.string(),
      author: z.string(),
      description: z.string(),
      factorio_version: z.string(),
      dependencies: z.array(z.string()),
      enabled: z.boolean(),
      file_name: z.string(),
      size_bytes: z.number(),
    }),
  ),
}) satisfies z.ZodType<components["schemas"]["ModsDocument"]>;
export type ModsDocument = z.infer<typeof modsSchema>;
export const modUpdatesSchema = z.object({
  name: z.string(),
  installed_version: z.string(),
  latest_version: z.string().nullable(),
  latest_released_at: z.string().nullable(),
  factorio_min_version: z.string(),
  compatible: z.boolean(),
  update_available: z.boolean(),
  authenticated: z.boolean(),
  factorio_version: z.string(),
  message: z.string().nullable(),
}) satisfies z.ZodType<components["schemas"]["ModUpdates"]>;
export type ModUpdates = z.infer<typeof modUpdatesSchema>;

export const saveSyncPlanSchema = z.object({
  revision: z.string(),
  save: z.string(),
  factorio_version: z.string(),
  engine_version: z.string(),
  mods: z.array(
    z.object({
      name: z.string(),
      version: z.string(),
      action: z.enum(["keep", "enable", "switch", "download", "bundled"]),
    }),
  ),
  disable: z.array(z.string()),
  blockers: z.array(z.string()),
}) satisfies z.ZodType<components["schemas"]["SyncPlan"]>;
export type SaveSyncPlan = z.infer<typeof saveSyncPlanSchema>;

export const syncProgressSchema = z.object({
  operation_id: z.string(),
  save: z.string(),
  phase: z.enum([
    "preparing",
    "resolving",
    "downloading",
    "validating",
    "applying",
    "complete",
    "failed",
  ]),
  completed: z.number().int().nonnegative(),
  total: z.number().int().nonnegative(),
  current_mod: z.string().nullable(),
  error: z.string().nullable(),
}) satisfies z.ZodType<components["schemas"]["SyncProgress"]>;

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "ApiError";
  }
}
async function request<T>(
  url: string,
  schema: z.ZodType<T>,
  init?: RequestInit,
): Promise<T> {
  const epoch = captureAuthEpoch();
  const response = await fetch(url, init);
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    if (response.status === 401) authExpired(epoch);
    const problem = z.object({ message: z.string() }).safeParse(body);
    throw new ApiError(
      problem.success
        ? problem.data.message
        : `Request failed (${String(response.status)}). Please try again.`,
      response.status,
    );
  }
  const parsed = schema.safeParse(body);
  if (!parsed.success)
    throw new ApiError(
      "The server returned an unexpected response. Your changes have not been confirmed.",
      502,
    );
  return parsed.data;
}
const json = (method: "POST" | "PUT", body: unknown): RequestInit => ({
  method,
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(body),
});
export const api = {
  status: () => request("/api/status", statusSchema),
  saves: () => request("/api/saves", z.array(saveSchema)),
  selection: () => request("/api/saves/selection", selectionSchema),
  selectSave: (document: z.infer<typeof selectionSchema>) =>
    request("/api/saves/selection", selectionSchema, json("PUT", document)),
  settings: () => request("/api/settings", settingsDocumentSchema),
  saveSettings: (document: SettingsDocument) =>
    request("/api/settings", settingsDocumentSchema, json("PUT", document)),
  rawSettings: () => request("/api/settings/raw", rawSettingsSchema),
  saveRawSettings: (document: RawSettingsDocument) =>
    request("/api/settings/raw", rawSettingsSchema, json("PUT", document)),
  manager: () => request("/api/manager-settings", managerSchema),
  saveManager: (document: ManagerDocument) =>
    request("/api/manager-settings", managerSchema, json("PUT", document)),
  admins: () => request("/api/admins", adminsSchema),
  saveAdmins: (document: AdminsDocument) =>
    request("/api/admins", adminsSchema, json("PUT", document)),
  logs: () => request("/api/logs", z.array(logSchema)),
  telemetry: () => request("/api/telemetry", telemetrySchema),
  start: (save?: string) =>
    request(
      "/api/server/start",
      statusSchema,
      json("POST", save ? { save } : {}),
    ),
  stop: () => request("/api/server/stop", statusSchema, json("POST", {})),
  upload: (file: File) =>
    request(`/api/saves/${encodeURIComponent(file.name)}`, saveSchema, {
      method: "POST",
      headers: { "Content-Type": "application/zip" },
      body: file,
    }),
  saveModPlan: (name: string) =>
    request(`/api/saves/${encodeURIComponent(name)}/mods`, saveSyncPlanSchema),
  saveSyncProgress: (name: string) =>
    request(
      `/api/saves/${encodeURIComponent(name)}/mods/sync`,
      syncProgressSchema.nullable(),
    ),
  syncSaveMods: ({
    name,
    revision,
    operation_id,
  }: {
    name: string;
    revision: string;
    operation_id?: string;
  }) =>
    request(
      `/api/saves/${encodeURIComponent(name)}/mods/sync`,
      modsSchema,
      json("POST", { revision, operation_id }),
    ),
  mods: () => request("/api/mods", modsSchema),
  uploadMod: ({ file, revision }: { file: File; revision: string }) =>
    request("/api/mods/upload", modsSchema, {
      method: "POST",
      headers: { "Content-Type": "application/zip", "If-Match": revision },
      body: file,
    }),
  enableMod: ({
    name,
    revision,
    enabled,
  }: {
    name: string;
    revision: string;
    enabled: boolean;
  }) =>
    request(
      `/api/mods/${encodeURIComponent(name)}/enabled`,
      modsSchema,
      json("PUT", { revision, enabled }),
    ),
  modUpdates: (name: string) =>
    request(`/api/mods/${encodeURIComponent(name)}/updates`, modUpdatesSchema),
  updateMod: ({
    name,
    revision,
    version,
  }: {
    name: string;
    revision: string;
    version: string;
  }) =>
    request(
      `/api/mods/${encodeURIComponent(name)}/update`,
      modsSchema,
      json("POST", { revision, version }),
    ),
  modSettings: () => request("/api/mods/settings", modSettingsSchema),
  saveModSettings: (document: {
    revision: string;
    changes: Record<string, ModSettingsChange>;
  }) => request("/api/mods/settings", modSettingsSchema, json("PUT", document)),
  uploadModSettings: ({ file, revision }: { file: File; revision: string }) =>
    request("/api/mods/settings/file", modSettingsSchema, {
      method: "POST",
      headers: {
        "Content-Type": "application/octet-stream",
        "If-Match": revision,
      },
      body: file,
    }),
};
export function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Something went wrong. Please try again.";
}
export function formatBytes(bytes: number): string {
  return bytes < 1024 * 1024
    ? `${(bytes / 1024).toFixed(1)} KB`
    : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
export function formatDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
