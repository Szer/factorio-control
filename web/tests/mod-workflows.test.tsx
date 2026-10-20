import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { api, ApiError } from "../src/api";
import type { ModsDocument, Status } from "../src/api";
import { modWorkflowApi } from "../src/modWorkflowApi";
import Mods from "../src/pages/Mods";
const document: ModsDocument = {
  revision: "mods-one",
  portal_authenticated: true,
  mods: [
    {
      name: "fixture",
      title: "Fixture",
      version: "2.0.0",
      enabled: true,
      file_name: "fixture_2.0.0.zip",
      size_bytes: 1,
      factorio_version: "2.1",
      dependencies: ["base"],
      author: "Synthetic",
      description: "",
    },
  ],
};
const status: Status = {
  state: "stopped",
  since: 1,
  active_save: null,
  started_at: null,
  binary_available: true,
  version: "2.1.23",
  error: null,
};
function mount(state: Status = status) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Mods status={state} />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.spyOn(modWorkflowApi, "progress").mockResolvedValue(null);
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
  vi.spyOn(api, "mods").mockResolvedValue(structuredClone(document));
  vi.spyOn(api, "modSettings").mockResolvedValue({
    revision: "settings-one",
    available: false,
    settings: [],
  });
  vi.spyOn(api, "modUpdates").mockResolvedValue({
    name: "fixture",
    installed_version: "2.0.0",
    latest_version: "2.0.0",
    latest_released_at: null,
    factorio_min_version: "2.1.0",
    compatible: true,
    update_available: false,
    authenticated: true,
    factorio_version: "2.1.23",
    message: null,
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
it("requires a preview and confirmation before applying the exact bulk plan", async () => {
  const plan = {
    revision: "mods-one",
    changes: [{ name: "fixture", current_version: "2.0.0", version: "3.0.0" }],
  };
  vi.spyOn(modWorkflowApi, "preview").mockResolvedValue(plan);
  const apply = vi
    .spyOn(modWorkflowApi, "apply")
    .mockResolvedValue({ ...document, revision: "mods-two" });
  mount();
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Manage mods" }));
  await user.click(screen.getByRole("button", { name: "Bulk update" }));
  expect(screen.queryByRole("button", { name: "Apply upgrades" })).toBeNull();
  await user.click(screen.getByRole("button", { name: "Preview upgrades" }));
  await user.click(
    await screen.findByRole("button", { name: "Apply upgrades" }),
  );
  expect(apply).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm" }));
  await waitFor(() => {
    expect(apply).toHaveBeenCalledWith(plan);
  });
});
it("installs an explicitly selected older compatible release after confirmation", async () => {
  vi.spyOn(modWorkflowApi, "releases").mockResolvedValue({
    name: "fixture",
    releases: [
      { version: "2.0.0", factorio_version: "2.1", released_at: null },
      { version: "1.0.0", factorio_version: "2.1", released_at: null },
    ],
  });
  const install = vi
    .spyOn(modWorkflowApi, "install")
    .mockResolvedValue({ ...document, revision: "mods-two" });
  mount();
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Manage mods" }));
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Installed mod versions" }),
    "fixture",
  );
  await user.selectOptions(
    await screen.findByRole("combobox", { name: "Portal release" }),
    "1.0.0",
  );
  await user.click(screen.getByRole("button", { name: "Install version" }));
  expect(install).not.toHaveBeenCalled();
  expect(screen.getByText(/currently 2.0.0/)).toBeDefined();
  await user.click(screen.getByRole("button", { name: "Confirm" }));
  await waitFor(() => {
    expect(install).toHaveBeenCalledWith("mods-one", "fixture", "1.0.0");
  });
});
it("shows dependency errors without reporting partial update success", async () => {
  const remove = vi
    .spyOn(modWorkflowApi, "remove")
    .mockRejectedValue(new ApiError("Other mod requires fixture", 400));
  mount();
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Manage mods" }));
  await user.click(screen.getByRole("button", { name: "Files & removal" }));
  await user.click(screen.getByRole("button", { name: "Remove" }));
  expect(remove).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Other mod requires fixture",
  );
  expect(screen.queryByText("Changes applied.")).toBeNull();
});
it("requires confirmation to restore a snapshot and keeps the table compact", async () => {
  vi.spyOn(modWorkflowApi, "packs").mockResolvedValue([
    { name: "working", mods: 1 },
  ]);
  const restore = vi
    .spyOn(modWorkflowApi, "restorePack")
    .mockResolvedValue({ ...document, revision: "mods-two" });
  mount();
  const user = userEvent.setup();
  expect(
    within(
      await screen.findByRole("table", { name: "Installed mod versions" }),
    ).getAllByRole("columnheader"),
  ).toHaveLength(5);
  await user.click(screen.getByRole("button", { name: "Manage mods" }));
  await user.click(screen.getByRole("button", { name: "Mod packs" }));
  await user.click(await screen.findByRole("button", { name: "Restore" }));
  expect(restore).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm" }));
  await waitFor(() => {
    expect(restore).toHaveBeenCalledWith("mods-one", "working");
  });
});
it("disables mutations while running and leaves exports available", async () => {
  mount({ ...status, state: "running" });
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: "Manage mods" }));
  await user.click(screen.getByRole("button", { name: "Files & removal" }));
  expect(
    screen.getByRole("button", { name: "Remove" }).hasAttribute("disabled"),
  ).toBe(true);
  expect(
    screen
      .getByRole("button", { name: "Import mod list" })
      .hasAttribute("disabled"),
  ).toBe(true);
  expect(
    screen
      .getByRole("link", { name: "Download all mods" })
      .getAttribute("href"),
  ).toBe("/api/mods/export");
});
