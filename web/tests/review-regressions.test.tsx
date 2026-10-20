import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { TestAccess } from "./access";
import { api } from "../src/api";
import type { Status, ModsDocument, ModSettingsDocument } from "../src/api";
import { operations } from "../src/operationsApi";
import Mods from "../src/pages/Mods";
import Saves from "../src/pages/Saves";
import ServerControls from "../src/components/ServerControls";
import { SaveTools } from "../src/components/SaveTools";

const stopped: Status = {
  state: "stopped",
  since: 1,
  active_save: null,
  started_at: null,
  binary_available: true,
  version: "2.1.23",
  error: null,
};
const mods: ModsDocument = {
  revision: "mods-one",
  portal_authenticated: true,
  mods: [],
};
const initial: ModSettingsDocument = {
  revision: "one",
  available: true,
  settings: [
    { name: "test-setting", kind: "integer", value: "1", components: [] },
  ],
};
function mount(element: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const wrap = (child: ReactElement) => (
    <QueryClientProvider client={client}>
      <TestAccess>{child}</TestAccess>
    </QueryClientProvider>
  );
  const result = render(wrap(element));
  return {
    client,
    ...result,
    update: (child: ReactElement) => {
      result.rerender(wrap(child));
    },
  };
}
beforeEach(() => {
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
  vi.spyOn(api, "mods").mockResolvedValue(mods);
  vi.spyOn(api, "modSettings").mockResolvedValue(initial);
  vi.spyOn(api, "selection").mockResolvedValue({
    revision: "selected",
    save: "world.zip",
    load_latest: false,
  });
  vi.spyOn(api, "saves").mockResolvedValue([
    { name: "world.zip", revision: "save", modified_at: 1, size_bytes: 10 },
  ]);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("discards an edited startup setting to the newest polled document and revision", async () => {
  const save = vi
    .spyOn(api, "saveModSettings")
    .mockResolvedValue({ ...initial, revision: "three" });
  const { client } = mount(<Mods status={stopped} active />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Startup settings" }));
  const input = await screen.findByRole("spinbutton", { name: "test-setting" });
  await user.clear(input);
  await user.type(input, "9");
  act(() => {
    client.setQueryData(["mod-settings"], {
      ...initial,
      revision: "two",
      settings: [
        { name: "test-setting", kind: "integer", value: "2", components: [] },
      ],
    });
  });
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(input).toHaveProperty("value", "2");
  await user.clear(input);
  await user.type(input, "3");
  await user.click(
    screen.getByRole("button", { name: "Save startup settings" }),
  );
  await waitFor(() => {
    expect(save.mock.calls[0]?.[0]).toMatchObject({ revision: "two" });
  });
});

it("keeps saves and mods visible when a background refresh fails", async () => {
  const saves = mount(<Saves status={stopped} />);
  await screen.findByRole("link", { name: "Download world.zip" });
  vi.mocked(api.saves).mockRejectedValue(new Error("Save refresh failed"));
  await act(async () => {
    await saves.client.refetchQueries({ queryKey: ["saves"] });
  });
  const saveWarning = await screen.findByRole("alert");
  expect(screen.getByRole("table")).toBeTruthy();
  expect(saveWarning.textContent).toContain("Showing the last loaded saves");
  cleanup();
  const installed = mount(<Mods status={stopped} active />);
  await screen.findByText(
    "No uploaded mods. Factorio's bundled content is managed by the game.",
  );
  vi.mocked(api.mods).mockRejectedValue(new Error("Mod refresh failed"));
  await act(async () => {
    await installed.client.refetchQueries({ queryKey: ["mods"] });
  });
  const modWarning = await screen.findByRole("alert");
  expect(
    screen.getByText(
      "No uploaded mods. Factorio's bundled content is managed by the game.",
    ),
  ).toBeTruthy();
  expect(modWarning.textContent).toContain("Showing the last loaded mods");
});

it("does not let an older status request overwrite the completed start result", async () => {
  const starting = { ...stopped, state: "starting" as const };
  vi.spyOn(api, "start").mockResolvedValue(starting);
  const { client } = mount(<ServerControls status={stopped} />);
  const user = userEvent.setup();
  await waitFor(() => {
    expect(
      screen
        .getByRole("button", { name: "Start server" })
        .hasAttribute("disabled"),
    ).toBe(false);
  });
  let resolve: ((value: Status) => void) | undefined;
  const oldPoll = client
    .query({
      queryKey: ["status"],
      queryFn: () =>
        new Promise<Status>((done) => {
          resolve = done;
        }),
    })
    .catch(() => undefined);
  await user.click(screen.getByRole("button", { name: "Start server" }));
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Start server",
    }),
  );
  await waitFor(() => {
    expect(client.getQueryData(["status"])).toEqual(starting);
  });
  await act(async () => {
    resolve?.(stopped);
    await oldPoll;
  });
  expect(client.getQueryData(["status"])).toEqual(starting);
});

