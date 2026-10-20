import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "../src/App";
import { authKey } from "../src/authApi";
import type { Account } from "../src/authApi";

const owner: Account = {
  username: "owner",
  account_id: "original-owner",
  role: "admin",
  must_change_password: false,
};
function fixture() {
  let authenticated = true;
  let loginAccount = owner;
  let protectedReads = 0;
  let expiresAt = Math.floor(Date.now() / 1000) + 28800;
  const settings = {
    revision: "settings-one",
    settings: {
      name: "Original server",
      description: "",
      max_players: 12,
      tags: [],
      username: "",
      password: "",
      token: "",
      game_password: "",
      require_user_verification: true,
      max_upload_in_kilobytes_per_second: 0,
      max_upload_slots: 5,
      minimum_latency_in_ticks: 0,
      max_heartbeats_per_second: 60,
      ignore_player_limit_for_returning_players: false,
      allow_commands: "admins-only",
      autosave_slots: 5,
      afk_autokick_interval: 0,
      auto_pause_when_players_connect: false,
      only_admins_can_pause_the_game: true,
      autosave_only_on_server: true,
      non_blocking_saving: false,
      minimum_segment_size: 25,
      minimum_segment_size_peer_count: 20,
      maximum_segment_size: 100,
      maximum_segment_size_peer_count: 10,
      auto_pause: true,
      autosave_interval: 5,
      visibility: { public: false, lan: true },
    },
  };
  const session = () => ({
    user: authenticated ? loginAccount : null,
    expires_at: authenticated ? expiresAt : null,
  });
  const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "Content-Type": "application/json" },
    });
  vi.stubGlobal(
    "fetch",
    vi.fn((input: string) => {
      if (input === "/api/auth/status") return Promise.resolve(json(session()));
      if (input === "/api/auth/login") {
        authenticated = true;
        expiresAt = Math.floor(Date.now() / 1000) + 28800;
        return Promise.resolve(json(session()));
      }
      protectedReads++;
      if (!authenticated)
        return Promise.resolve(json({ message: "Session expired" }, 401));
      switch (input) {
        case "/api/status":
          return Promise.resolve(
            json({
              state: "stopped",
              since: 1,
              active_save: null,
              started_at: null,
              binary_available: true,
              version: "2.1.23",
              error: null,
            }),
          );
        case "/api/settings":
        case "/api/settings/raw":
          return Promise.resolve(json(settings));
        case "/api/manager-settings":
          return Promise.resolve(
            json({
              revision: "manager-one",
              settings: {
                poll_interval_seconds: 2,
                ups_history_minutes: 10,
                enable_script_telemetry: false,
                game_bind_address: "0.0.0.0",
                game_port: 34197,
                use_server_whitelist: false,
              },
            }),
          );
        case "/api/admins":
          return Promise.resolve(
            json({ revision: "admins-one", admins: ["owner"] }),
          );
        case "/api/auth/accounts":
          return Promise.resolve(
            json({ revision: "accounts-one", accounts: [loginAccount] }),
          );
        case "/api/mods":
          return Promise.resolve(
            json({
              revision: "mods-one",
              portal_authenticated: false,
              mods: [],
            }),
          );
        case "/api/mods/settings":
          return Promise.resolve(
            json({
              revision: "startup-one",
              available: true,
              settings: [
                {
                  name: "fixture-startup",
                  kind: "integer",
                  value: "1",
                  components: [],
                },
              ],
            }),
          );
        default:
          return Promise.resolve(
            json({ message: "Unknown fixture route" }, 404),
          );
      }
    }),
  );
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 20 },
      mutations: { retry: false },
    },
  });
  window.history.replaceState(null, "", "#settings");
  render(
    <QueryClientProvider client={client}>
      <App />
    </QueryClientProvider>,
  );
  return {
    client,
    expire: () => {
      authenticated = false;
    },
    loginAs: (account: Account) => {
      loginAccount = account;
    },
    reads: () => protectedReads,
  };
}
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  window.history.replaceState(null, "", "#overview");
});

