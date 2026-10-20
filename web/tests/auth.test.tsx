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
  user: null,
  expires_at: null,
};
const administrator: SessionStatus = {
  user: {
    username: "owner",
    account_id: "fixture-owner",
    role: "admin",
    must_change_password: false,
  },
  expires_at: 9999999999,
};
function client(session?: SessionStatus) {
  const result = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  if (session) result.setQueryData(authKey, session);
  return result;
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

it("suspends cached data and drafts when a background status poll expires the session", async () => {
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
    expect(queryClient.getQueryData(["settings"])).toEqual({
      token: "synthetic-test-value",
    });
  });
  expect(queryClient.getMutationCache().getAll()).toHaveLength(0);
  expect(screen.queryByRole("textbox", { name: "Secret draft" })).toBeNull();
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
    user: {
      username: "owner",
      account_id: "fixture-owner",
      role: "viewer",
      must_change_password: false,
    },
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

it("hides cached protected data after unauthorized API responses", async () => {
  vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  const queryClient = mountGate();
  expect(await screen.findByText("Protected admin")).toBeTruthy();
  queryClient.setQueryData(["settings"], { token: "synthetic-test-value" });
  act(() => {
    authExpired();
  });
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  expect(queryClient.getQueryData(["settings"])).toEqual({
    token: "synthetic-test-value",
  });
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
          user: {
            username: "reader",
            account_id: "fixture-reader",
            role: "viewer",
            must_change_password: false,
          },
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
    accounts: [
      {
        username: "owner",
        account_id: "fixture-owner",
        role: "admin",
        must_change_password: false,
      },
    ],
  });
  const create = vi.spyOn(authApi, "create").mockResolvedValue({
    revision: "fixture-next",
    accounts: [
      {
        username: "owner",
        account_id: "fixture-owner",
        role: "admin",
        must_change_password: false,
      },
      {
        username: "reader",
        account_id: "fixture-reader",
        role: "viewer",
        must_change_password: false,
      },
    ],
  });
  render(
    <QueryClientProvider client={client(administrator)}>
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
    accounts: [
      {
        username: "owner",
        account_id: "fixture-owner",
        role: "admin",
        must_change_password: false,
      },
    ],
  });
  render(
    <QueryClientProvider client={client(administrator)}>
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

it("requires a temporary password change without mounting protected pages", async () => {
  vi.spyOn(authApi, "status").mockResolvedValue({
    ...administrator,
    user: {
      username: "admin",
      account_id: "fixture-admin",
      role: "admin",
      must_change_password: true,
    },
  });
  const change = vi.spyOn(authApi, "password").mockResolvedValue(undefined);
  const queryClient = mountGate();
  const user = userEvent.setup();
  expect(
    await screen.findByRole("heading", { name: "Change temporary password" }),
  ).toBeTruthy();
  expect(screen.queryByText(/Protected/)).toBeNull();
  await user.type(
    screen.getByLabelText("Current password"),
    "synthetic-temporary-password",
  );
  await user.type(
    screen.getByLabelText("New password"),
    "synthetic-permanent-password",
  );
  await user.click(screen.getByRole("button", { name: "Change password" }));
  expect(change).toHaveBeenCalledWith(
    "synthetic-temporary-password",
    "synthetic-permanent-password",
  );
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  expect(queryClient.getQueryData(authKey)).toEqual(signedOut);
  expect(screen.queryByDisplayValue("synthetic-temporary-password")).toBeNull();
  expect(screen.queryByText(/Protected/)).toBeNull();
});

it("grants no permissions without an authenticated context", () => {
  render(
    <QueryClientProvider client={client()}>
      <Protected />
    </QueryClientProvider>,
  );
  expect(screen.getByText("Cannot administer")).toBeTruthy();
  expect(screen.getByText("Cannot operate")).toBeTruthy();
});

it("keeps the mounted draft through a failed background session request", async () => {
  const status = vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  const queryClient = mountGate();
  const draft = await screen.findByRole("textbox", { name: "Secret draft" });
  await userEvent.setup().type(draft, "unsaved work");
  status.mockRejectedValue(new Error("Network interrupted"));
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: authKey });
  });
  const alert = await screen.findByRole("alert");
  expect(screen.getByRole("textbox", { name: "Secret draft" })).toBe(draft);
  expect(draft).toHaveProperty("value", "unsaved work");
  expect(alert).toHaveProperty(
    "textContent",
    "Connection interrupted. Your edits are still here.",
  );
});

