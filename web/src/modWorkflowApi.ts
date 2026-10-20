import { authExpired, captureAuthEpoch } from "./authApi";
import { z } from "zod";
import { ApiError, modsSchema } from "./api";

export const listEntrySchema = z.object({
  name: z.string().min(1).max(100),
  enabled: z.boolean(),
  version: z.string().nullable().optional(),
});
export const modListFileSchema = z.object({
  mods: z.array(listEntrySchema).max(500),
});
const packSchema = z.object({
  revision: z.string(),
  name: z.string(),
  mods: z.number().int().nonnegative(),
});
const releaseSchema = z.object({
  version: z.string(),
  factorio_version: z.string(),
  released_at: z.string().nullable(),
});
const searchSchema = z.object({ name: z.string(), title: z.string() });
const bulkChangeSchema = z.object({
  name: z.string(),
  current_version: z.string(),
  version: z.string(),
});
export const bulkPlanSchema = z.object({
  revision: z.string(),
  changes: z.array(bulkChangeSchema),
});
export type BulkPlan = z.infer<typeof bulkPlanSchema>;
async function request<T>(
  path: string,
  schema: z.ZodType<T>,
  method = "GET",
  body?: unknown,
): Promise<T> {
  const epoch = captureAuthEpoch();
  const response = await fetch(`/api/mods${path}`, {
    method,
    ...(body === undefined
      ? {}
      : {
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        }),
  });
  const value: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    if (response.status === 401) authExpired(epoch);
    const error = z.object({ message: z.string() }).safeParse(value);
    throw new ApiError(
      error.success
        ? error.data.message
        : `Request failed (${String(response.status)})`,
      response.status,
    );
  }
  const result = schema.safeParse(value);
  if (!result.success)
    throw new ApiError(
      "Unexpected mod-management response; changes have not been confirmed",
      502,
    );
  return result.data;
}
const query = (revision: string) => `?revision=${encodeURIComponent(revision)}`;
export const modWorkflowApi = {
  progress: () =>
    request(
      "/operation",
      z
        .object({
          active: z.boolean(),
          stage: z.string(),
          completed: z.number().int(),
          total: z.number().int(),
          current: z.string().nullable(),
        })
        .nullable(),
    ),
  search: (term: string) =>
    request(
      `/portal/search?q=${encodeURIComponent(term)}`,
      z.array(searchSchema),
    ),
  releases: (name: string) =>
    request(
      `/portal/${encodeURIComponent(name)}/releases`,
      z.object({ name: z.string(), releases: z.array(releaseSchema) }),
    ),
  install: (revision: string, name: string, version: string) =>
    request("/portal/install", modsSchema, "POST", { revision, name, version }),
  preview: (revision: string) =>
    request("/bulk/preview", bulkPlanSchema, "POST", { revision }),
  apply: (plan: BulkPlan) => request("/bulk/apply", modsSchema, "POST", plan),
  packs: () => request("/packs", z.array(packSchema)),
  createPack: (revision: string, name: string) =>
    request("/packs", z.array(packSchema), "POST", { revision, name }),
  restorePack: (revision: string, name: string, packRevision: string) =>
    request(`/packs/${encodeURIComponent(name)}/restore`, modsSchema, "POST", {
      revision,
      pack_revision: packRevision,
    }),
  deletePack: (revision: string, name: string, packRevision: string) =>
    request(
      `/packs/${encodeURIComponent(name)}${query(revision)}&pack_revision=${encodeURIComponent(packRevision)}`,
      z.array(packSchema),
      "DELETE",
    ),
  remove: (revision: string, name: string) =>
    request(
      `/${encodeURIComponent(name)}${query(revision)}`,
      modsSchema,
      "DELETE",
    ),
  importList: (revision: string, value: z.infer<typeof modListFileSchema>) =>
    request("/list", modsSchema, "PUT", {
      revision,
      mods: value.mods.map((item) => ({
        ...item,
        version: item.version ?? null,
      })),
    }),
};
