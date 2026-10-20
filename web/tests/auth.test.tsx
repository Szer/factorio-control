import { useState } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AccountPanel, AuthGate, SessionMenu } from "../src/components/Auth";
import {
  AuthContext,
  authApi,
  authExpired,
  authKey,
  useAccess,
} from "../src/authApi";
import type { SessionStatus } from "../src/authApi";

const signedOut: SessionStatus = {
  enabled: true,
  user: null,
  expires_at: null,
};
const administrator: SessionStatus = {
  enabled: true,
  user: { username: "owner", role: "admin" },
  expires_at: 9999999999,
};
function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}
function Protected() {
  const access = useAccess();
  const [draft, setDraft] = useState("");
  return (
    <>
      <p>Protected {access.role}</p>
      <p>{access.canAdmin ? "Can administer" : "Cannot administer"}</p>
      <p>{access.canOperate ? "Can operate" : "Cannot operate"}</p>
      <input
        aria-label="Secret draft"
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
        }}
      />
      <SessionMenu />
    </>
  );
}
function mountGate(queryClient = client()) {
  render(
    <QueryClientProvider client={queryClient}>
      <AuthGate>
        <Protected />
      </AuthGate>
    </QueryClientProvider>,
  );
  return queryClient;
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("does not mount the app before session status is known", async () => {
  let resolve: ((session: SessionStatus) => void) | undefined;
  vi.spyOn(authApi, "status").mockReturnValue(
    new Promise((done) => {
      resolve = done;
    }),
  );
  mountGate();
  expect(screen.queryByText(/Protected/)).toBeNull();
  expect(screen.queryByRole("button", { name: "Sign in" })).toBeNull();
  act(() => {
    resolve?.(signedOut);
  });
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  expect(screen.queryByText(/Protected/)).toBeNull();
});

it("signs in, clears old query data, and signs out without leaving protected state", async () => {
  vi.spyOn(authApi, "status").mockResolvedValue(signedOut);
  const login = vi.spyOn(authApi, "login").mockResolvedValue(administrator);
  vi.spyOn(authApi, "logout").mockResolvedValue(undefined);
  const queryClient = mountGate();
  const user = userEvent.setup();
  await user.type(await screen.findByLabelText("Account name"), "owner");
  await user.type(screen.getByLabelText("Password"), "test-only-password");
  queryClient.setQueryData(["secret"], { token: "synthetic-test-value" });
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  expect(await screen.findByText("Protected admin")).toBeTruthy();
  expect(login).toHaveBeenCalledWith("owner", "test-only-password");
  expect(queryClient.getQueryData(["secret"])).toBeUndefined();
  await user.type(screen.getByLabelText("Secret draft"), "private draft");
  queryClient.setQueryData(["settings"], { token: "synthetic-test-value" });
  await user.click(screen.getByText("owner"));
  await user.click(screen.getByRole("button", { name: "Sign out" }));
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  expect(queryClient.getQueryData(["settings"])).toBeUndefined();
  expect(screen.queryByDisplayValue("private draft")).toBeNull();
});

it("clears protected caches and drafts when a background status poll revokes the session", async () => {
  const status = vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  const queryClient = mountGate();
  expect(await screen.findByText("Protected admin")).toBeTruthy();
  await userEvent
    .setup()
    .type(screen.getByLabelText("Secret draft"), "private draft");
  queryClient.setQueryData(["settings"], { token: "synthetic-test-value" });
  queryClient
    .getMutationCache()
    .build(queryClient, { mutationKey: ["settings-update"] });
  status.mockResolvedValue(signedOut);
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: authKey });
  });
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  await waitFor(() => {
    expect(queryClient.getQueryData(["settings"])).toBeUndefined();
  });
  expect(queryClient.getMutationCache().getAll()).toHaveLength(0);
  expect(screen.queryByDisplayValue("private draft")).toBeNull();
});

