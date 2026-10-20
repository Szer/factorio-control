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
import type { SettingsDocument, Status } from "../src/api";
import SettingsPage from "../src/pages/Settings";
import Players from "../src/pages/Players";
import ServerControls from "../src/components/ServerControls";
import App from "../src/App";

const settings: SettingsDocument = {
  revision: "one",
  settings: {
    name: "Nauvis",
    description: "",
    max_players: 12,
    auto_pause: true,
    autosave_interval: 5,
    visibility: { public: false, lan: true },
  },
};
const stopped: Status = {
  state: "stopped",
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
    let finish: ((document: SettingsDocument) => void) | undefined;
    vi.spyOn(api, "saveSettings").mockImplementation(
      () =>
        new Promise<SettingsDocument>((resolve) => {
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
    vi.spyOn(api, "saveSettings").mockRejectedValue(
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
    const first = mount(<Players />);
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
    mount(<Players />);
    await screen.findByText("paragith");
    expect(screen.getByText("Szer")).toBeDefined();
  });
  it("preserves unsaved settings while navigating away and back", async () => {
    window.location.hash = "settings";
    vi.spyOn(api, "status").mockResolvedValue(stopped);
    vi.spyOn(api, "settings").mockResolvedValue(structuredClone(settings));
    vi.spyOn(api, "admins").mockResolvedValue({ revision: "one", admins: [] });
    vi.spyOn(api, "saves").mockResolvedValue([]);
    vi.spyOn(api, "logs").mockResolvedValue([]);
    mount(<App />);
    const user = userEvent.setup();
    const name = await screen.findByRole("textbox", { name: /Server name/ });
    await user.clear(name);
    await user.type(name, "Still here");
    await user.click(screen.getByRole("link", { name: "Players" }));
    await screen.findByRole("heading", { name: "Players", level: 1 });
    await user.click(screen.getByRole("link", { name: "Settings" }));
    await screen.findByRole("heading", { name: "Settings", level: 1 });
    expect(
      screen
        .getByRole("textbox", { name: /Server name/ })
        .getAttribute("value"),
    ).toBe("Still here");
  });
});

it("sends the world actually selected in the launch dialog", async () => {
  const saves = ["alpha.zip", "beta.zip"].map((name) => ({
    name,
    size_bytes: 123,
    modified_at: 1,
  }));
  vi.spyOn(api, "saves").mockResolvedValue(saves);
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
  const showModal = vi.fn(function (this: HTMLDialogElement) {
    this.setAttribute("open", "");
  });
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value: showModal,
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.removeAttribute("open");
    },
  });
  await user.click(screen.getByRole("button", { name: "Start server" }));
  const dialog = screen.getByRole("dialog");
  await user.selectOptions(
    within(dialog).getByRole("combobox", { name: "World" }),
    "beta.zip",
  );
  await user.click(
    within(dialog).getByRole("button", { name: "Start server" }),
  );
  await waitFor(() => {
    expect(start).toHaveBeenCalledWith("beta.zip");
  });
  expect(showModal).toHaveBeenCalled();
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
