import { afterEach, describe, expect, it, vi } from "vitest";
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
import { api, ApiError, settingsSchema } from "../src/api";
import type { SettingsDocument, RawSettingsDocument, Status } from "../src/api";
import SettingsPage from "../src/pages/Settings";
import Administrators from "../src/components/Administrators";
import ServerControls from "../src/components/ServerControls";
import App from "../src/App";

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
    revision: "one",
    save: "test.zip",
  });
  mount(<ServerControls status={{ ...stopped, state: "paused" }} />);
  expect(screen.getByRole("button", { name: "Stop server" })).toBeDefined();
  expect(screen.queryByRole("button", { name: "Start server" })).toBeNull();
});

it("does not enable Start without an explicitly selected save", async () => {
  vi.spyOn(api, "selection").mockResolvedValue({ revision: "one", save: null });
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