it("remounts the app and clears privileged caches when the session role changes", async () => {
  const status = vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  const queryClient = mountGate();
  expect(await screen.findByText("Protected admin")).toBeTruthy();
  await userEvent
    .setup()
    .type(screen.getByLabelText("Secret draft"), "private draft");
  queryClient.setQueryData(["settings"], { token: "synthetic-test-value" });
  status.mockResolvedValue({
    ...administrator,
    user: { username: "owner", role: "viewer" },
  });
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: authKey });
  });
  expect(await screen.findByText("Protected viewer")).toBeTruthy();
  expect(screen.getByText("Cannot administer")).toBeTruthy();
  expect(screen.getByText("Cannot operate")).toBeTruthy();
  expect(screen.queryByDisplayValue("private draft")).toBeNull();
  expect(queryClient.getQueryData(["settings"])).toBeUndefined();
});

it("handles unauthorized API responses without retaining cached data", async () => {
  vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  const queryClient = mountGate();
  expect(await screen.findByText("Protected admin")).toBeTruthy();
  queryClient.setQueryData(["settings"], { token: "synthetic-test-value" });
  act(() => {
    authExpired();
  });
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  expect(queryClient.getQueryData(["settings"])).toBeUndefined();
});

it("shows failed login without mounting the app or retaining the password", async () => {
  vi.spyOn(authApi, "status").mockResolvedValue(signedOut);
  vi.spyOn(authApi, "login").mockRejectedValue(
    new Error("Invalid account name or password"),
  );
  mountGate();
  const user = userEvent.setup();
  await user.type(await screen.findByLabelText("Account name"), "owner");
  await user.type(screen.getByLabelText("Password"), "test-only-password");
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Invalid account name or password",
  );
  expect(screen.getByLabelText("Password")).toHaveProperty("value", "");
  expect(screen.queryByText(/Protected/)).toBeNull();
});

it("does not fetch accounts for viewers or local unauthenticated mode", () => {
  const accounts = vi.spyOn(authApi, "accounts");
  const queryClient = client();
  render(
    <QueryClientProvider client={queryClient}>
      <AuthContext.Provider
        value={{
          ...administrator,
          user: { username: "reader", role: "viewer" },
        }}
      >
        <AccountPanel />
      </AuthContext.Provider>
    </QueryClientProvider>,
  );
  expect(accounts).not.toHaveBeenCalled();
  expect(screen.queryByText("Panel accounts")).toBeNull();
});

it("creates an account using the current revision and explicit role", async () => {
  vi.spyOn(authApi, "accounts").mockResolvedValue({
    revision: "fixture-revision",
    accounts: [{ username: "owner", role: "admin" }],
  });
  const create = vi.spyOn(authApi, "create").mockResolvedValue({
    revision: "fixture-next",
    accounts: [
      { username: "owner", role: "admin" },
      { username: "reader", role: "viewer" },
    ],
  });
  render(
    <QueryClientProvider client={client()}>
      <AuthContext.Provider value={administrator}>
        <AccountPanel />
      </AuthContext.Provider>
    </QueryClientProvider>,
  );
  const user = userEvent.setup();
  await user.type(await screen.findByLabelText("Account name"), "reader");
  await user.type(screen.getByLabelText("Password"), "test-only-password");
  await user.click(screen.getByRole("button", { name: "Create account" }));
  await waitFor(() => {
    expect(create).toHaveBeenCalledWith({
      revision: "fixture-revision",
      username: "reader",
      password: "test-only-password",
      role: "viewer",
    });
  });
  expect(await screen.findByText("reader")).toBeTruthy();
  expect(screen.getByLabelText("Password")).toHaveProperty("value", "");
});

it("does not offer self deletion while editing the current account", async () => {
  vi.spyOn(authApi, "accounts").mockResolvedValue({
    revision: "fixture-revision",
    accounts: [{ username: "owner", role: "admin" }],
  });
  render(
    <QueryClientProvider client={client()}>
      <AuthContext.Provider value={administrator}>
        <AccountPanel />
      </AuthContext.Provider>
    </QueryClientProvider>,
  );
  await userEvent
    .setup()
    .click(await screen.findByRole("button", { name: "Edit" }));
  expect(await screen.findByText("Edit owner")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Delete account" })).toBeNull();
});
