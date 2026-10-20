import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { authApi } from "../src/authApi";
import { api, ApiError, settingsSchema } from "../src/api";
import type { SettingsDocument, RawSettingsDocument, Status } from "../src/api";
import SettingsPage from "../src/pages/Settings";
import Administrators from "../src/components/Administrators";
import ServerControls from "../src/components/ServerControls";
import App from "../src/App";
import Saves from "../src/pages/Saves";

const settings: SettingsDocument = {
  revision: "one",
  settings: {
    name: "Nauvis",
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
const stopped: Status = {
  state: "stopped",
  since: 1,
  active_save: null,
  started_at: null,
  binary_available: true,
  version: null,
  error: null,
};
function mount(element: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>{element}</QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.spyOn(authApi, "status").mockResolvedValue({
    enabled: false,
    user: null,
    expires_at: null,
  });
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.setAttribute("open", "");
    },
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.removeAttribute("open");
    },
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("settings and administrators", () => {
  it("locks fields until a settings write completes so later typing cannot be lost", async () => {
    vi.spyOn(api, "settings").mockResolvedValue(structuredClone(settings));
    vi.spyOn(api, "rawSettings").mockResolvedValue(structuredClone(settings));
    vi.spyOn(api, "manager").mockResolvedValue({
      revision: "one",
      settings: {
        poll_interval_seconds: 2,
        ups_history_minutes: 10,
        enable_script_telemetry: false,
        game_bind_address: "0.0.0.0",
        game_port: 34197,
      },
    });
    let finish: ((document: RawSettingsDocument) => void) | undefined;
    vi.spyOn(api, "saveRawSettings").mockImplementation(
      () =>
        new Promise<RawSettingsDocument>((resolve) => {
          finish = resolve;
        }),
    );
    mount(<SettingsPage />);
    const user = userEvent.setup();
    const name = await screen.findByRole("textbox", { name: /Server name/ });
    await user.clear(name);
    await user.type(name, "Saved factory");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    expect(name.hasAttribute("disabled")).toBe(true);
    if (!finish) throw new Error("Expected a pending save");
    finish({
      revision: "two",
      settings: { ...settings.settings, name: "Saved factory" },
    });
    await screen.findByText(
      "Settings saved. Restart the server to apply them.",
    );
    expect(name.hasAttribute("disabled")).toBe(false);
  });

  it("keeps edits after a stale-write conflict and does not report success", async () => {
    vi.spyOn(api, "settings").mockResolvedValue(structuredClone(settings));
    vi.spyOn(api, "rawSettings").mockResolvedValue(structuredClone(settings));
    vi.spyOn(api, "manager").mockResolvedValue({
      revision: "one",
      settings: {
        poll_interval_seconds: 2,
        ups_history_minutes: 10,
        enable_script_telemetry: false,
        game_bind_address: "0.0.0.0",
        game_port: 34197,
      },
    });
    vi.spyOn(api, "saveRawSettings").mockRejectedValue(
      new ApiError("Settings changed on disk. Reload before saving", 409),
    );
    mount(<SettingsPage />);
    const user = userEvent.setup();
    const name = await screen.findByRole("textbox", { name: /Server name/ });
    await user.clear(name);
    await user.type(name, "My unsaved factory");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Settings changed on disk",
    );
    expect(
      screen.queryByText("Settings saved. Restart the server to apply them."),
    ).toBeNull();
    expect(name.getAttribute("value")).toBe("My unsaved factory");
  });
  it("persists individual admin entries and reads them after remount", async () => {
    let stored = { revision: "one", admins: ["Szer"] };
    vi.spyOn(api, "admins").mockImplementation(() =>
      Promise.resolve(structuredClone(stored)),
    );
    vi.spyOn(api, "saveAdmins").mockImplementation((document) => {
      stored = { revision: "two", admins: [...document.admins] };
      return Promise.resolve(structuredClone(stored));
    });
    const first = mount(<Administrators />);
    const user = userEvent.setup();
    await user.type(
      await screen.findByRole("textbox", { name: "Factorio username" }),
      "paragith",
    );
    await user.click(screen.getByRole("button", { name: "Add player" }));
    await user.click(
      screen.getByRole("button", { name: "Save administrators" }),
    );
    await screen.findByText(
      "Administrators saved. Restart the server to apply the list.",
    );
    expect(stored.admins).toEqual(["Szer", "paragith"]);
    first.unmount();
    mount(<Administrators />);
    await screen.findByText("paragith");
    expect(screen.getByText("Szer")).toBeDefined();
  });
  it("preserves unsaved settings while navigating away and back", async () => {
    window.location.hash = "settings";
    vi.spyOn(api, "status").mockResolvedValue(stopped);
    vi.spyOn(api, "settings").mockResolvedValue(structuredClone(settings));
    vi.spyOn(api, "rawSettings").mockResolvedValue(structuredClone(settings));
    vi.spyOn(api, "manager").mockResolvedValue({
      revision: "one",
      settings: {
        poll_interval_seconds: 2,
        ups_history_minutes: 10,
        enable_script_telemetry: false,
        game_bind_address: "0.0.0.0",
        game_port: 34197,
      },
    });
    vi.spyOn(api, "admins").mockResolvedValue({ revision: "one", admins: [] });
    vi.spyOn(api, "saves").mockResolvedValue([]);
    vi.spyOn(api, "logs").mockResolvedValue([]);
    mount(<App />);
    const user = userEvent.setup();
    const name = await screen.findByRole("textbox", { name: /Server name/ });
    await user.clear(name);
    await user.type(name, "Still here");
    await user.click(screen.getByRole("link", { name: "Saves" }));
    await screen.findByRole("heading", { name: "Saves", level: 1 });
    await user.click(screen.getByRole("link", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings", level: 1 });
    expect(
      screen
        .getByRole("textbox", { name: /Server name/ })
        .getAttribute("value"),
    ).toBe("Still here");
  });
});

it("starts the persisted selected save rather than the newest upload", async () => {
  vi.spyOn(api, "selection").mockResolvedValue({
    load_latest: false,
    revision: "one",
    save: "beta.zip",
  });
  const start = vi.spyOn(api, "start").mockResolvedValue({
    ...stopped,
    state: "starting",
    active_save: "beta.zip",
  });
  mount(<ServerControls status={stopped} />);
  const user = userEvent.setup();
  await waitFor(() => {
    expect(
      screen
        .getByRole("button", { name: "Start server" })
        .hasAttribute("disabled"),
    ).toBe(false);
  });
  await user.click(screen.getByRole("button", { name: "Start server" }));
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByText("beta.zip")).toBeDefined();
  await user.click(
    within(dialog).getByRole("button", { name: "Start server" }),
  );
  await waitFor(() => {
    expect(start).toHaveBeenCalledWith("beta.zip");
  });
});

it("rejects HTML masquerading as a successful API response", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response("<html>Login</html>", {
        status: 200,
        headers: { "Content-Type": "text/html" },
      }),
    ),
  );
  await expect(api.settings()).rejects.toThrow("unexpected response");
});