it("hides drafts during expiry and resumes them only after the same identity signs in", async () => {
  const status = vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  vi.spyOn(authApi, "login").mockResolvedValue(administrator);
  const queryClient = mountGate();
  const user = userEvent.setup();
  const draft = await screen.findByRole("textbox", { name: "Secret draft" });
  await user.type(draft, "unsaved work");
  status.mockResolvedValue(signedOut);
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: authKey });
  });
  await screen.findByRole("button", { name: "Sign in" });
  expect(screen.queryByRole("textbox", { name: "Secret draft" })).toBeNull();
  await user.type(screen.getByLabelText("Account name"), "owner");
  await user.type(screen.getByLabelText("Password"), "synthetic-password");
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  expect(await screen.findByRole("textbox", { name: "Secret draft" })).toBe(
    draft,
  );
  expect(draft).toHaveProperty("value", "unsaved work");
});

it("destroys suspended drafts when a different user signs in", async () => {
  const status = vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  vi.spyOn(authApi, "login").mockResolvedValue({
    ...administrator,
    user: {
      username: "another",
      account_id: "fixture-another",
      role: "admin",
      must_change_password: false,
    },
  });
  const queryClient = mountGate();
  const user = userEvent.setup();
  await user.type(
    await screen.findByRole("textbox", { name: "Secret draft" }),
    "private work",
  );
  status.mockResolvedValue(signedOut);
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: authKey });
  });
  await user.type(await screen.findByLabelText("Account name"), "another");
  await user.type(screen.getByLabelText("Password"), "synthetic-password");
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  expect(
    await screen.findByRole("textbox", { name: "Secret draft" }),
  ).toHaveProperty("value", "");
  expect(screen.queryByDisplayValue("private work")).toBeNull();
});

it("does not restore a draft to a recreated account with the same name and role", async () => {
  const status = vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  vi.spyOn(authApi, "login").mockResolvedValue({
    ...administrator,
    user: {
      username: "owner",
      account_id: "new-account-incarnation",
      role: "admin",
      must_change_password: false,
    },
  });
  const queryClient = mountGate();
  const user = userEvent.setup();
  await user.type(
    await screen.findByRole("textbox", { name: "Secret draft" }),
    "former account's draft",
  );
  status.mockResolvedValue(signedOut);
  await act(async () => {
    await queryClient.refetchQueries({ queryKey: authKey });
  });
  await user.type(await screen.findByLabelText("Account name"), "owner");
  await user.type(screen.getByLabelText("Password"), "synthetic-password");
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  expect(
    await screen.findByRole("textbox", { name: "Secret draft" }),
  ).toHaveProperty("value", "");
  expect(screen.queryByDisplayValue("former account's draft")).toBeNull();
});

it("initializes authentication under StrictMode effect replay", async () => {
  const { StrictMode } = await import("react");
  vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  render(
    <StrictMode>
      <QueryClientProvider client={client()}>
        <AuthGate>
          <Protected />
        </AuthGate>
      </QueryClientProvider>
    </StrictMode>,
  );
  expect(await screen.findByText("Protected admin")).toBeTruthy();
});

it("locks cached content at its session deadline even without another successful poll", async () => {
  vi.spyOn(authApi, "status").mockResolvedValue({
    ...administrator,
    expires_at: 1,
  });
  const queryClient = mountGate();
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeTruthy();
  expect(queryClient.getQueryData(authKey)).toEqual(signedOut);
  expect(screen.queryByRole("textbox", { name: "Secret draft" })).toBeNull();
});

it("does not let delayed sign-out cleanup clear a newly authenticated identity", async () => {
  vi.spyOn(authApi, "status").mockResolvedValue(administrator);
  vi.spyOn(authApi, "logout").mockResolvedValue(undefined);
  vi.spyOn(authApi, "login").mockResolvedValue({
    ...administrator,
    user: {
      username: "another",
      account_id: "another-account",
      role: "admin",
      must_change_password: false,
    },
  });
  const queryClient = mountGate();
  await screen.findByText("Protected admin");
  let release: (() => void) | undefined;
  const originalCancel = queryClient.cancelQueries.bind(queryClient);
  vi.spyOn(queryClient, "cancelQueries").mockImplementation(
    (filters, options) =>
      filters === undefined
        ? new Promise<void>((resolve) => {
            release = resolve;
          })
        : originalCancel(filters, options),
  );
  const user = userEvent.setup();
  await user.click(screen.getByText("owner"));
  await user.click(screen.getByRole("button", { name: "Sign out" }));
  await user.type(await screen.findByLabelText("Account name"), "another");
  await user.type(screen.getByLabelText("Password"), "synthetic-password");
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  await screen.findByText("another");
  queryClient.setQueryData(["new-account-cache"], "new-account-data");
  await act(async () => {
    release?.();
    await Promise.resolve();
  });
  expect(queryClient.getQueryData<SessionStatus>(authKey)?.user?.username).toBe(
    "another",
  );
  expect(queryClient.getQueryData(["new-account-cache"])).toBe(
    "new-account-data",
  );
  expect(screen.queryByRole("button", { name: "Sign in" })).toBeNull();
});