it.each([0, 6 * 60 * 1000])(
  "preserves the actual settings editor through a status 401, %i ms suspension and same-account login",
  async (suspension) => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { client, expire, reads } = fixture();
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const name = await screen.findByRole("textbox", { name: "Server name" });
    await user.clear(name);
    await user.type(name, "Private unsaved server name");
    expire();
    await act(async () => {
      await client.refetchQueries({ queryKey: ["status"] });
    });
    await screen.findByRole("button", { name: "Sign in" });
    expect(screen.queryByRole("textbox", { name: "Server name" })).toBeNull();
    const lockedReads = reads();
    client.setQueryData(
      ["mod-portal-search", "temporary"],
      ["temporary result"],
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(suspension);
    });
    expect(reads()).toBe(lockedReads);
    if (suspension > 0)
      expect(
        client.getQueryData(["mod-portal-search", "temporary"]),
      ).toBeUndefined();
    await user.type(
      screen.getByRole("textbox", { name: "Account name" }),
      "owner",
    );
    await user.type(
      within(screen.getByRole("main")).getByLabelText("Password"),
      "synthetic-password",
    );
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    const resumed = await screen.findByRole("textbox", { name: "Server name" });
    expect(resumed).toBe(name);
    expect(resumed).toHaveProperty("value", "Private unsaved server name");
    expect(client.getQueryData(authKey)).toBeDefined();
  },
);

it("discards the actual settings editor before another account can see it", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  const { client, expire, loginAs } = fixture();
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  await user.clear(await screen.findByRole("textbox", { name: "Server name" }));
  await user.type(
    screen.getByRole("textbox", { name: "Server name" }),
    "Previous owner's draft",
  );
  expire();
  await act(async () => {
    await client.refetchQueries({ queryKey: ["status"] });
  });
  await screen.findByRole("button", { name: "Sign in" });
  loginAs({ ...owner, username: "another", account_id: "another-account" });
  await user.type(
    screen.getByRole("textbox", { name: "Account name" }),
    "another",
  );
  await user.type(
    within(screen.getByRole("main")).getByLabelText("Password"),
    "synthetic-password",
  );
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  expect(
    await screen.findByRole("textbox", { name: "Server name" }),
  ).toHaveProperty("value", "Original server");
  expect(screen.queryByDisplayValue("Previous owner's draft")).toBeNull();
});

it("keeps server, administrator, manager and startup drafts together through a long session suspension", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  const { client, expire } = fixture();
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  const serverName = await screen.findByRole("textbox", {
    name: "Server name",
  });
  await user.clear(serverName);
  await user.type(serverName, "Draft server name");
  await user.click(screen.getByRole("button", { name: "Administrators" }));
  await user.type(
    screen.getByRole("textbox", { name: "Factorio username" }),
    "DraftAdministrator",
  );
  await user.click(screen.getByRole("button", { name: "Add player" }));
  await user.click(screen.getByRole("button", { name: "Manager settings" }));
  const refresh = screen.getByRole("spinbutton", {
    name: /^Refresh interval/u,
  });
  await user.clear(refresh);
  await user.type(refresh, "13");
  await user.click(screen.getByRole("link", { name: "Mods" }));
  await user.click(screen.getByRole("button", { name: "Startup settings" }));
  const startup = await screen.findByRole("spinbutton", {
    name: "fixture-startup",
  });
  await user.clear(startup);
  await user.type(startup, "9");
  expire();
  await act(async () => {
    await client.refetchQueries({ queryKey: ["status"] });
  });
  await screen.findByRole("button", { name: "Sign in" });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(6 * 60 * 1000);
  });
  await user.type(
    screen.getByRole("textbox", { name: "Account name" }),
    "owner",
  );
  await user.type(
    within(screen.getByRole("main")).getByLabelText("Password"),
    "synthetic-password",
  );
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  expect(
    await screen.findByRole("spinbutton", { name: "fixture-startup" }),
  ).toBe(startup);
  expect(startup).toHaveProperty("value", "9");
  await user.click(screen.getByRole("link", { name: "Settings" }));
  expect(screen.getByRole("spinbutton", { name: /^Refresh interval/u })).toBe(
    refresh,
  );
  expect(refresh).toHaveProperty("value", "13");
  await user.click(screen.getByRole("button", { name: "Administrators" }));
  expect(
    screen.getByRole("button", { name: "Remove DraftAdministrator" }),
  ).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Server settings" }));
  expect(screen.getByRole("textbox", { name: "Server name" })).toBe(serverName);
  expect(serverName).toHaveProperty("value", "Draft server name");
});