it("accepts disabled autosaves and rejects invalid numeric settings", () => {
  expect(
    settingsSchema.safeParse({ ...settings.settings, autosave_interval: 0 })
      .success,
  ).toBe(true);
  expect(
    settingsSchema.safeParse({ ...settings.settings, max_players: Number.NaN })
      .success,
  ).toBe(false);
  expect(
    settingsSchema.safeParse({ ...settings.settings, max_players: -1 }).success,
  ).toBe(false);
});

it("preserves unknown configuration keys across form and raw editing", async () => {
  vi.spyOn(api, "rawSettings").mockResolvedValue({
    revision: "one",
    settings: { ...settings.settings, mod_specific: { precision: 12 } },
  });
  const save = vi
    .spyOn(api, "saveRawSettings")
    .mockImplementation((document) =>
      Promise.resolve({ ...document, revision: "two" }),
    );
  mount(<SettingsPage />);
  const user = userEvent.setup();
  const name = await screen.findByRole("textbox", { name: "Server name" });
  await user.clear(name);
  await user.type(name, "Edited factory");
  await user.click(screen.getByRole("button", { name: "Raw JSON" }));
  const raw = screen.getByRole("textbox", { name: "Raw server settings" });
  expect((raw as HTMLTextAreaElement).value).toContain('"mod_specific"');
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => {
    expect(save.mock.calls[0]?.[0].settings.name).toBe("Edited factory");
    expect(save.mock.calls[0]?.[0].settings.mod_specific).toEqual({
      precision: 12,
    });
  });
});

