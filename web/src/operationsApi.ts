import { authExpired, captureAuthEpoch } from "./authApi";
import { z } from "zod";
import type { components } from "./api.generated";
import { ApiError, saveSchema, statusSchema } from "./api";

export const archivedSaveSchema = z.object({
  id: z.string(),
  name: z.string(),
  size_bytes: z.number(),
  modified_at: z.number(),
  revision: z.string(),
}) satisfies z.ZodType<components["schemas"]["ArchivedSave"]>;
export type ArchivedSave = z.infer<typeof archivedSaveSchema>;
const fileLogSchema = z.object({
  name: z.string(),
  content: z.string(),
  truncated: z.boolean(),
}) satisfies z.ZodType<components["schemas"]["GameLogDocument"]>;
async function request<T>(
  url: string,
  schema: z.ZodType<T>,
  body?: unknown,
): Promise<T> {
  const epoch = captureAuthEpoch();
  const response = await fetch(
    url,
    body === undefined
      ? undefined
      : {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        },
  );
  const value: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    if (response.status === 401) authExpired(epoch);
    const problem = z.object({ message: z.string() }).safeParse(value);
    throw new ApiError(
      problem.success ? problem.data.message : "Operation failed",
      response.status,
    );
  }
  const parsed = schema.safeParse(value);
  if (!parsed.success)
    throw new ApiError(
      "The server returned an unexpected response. Refresh and try again.",
      response.status,
    );
  return parsed.data;
}
const runtimePathsSchema = z.object({
  manager_data: z.string(),
  game_binary: z.string().nullable(),
  config_file: z.string(),
  read_data: z.string(),
  write_data: z.string(),
}) satisfies z.ZodType<components["schemas"]["RuntimePaths"]>;
export const operations = {
  runtimePaths: () => request("/api/runtime-paths", runtimePathsSchema),
  createSave: (name: string) =>
    request("/api/saves/create", saveSchema, { name }),
  archiveSave: ({ name, revision }: { name: string; revision: string }) =>
    request(
      `/api/saves/${encodeURIComponent(name)}/archive`,
      archivedSaveSchema,
      { revision },
    ),
  archivedSaves: () =>
    request("/api/saves/archived", z.array(archivedSaveSchema)),
  restoreSave: ({ id, revision }: { id: string; revision: string }) =>
    request(
      `/api/saves/archived/${encodeURIComponent(id)}/restore`,
      saveSchema,
      { revision },
    ),
  command: ({
    command,
    acknowledge_scripting,
  }: {
    command: string;
    acknowledge_scripting: boolean;
  }) =>
    request("/api/server/command", z.object({ accepted: z.literal(true) }), {
      command,
      acknowledge_scripting,
    }),
  forceStop: () =>
    request("/api/server/force-stop", statusSchema, { confirm: true }),
  fileLog: () => request("/api/logs/file", fileLogSchema),
};
