import { createContext, useContext } from "react";
import { z } from "zod";

export const roleSchema = z.enum(["admin", "operator", "viewer"]);
export type Role = z.infer<typeof roleSchema>;
export const accountSchema = z.object({
  username: z.string(),
  role: roleSchema,
});
export type Account = z.infer<typeof accountSchema>;
export const sessionSchema = z.object({
  enabled: z.boolean(),
  user: accountSchema.nullable(),
  expires_at: z.number().nullable(),
});
export type SessionStatus = z.infer<typeof sessionSchema>;
export const accountsSchema = z.object({
  revision: z.string(),
  accounts: z.array(accountSchema),
});
export type AccountsDocument = z.infer<typeof accountsSchema>;
export const authKey = ["auth"] as const;
export const authEvent = "fc-auth-expired";
export const AuthContext = createContext<SessionStatus>({
  enabled: false,
  user: null,
  expires_at: null,
});

export function useAccess() {
  const session = useContext(AuthContext);
  const role = session.enabled ? session.user?.role : "admin";
  return {
    ...session,
    role,
    canOperate: role === "admin" || role === "operator",
    canAdmin: role === "admin",
  };
}

export function authExpired() {
  window.dispatchEvent(new Event(authEvent));
}

async function request<T>(
  path: string,
  schema: z.ZodType<T>,
  input?: unknown,
  notify = true,
): Promise<T> {
  const response = await fetch(`/api/auth/${path}`, {
    method: input === undefined ? "GET" : "POST",
    credentials: "same-origin",
    cache: "no-store",
    ...(input === undefined
      ? {}
      : {
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(input),
        }),
  });
  if (!response.ok) {
    if (response.status === 401 && notify) authExpired();
    const body: unknown = await response.json().catch(() => null);
    const parsed = z.object({ message: z.string() }).safeParse(body);
    throw new Error(
      parsed.success ? parsed.data.message : "Account request failed",
    );
  }
  return schema.parse(
    response.status === 204 ? undefined : await response.json(),
  );
}

export const authApi = {
  status: () => request("status", sessionSchema),
  login: (username: string, password: string) =>
    request("login", sessionSchema, { username, password }, false),
  logout: () => request("logout", z.undefined(), {}),
  password: (current_password: string, password: string) =>
    request("password", z.undefined(), { current_password, password }),
  accounts: () => request("accounts", accountsSchema),
  create: (input: {
    revision: string;
    username: string;
    password: string;
    role: Role;
  }) => request("accounts/create", accountsSchema, input),
  update: (input: {
    revision: string;
    username: string;
    password: string | null;
    role: Role;
  }) => request("accounts/update", accountsSchema, input),
  delete: (revision: string, username: string) =>
    request("accounts/delete", accountsSchema, { revision, username }),
};