it("keeps invalid raw JSON editable and does not submit it", async () => {
  vi.spyOn(api, "rawSettings").mockResolvedValue(structuredClone(settings));
  const save = vi.spyOn(api, "saveRawSettings");
  mount(<SettingsPage />);
  const user = userEvent.setup();
  await screen.findByRole("textbox", { name: "Server name" });
  await user.click(screen.getByRole("button", { name: "Raw JSON" }));
  const raw = screen.getByRole("textbox", { name: "Raw server settings" });
  await user.clear(raw);
  await user.type(raw, "broken JSON");
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  expect((raw as HTMLTextAreaElement).value).toBe("broken JSON");
  expect(
    screen.getByText("Enter a valid JSON object. Your draft has been kept."),
  ).toBeDefined();
  expect(save).not.toHaveBeenCalled();
});

it("shows Stop for a paused server", () => {
  vi.spyOn(api, "selection").mockResolvedValue({
    load_latest: false,
    revision: "one",
    save: "test.zip",
  });
  mount(<ServerControls status={{ ...stopped, state: "paused" }} />);
  expect(screen.getByRole("button", { name: "Stop server" })).toBeDefined();
  expect(screen.queryByRole("button", { name: "Start server" })).toBeNull();
});

it("does not enable Start without an explicitly selected save", async () => {
  vi.spyOn(api, "selection").mockResolvedValue({
    load_latest: false,
    revision: "one",
    save: null,
  });
  mount(<ServerControls status={stopped} />);
  await waitFor(() => {
    expect(
      screen
        .getByRole("button", { name: "Start server" })
        .hasAttribute("disabled"),
    ).toBe(true);
  });
});

it("shares administrator edits between the form and its separate raw file", async () => {
  vi.spyOn(api, "admins").mockResolvedValue({
    revision: "admin-one",
    admins: ["Szer"],
  });
  const save = vi
    .spyOn(api, "saveAdmins")
    .mockImplementation((document) =>
      Promise.resolve({ ...document, revision: "admin-two" }),
    );
  mount(<Administrators />);
  const user = userEvent.setup();
  await user.type(
    await screen.findByRole("textbox", { name: "Factorio username" }),
    "paragith",
  );
  await user.click(screen.getByRole("button", { name: "Add player" }));
  await user.click(screen.getByRole("button", { name: "Raw JSON" }));
  const raw = screen.getByRole<HTMLTextAreaElement>("textbox", {
    name: "Raw administrator list",
  });
  expect(JSON.parse(raw.value)).toEqual(["Szer", "paragith"]);
  await user.clear(raw);
  await user.paste('["Szer", "Explorer"]');
  await user.click(screen.getByRole("button", { name: "Form" }));
  expect(screen.getByText("Explorer")).toBeDefined();
  expect(screen.queryByText("paragith")).toBeNull();
  await user.click(screen.getByRole("button", { name: "Save administrators" }));
  await waitFor(() => {
    expect(save).toHaveBeenCalledWith(
      { revision: "admin-one", admins: ["Szer", "Explorer"] },
      expect.anything(),
    );
  });
});