it("normalizes a create name without duplicating an uppercase ZIP extension", async () => {
  const create = vi.spyOn(operations, "createSave").mockResolvedValue({
    name: "Base.ZIP",
    revision: "new",
    size_bytes: 1,
    modified_at: 1,
  });
  mount(<SaveTools stopped />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "New save" }));
  await user.type(
    screen.getByRole("textbox", { name: "Name" }),
    "  Base.ZIP  ",
  );
  await user.click(screen.getByRole("button", { name: "Create save" }));
  await waitFor(() => {
    expect(create.mock.calls[0]?.[0]).toBe("Base.ZIP");
  });
});

it("shows a useful message for an invalid successful operations response", async () => {
  vi.stubGlobal(
    "fetch",
    vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify({ unexpected: "body" }), { status: 200 }),
      ),
  );
  await expect(operations.fileLog()).rejects.toThrow(
    "The server returned an unexpected response. Refresh and try again.",
  );
});

it("names the latest-save policy in its success notice", async () => {
  vi.spyOn(api, "selectSave").mockResolvedValue({
    revision: "next",
    save: null,
    load_latest: true,
  });
  mount(<Saves status={stopped} />);
  const user = userEvent.setup();
  await waitFor(() => {
    expect(
      screen
        .getByRole("button", { name: "Always load latest" })
        .hasAttribute("disabled"),
    ).toBe(false);
  });
  await user.click(screen.getByRole("button", { name: "Always load latest" }));
  expect(
    await screen.findByText(
      "The latest save (including autosaves) will be used on the next start.",
    ),
  ).toBeTruthy();
});

it("closes a native modal when browser navigation hides the Mods page", async () => {
  const close = vi.spyOn(HTMLDialogElement.prototype, "close");
  const { update } = mount(<Mods status={stopped} active />);
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Manage mods" }));
  expect(screen.getByRole("dialog").hasAttribute("open")).toBe(true);
  update(<Mods status={stopped} active={false} />);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(close).toHaveBeenCalled();
});

it("removes only one occurrence from a legacy administrator list with duplicate names", async () => {
  const { default: Administrators } =
    await import("../src/components/Administrators");
  vi.spyOn(api, "admins").mockResolvedValue({
    revision: "duplicate-list",
    admins: ["player", "player"],
  });
  const save = vi
    .spyOn(api, "saveAdmins")
    .mockResolvedValue({ revision: "normalized", admins: ["player"] });
  const errors = vi.spyOn(console, "error");
  mount(<Administrators />);
  const buttons = await screen.findAllByRole("button", {
    name: "Remove player",
  });
  const first = buttons[0];
  if (!first) throw new Error("Missing fixture administrator");
  await userEvent.setup().click(first);
  expect(screen.getAllByRole("button", { name: "Remove player" })).toHaveLength(
    1,
  );
  await userEvent
    .setup()
    .click(screen.getByRole("button", { name: "Save administrators" }));
  await waitFor(() => {
    expect(save.mock.calls[0]?.[0]).toEqual({
      revision: "duplicate-list",
      admins: ["player"],
    });
  });
  expect(errors.mock.calls.flat().join(" ")).not.toContain("same key");
});

it.each(["signed out", "different account", "renewed session"] as const)(
  "ignores a pending mutation response after %s replaces its session",
  async (replacement) => {
    const { authKey } = await import("../src/authApi");
    let finish: ((value: Status) => void) | undefined;
    vi.spyOn(api, "start").mockImplementation(
      () =>
        new Promise<Status>((resolve) => {
          finish = resolve;
        }),
    );
    const { client } = mount(<ServerControls status={stopped} />);
    const user = userEvent.setup();
    await waitFor(() => {
      expect(
        screen
          .getByRole("button", { name: "Start server" })
          .hasAttribute("disabled"),
      ).toBe(false);
    });
    await user.click(screen.getByRole("button", { name: "Start server" }));
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Start server",
      }),
    );
    await waitFor(() => {
      expect(finish).toBeTypeOf("function");
    });
    const currentStatus = { ...stopped, state: "running" as const };
    act(() => {
      client.setQueryData(
        authKey,
        replacement === "signed out"
          ? { user: null, expires_at: null }
          : {
              user: {
                username:
                  replacement === "different account" ? "another" : "owner",
                account_id:
                  replacement === "different account"
                    ? "another-id"
                    : "fixture-owner",
                role: "admin",
                must_change_password: false,
              },
              expires_at: 9999999998,
            },
      );
      client.setQueryData(["status"], currentStatus);
    });
    await act(async () => {
      finish?.({ ...stopped, state: "starting" });
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(screen.queryByText("Working…")).toBeNull();
    });
    expect(client.getQueryData(["status"])).toEqual(currentStatus);
  },
);

