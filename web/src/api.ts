import { z } from "zod";
import type { components } from "./api.generated";
export type Status = components["schemas"]["Status"];
export type Save = components["schemas"]["Save"];
export type SettingsDocument = components["schemas"]["SettingsDocument"];
export type Settings = components["schemas"]["ServerSettings"];
export type AdminsDocument = components["schemas"]["AdminsDocument"];
export type LogEntry = components["schemas"]["LogEntry"];

export const statusSchema: z.ZodType<Status> = z.object({
  state: z.enum(["stopped", "starting", "running", "stopping", "failed"]),
  active_save: z.string().nullable(),
  started_at: z.number().nullable(),
  binary_available: z.boolean(),
  version: z.string().nullable(),
  error: z.string().nullable(),
});
export const saveSchema: z.ZodType<Save> = z.object({
  name: z.string(),
  size_bytes: z.number(),
  modified_at: z.number(),
});
export const settingsSchema: z.ZodType<Settings> = z.object({
  name: z.string().trim().min(1, "Give your server a name.").max(100),
  description: z.string().max(2000),
  max_players: z.number().int().min(0).max(65535),
  auto_pause: z.boolean(),
  autosave_interval: z.number().int().min(0).max(1440),
  visibility: z.object({ public: z.boolean(), lan: z.boolean() }),
});
export const settingsDocumentSchema: z.ZodType<SettingsDocument> = z.object({
  revision: z.string(),
  settings: settingsSchema,
});
export const adminsSchema: z.ZodType<AdminsDocument> = z.object({
  revision: z.string(),
  admins: z.array(z.string()),
});
export const logSchema: z.ZodType<LogEntry> = z.object({
  id: z.number(),
  timestamp: z.number(),
  level: z.enum(["info", "warn", "error"]),
  message: z.string(),
});

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
  const response = await fetch(url, init);
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
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
  settings: () => request("/api/settings", settingsDocumentSchema),
  saveSettings: (document: SettingsDocument) =>
    request("/api/settings", settingsDocumentSchema, json("PUT", document)),
  admins: () => request("/api/admins", adminsSchema),
  saveAdmins: (document: AdminsDocument) =>
    request("/api/admins", adminsSchema, json("PUT", document)),
  logs: () => request("/api/logs", z.array(logSchema)),
  start: (save: string) =>
    request("/api/server/start", statusSchema, json("POST", { save })),
  stop: () => request("/api/server/stop", statusSchema, json("POST", {})),
  upload: (file: File) =>
    request(`/api/saves/${encodeURIComponent(file.name)}`, saveSchema, {
      method: "POST",
      headers: { "Content-Type": "application/zip" },
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