it("keeps invalid raw administrator JSON without submitting or losing it", async () => {
  vi.spyOn(api, "admins").mockResolvedValue({
    revision: "one",
    admins: ["Szer"],
  });
  const save = vi.spyOn(api, "saveAdmins");
  mount(<Administrators />);
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Raw JSON" }));
  const raw = screen.getByRole<HTMLTextAreaElement>("textbox", {
    name: "Raw administrator list",
  });
  await user.clear(raw);
  await user.paste('{"admins":["Szer"]}');
  await user.click(screen.getByRole("button", { name: "Save administrators" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "JSON array",
  );
  expect(save).not.toHaveBeenCalled();
  expect(raw.value).toBe('{"admins":["Szer"]}');
  expect(
    screen.getByRole("button", { name: "Form" }).hasAttribute("disabled"),
  ).toBe(true);
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(JSON.parse(raw.value)).toEqual(["Szer"]);
});

it("edits manager JSON separately, validates unknown keys, and preserves form changes", async () => {
  vi.spyOn(api, "rawSettings").mockResolvedValue(structuredClone(settings));
  vi.spyOn(api, "admins").mockResolvedValue({ revision: "one", admins: [] });
  const preferences = {
    poll_interval_seconds: 2,
    ups_history_minutes: 10,
    enable_script_telemetry: false,
    game_bind_address: "0.0.0.0",
    game_port: 34197,
  };
  vi.spyOn(api, "manager").mockResolvedValue({
    revision: "manager-one",
    settings: preferences,
  });
  const save = vi
    .spyOn(api, "saveManager")
    .mockImplementation((document) =>
      Promise.resolve({ ...document, revision: "manager-two" }),
    );
  mount(<SettingsPage />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Manager settings" }));
  const interval = await screen.findByRole<HTMLInputElement>("spinbutton", {
    name: /Refresh interval/,
  });
  await user.clear(interval);
  expect(interval.value).toBe("");
  await user.type(interval, "5");
  await user.click(screen.getByRole("button", { name: "Raw JSON" }));
  const raw = screen.getByRole<HTMLTextAreaElement>("textbox", {
    name: "Raw manager settings",
  });
  expect(JSON.parse(raw.value)).toEqual({
    ...preferences,
    poll_interval_seconds: 5,
  });
  await user.clear(raw);
  await user.paste(JSON.stringify({ ...preferences, extra: true }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Unknown keys",
  );
  expect(save).not.toHaveBeenCalled();
  await user.clear(raw);
  await user.paste(JSON.stringify({ ...preferences, ups_history_minutes: 30 }));
  await user.click(screen.getByRole("button", { name: "Form" }));
  expect(
    screen.getByRole<HTMLInputElement>("spinbutton", { name: /UPS history/ })
      .value,
  ).toBe("30");
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => {
    expect(save).toHaveBeenCalledWith(
      {
        revision: "manager-one",
        settings: { ...preferences, ups_history_minutes: 30 },
      },
      expect.anything(),
    );
  });
});

it("accepts a 200 MiB save and previews mod sync before applying it", async () => {
  const file = new File(["zip"], "large.zip", { type: "application/zip" });
  Object.defineProperty(file, "size", { value: 200 * 1024 * 1024 });
  const save = {
    name: "large.zip",
    size_bytes: file.size,
    modified_at: 1,
    revision: "save-one",
  };
  vi.spyOn(api, "saves").mockResolvedValue([save]);
  vi.spyOn(api, "selection").mockResolvedValue({
    load_latest: false,
    revision: "one",
    save: null,
  });
  const upload = vi.spyOn(api, "upload").mockResolvedValue(save);
  vi.spyOn(api, "saveModPlan").mockResolvedValue({
    revision: "plan-one",
    save: "large.zip",
    factorio_version: "2.1.23",
    engine_version: "2.1.23",
    mods: [{ name: "demo", version: "1.0.0", action: "download" }],
    disable: ["extra"],
    blockers: [],
  });
  const sync = vi.spyOn(api, "syncSaveMods").mockResolvedValue({
    revision: "two",
    mods: [],
    portal_authenticated: true,
  });
  mount(<Saves status={stopped} />);
  const user = userEvent.setup();
  await user.upload(screen.getByLabelText("Upload Factorio save"), file);
  await waitFor(() => {
    expect(upload).toHaveBeenCalledWith(file, expect.anything());
  });
  await user.click(await screen.findByRole("button", { name: "Sync mods" }));
  await screen.findByRole("heading", { name: "Sync mods with large.zip" });
  expect(screen.getByText("Download exact version")).toBeDefined();
  expect(screen.getByText(/Will disable:/)).toBeDefined();
  expect(sync).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Apply mod sync" }));
  await waitFor(() => {
    expect(sync).toHaveBeenCalledWith(
      expect.objectContaining({ name: "large.zip", revision: "plan-one" }),
      expect.anything(),
    );
  });
});

it("does not allow applying a sync plan with missing credentials", async () => {
  vi.spyOn(api, "saves").mockResolvedValue([
    { name: "world.zip", size_bytes: 1, modified_at: 1, revision: "save-one" },
  ]);
  vi.spyOn(api, "selection").mockResolvedValue({
    load_latest: false,
    revision: "one",
    save: null,
  });
  vi.spyOn(api, "saveModPlan").mockResolvedValue({
    revision: "plan-one",
    save: "world.zip",
    factorio_version: "2.1.23",
    engine_version: "2.1.23",
    mods: [],
    disable: [],
    blockers: ["Configure Factorio username/token"],
  });
  mount(<Saves status={stopped} />);
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Sync mods" }));
  await screen.findByText("Configure Factorio username/token");
  expect(
    screen
      .getByRole("button", { name: "Apply mod sync" })
      .hasAttribute("disabled"),
  ).toBe(true);
});

it("keeps sync requirements compact and closes back to the save list", async () => {
  vi.spyOn(api, "saves").mockResolvedValue([
    {
      name: "large.zip",
      size_bytes: 200 * 1024 * 1024,
      modified_at: 1,
      revision: "save-one",
    },
  ]);
  vi.spyOn(api, "selection").mockResolvedValue({
    load_latest: false,
    revision: "one",
    save: "large.zip",
  });
  vi.spyOn(api, "saveModPlan").mockResolvedValue({
    revision: "plan-one",
    save: "large.zip",
    factorio_version: "2.1.19",
    engine_version: "2.1.23",
    mods: Array.from({ length: 30 }, (_, index) => ({
      name: `demo-${String(index)}`,
      version: "1.0.0",
      action: "download" as const,
    })),
    disable: [],
    blockers: Array.from(
      { length: 30 },
      (_, index) => `demo-${String(index)} requires a download.`,
    ),
  });
  mount(<Saves status={stopped} />);
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Sync mods" }));
  const dialog = await screen.findByRole("dialog", {
    name: "Sync mods with large.zip",
  });
  expect(within(dialog).getAllByRole("alert")).toHaveLength(1);
  expect(within(dialog).getByText("29 more requirements")).toBeDefined();
  const details = dialog.querySelector("details");
  expect(details?.open).toBe(false);
  expect(
    within(dialog)
      .getByRole("button", { name: "Apply mod sync" })
      .hasAttribute("disabled"),
  ).toBe(true);
  expect(
    within(dialog).getByRole("region", { name: "Mod changes" }),
  ).toBeDefined();
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(
    screen.getByRole("link", { name: "Download large.zip" }),
  ).toBeDefined();
});

it("shows server-reported sync progress and the current mod while downloads run", async () => {
  vi.spyOn(api, "saves").mockResolvedValue([
    {
      name: "progress.zip",
      size_bytes: 1,
      modified_at: 1,
      revision: "save-one",
    },
  ]);
  vi.spyOn(api, "selection").mockResolvedValue({
    load_latest: false,
    revision: "one",
    save: null,
  });
  vi.spyOn(api, "saveModPlan").mockResolvedValue({
    revision: "plan-one",
    save: "progress.zip",
    factorio_version: "2.1.23",
    engine_version: "2.1.23",
    mods: [{ name: "demo", version: "1.0.0", action: "download" }],
    disable: [],
    blockers: [],
  });
  let operationId = "";
  let finish:
    ((value: Awaited<ReturnType<typeof api.syncSaveMods>>) => void) | undefined;
  vi.spyOn(api, "syncSaveMods").mockImplementation((input) => {
    operationId = input.operation_id ?? "";
    return new Promise((resolve) => {
      finish = resolve;
    });
  });
  vi.spyOn(api, "saveSyncProgress").mockImplementation(() =>
    Promise.resolve({
      operation_id: operationId,
      save: "progress.zip",
      phase: "downloading",
      completed: 1,
      total: 3,
      current_mod: "demo",
      error: null,
    }),
  );
  mount(<Saves status={stopped} />);
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Sync mods" }));
  await user.click(
    await screen.findByRole("button", { name: "Apply mod sync" }),
  );
  await screen.findByText("Downloading: demo");
  expect(screen.getByText("1 / 3 downloaded")).toBeDefined();
  const bar = screen.getByRole("progressbar", {
    name: "Mod download progress",
  });
  expect(bar.getAttribute("value")).toBe("1");
  expect(bar.getAttribute("max")).toBe("3");
  expect(
    screen.getByRole("button", { name: "Cancel" }).hasAttribute("disabled"),
  ).toBe(true);
  if (!finish) throw new Error("Expected a pending sync");
  finish({ revision: "two", mods: [], portal_authenticated: true });
  await screen.findByText(/Mods synced/);
  expect(screen.queryByRole("dialog")).toBeNull();
});
