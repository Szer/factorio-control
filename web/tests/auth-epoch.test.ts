import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../src/api";
import { operations } from "../src/operationsApi";
import { modWorkflowApi } from "../src/modWorkflowApi";
import {
  authApi,
  authEvent,
  authExpired,
  authDiscard,
  captureAuthEpoch,
} from "../src/authApi";
import type { SessionStatus } from "../src/authApi";

const original: SessionStatus = {
  user: {
    account_id: "owner-one",
    username: "owner",
    role: "admin",
    must_change_password: false,
  },
  expires_at: 9999999999,
};
const replacement: SessionStatus = {
  user: {
    account_id: "owner-two",
    username: "another",
    role: "admin",
    must_change_password: false,
  },
  expires_at: 9999999999,
};
const helpers = [
  { name: "server API", request: () => api.status() },
  { name: "account API", request: () => authApi.accounts() },
  { name: "operations API", request: () => operations.fileLog() },
  { name: "mod workflows API", request: () => modWorkflowApi.packs() },
];
function json(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), { status });
}
const expired = vi.fn();
beforeEach(() => {
  expired.mockClear();
  window.addEventListener(authEvent, expired);
});
afterEach(() => {
  window.removeEventListener(authEvent, expired);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

for (const next of [original, replacement]) {
  it.each(helpers)(
    `ignores an old $name 401 after ${next === original ? "same-account same-second renewal" : "another account signs in"}`,
    async ({ request }) => {
      let session = original;
      let finish: ((response: Response) => void) | undefined;
      vi.stubGlobal(
        "fetch",
        vi.fn((path: string) =>
          path === "/api/auth/login"
            ? Promise.resolve(json(session))
            : new Promise<Response>((resolve) => {
                finish = resolve;
              }),
        ),
      );
      await authApi.login("owner", "synthetic-password");
      const old = request().catch((error: unknown) => error);
      const before = captureAuthEpoch();
      session = next;
      await authApi.login(next.user?.username ?? "owner", "synthetic-password");
      const renewed = captureAuthEpoch();
      expect(renewed).toBeGreaterThan(before);
      finish?.(json({ message: "Expired old session" }, 401));
      expect(await old).toBeInstanceOf(Error);
      expect(expired).not.toHaveBeenCalled();
      expect(captureAuthEpoch()).toBe(renewed);
    },
  );
}

it.each(helpers)(
  "expires the current session for a current $name 401",
  async ({ request }) => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(json({ message: "Current session expired" }, 401)),
    );
    const before = captureAuthEpoch();
    await expect(request()).rejects.toThrow("Current session expired");
    expect(expired).toHaveBeenCalledTimes(1);
    expect(captureAuthEpoch()).toBeGreaterThan(before);
  },
);

it("does not advance the generation for unchanged status polls", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockImplementation(() => Promise.resolve(json(original))),
  );
  await authApi.status();
  const first = captureAuthEpoch();
  await authApi.status();
  expect(captureAuthEpoch()).toBe(first);
});

it("does not accept side effects from an old successful status response", async () => {
  let finish: ((response: Response) => void) | undefined;
  vi.stubGlobal(
    "fetch",
    vi.fn((path: string) =>
      path === "/api/auth/login"
        ? Promise.resolve(json(replacement))
        : new Promise<Response>((resolve) => {
            finish = resolve;
          }),
    ),
  );
  const old = authApi.status().catch((error: unknown) => error);
  await authApi.login("another", "synthetic-password");
  const current = captureAuthEpoch();
  finish?.(json({ user: null, expires_at: null }));
  expect(await old).toBeInstanceOf(Error);
  expect(captureAuthEpoch()).toBe(current);
  vi.stubGlobal(
    "fetch",
    vi.fn().mockImplementation(() => Promise.resolve(json(replacement))),
  );
  await authApi.status();
  expect(captureAuthEpoch()).toBe(current);
});

it("ignores scoped expiry and logout callbacks after the session generation changes", () => {
  const old = captureAuthEpoch();
  expect(authExpired(old)).toBe(true);
  expect(authExpired(old)).toBe(false);
  expect(authDiscard(old)).toBe(false);
  expect(expired).toHaveBeenCalledTimes(1);
});