it("allows a 200 MiB mod ZIP through the upload flow", async () => {
  const upload = vi
    .spyOn(api, "uploadMod")
    .mockResolvedValue({ ...mods, revision: "uploaded" });
  mount(<Mods status={stopped} active />);
  const input = await screen.findByLabelText("Upload mod ZIP");
  await waitFor(() => {
    expect(input.hasAttribute("disabled")).toBe(false);
  });
  const file = new File(["synthetic archive"], "large_1.0.0.zip", {
    type: "application/zip",
  });
  Object.defineProperty(file, "size", { value: 200 * 1024 * 1024 });
  await userEvent.setup().upload(input, file);
  await waitFor(() => {
    expect(upload.mock.calls[0]?.[0]).toMatchObject({
      file,
      revision: "mods-one",
    });
  });
  expect(screen.queryByText(/limited to/)).toBeNull();
});

it.each(["expiry", "account switch"] as const)(
  "does not publish a completed start across %s while cancellation is pending",
  async (replacement) => {
    const { authKey } = await import("../src/authApi");
    vi.spyOn(api, "start").mockResolvedValue({ ...stopped, state: "starting" });
    const { client } = mount(<ServerControls status={stopped} />);
    let release: (() => void) | undefined;
    const originalCancel = client.cancelQueries.bind(client);
    vi.spyOn(client, "cancelQueries").mockImplementation((filters, options) =>
      filters?.queryKey?.[0] === "status"
        ? new Promise<void>((resolve) => {
            release = resolve;
          })
        : originalCancel(filters, options),
    );
    const user = userEvent.setup();
    await waitFor(() => {
      expect(
        screen
          .getByRole("button", { name: "Start server" })
          .hasAttribute("disabled"),
      ).toBe(false);
    });
    await user.click(screen.getByRole("button", { name: "Start server" }));
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Start server",
      }),
    );
    await waitFor(() => {
      expect(release).toBeTypeOf("function");
    });
    const nextStatus = { ...stopped, state: "running" as const };
    act(() => {
      client.setQueryData(
        authKey,
        replacement === "expiry"
          ? { user: null, expires_at: null }
          : {
              user: {
                username: "another",
                account_id: "another-account",
                role: "admin",
                must_change_password: false,
              },
              expires_at: 9999999999,
            },
      );
      client.setQueryData(["status"], nextStatus);
    });
    await act(async () => {
      release?.();
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(screen.queryByText("Working…")).toBeNull();
    });
    expect(client.getQueryData(["status"])).toEqual(nextStatus);
    expect(screen.getByRole("dialog")).toBeTruthy();
  },
);

it("preserves the create-save draft when its refresh finishes after session expiry", async () => {
  const { authKey } = await import("../src/authApi");
  vi.spyOn(operations, "createSave").mockResolvedValue({
    name: "draft.zip",
    revision: "new",
    size_bytes: 1,
    modified_at: 1,
  });
  const { client } = mount(<SaveTools stopped />);
  let release: (() => void) | undefined;
  vi.spyOn(client, "invalidateQueries").mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        release = resolve;
      }),
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "New save" }));
  await user.type(screen.getByRole("textbox", { name: "Name" }), "draft.zip");
  await user.click(screen.getByRole("button", { name: "Create save" }));
  await waitFor(() => {
    expect(release).toBeTypeOf("function");
  });
  act(() => {
    client.setQueryData(authKey, { user: null, expires_at: null });
  });
  await act(async () => {
    release?.();
    await Promise.resolve();
  });
  expect(
    await screen.findByRole("button", { name: "Create save" }),
  ).toBeTruthy();
  expect(screen.getByRole("textbox", { name: "Name" })).toHaveProperty(
    "value",
    "draft.zip",
  );
});

it("reports an expired session before sending a mutation request", async () => {
  const { authKey } = await import("../src/authApi");
  const start = vi.spyOn(api, "start");
  const { client } = mount(<ServerControls status={stopped} />);
  const user = userEvent.setup();
  await waitFor(() => {
    expect(
      screen
        .getByRole("button", { name: "Start server" })
        .hasAttribute("disabled"),
    ).toBe(false);
  });
  await user.click(screen.getByRole("button", { name: "Start server" }));
  act(() => {
    client.setQueryData(authKey, { user: null, expires_at: null });
  });
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Start server",
    }),
  );
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Sign in again before changing server data.",
  );
  expect(start).not.toHaveBeenCalled();
});
