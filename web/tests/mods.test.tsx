import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { api } from "../src/api";
import type { ModsDocument, ModUpdates, Status } from "../src/api";
import Mods from "../src/pages/Mods";

const status: Status = {
  state: "stopped",
  since: 1,
  active_save: null,
  started_at: null,
  binary_available: true,
  version: "2.1.23",
  error: null,
};
const document: ModsDocument = {
  revision: "revision-one",
  portal_authenticated: true,
  mods: ["old", "unsupported", "current"].map((name) => ({
    name,
    title: `${name} mod`,
    version: "1.0.0",
    author: "Verbose author",
    description: "Unnecessary long description",
    dependencies: ["base >= 2.1"],
    factorio_version: "2.1",
    enabled: true,
    file_name: `${name}_1.0.0.zip`,
    size_bytes: 123,
  })),
};
const summaries: Record<string, ModUpdates> = {
  old: {
    name: "old",
    installed_version: "1.0.0",
    latest_version: "1.2.0",
    latest_released_at: "2026-10-19T14:00:00Z",
    factorio_min_version: "2.1.0",
    compatible: true,
    update_available: true,
    authenticated: true,
    factorio_version: "2.1.23",
    message: null,
  },
  unsupported: {
    name: "unsupported",
    installed_version: "1.0.0",
    latest_version: null,
    latest_released_at: null,
    factorio_min_version: "2.1.30",
    compatible: false,
    update_available: false,
    authenticated: true,
    factorio_version: "2.1.23",
    message: null,
  },
  current: {
    name: "current",
    installed_version: "1.0.0",
    latest_version: "1.0.0",
    latest_released_at: "2026-10-18T14:00:00Z",
    factorio_min_version: "2.1.0",
    compatible: true,
    update_available: false,
    authenticated: true,
    factorio_version: "2.1.23",
    message: null,
  },
};
function mount(active = true) {
  vi.spyOn(api, "mods").mockResolvedValue(document);
  vi.spyOn(api, "modSettings").mockResolvedValue({
    revision: "startup",
    settings: [],
    available: false,
  });
  const checks = vi.spyOn(api, "modUpdates").mockImplementation((name) => {
    const result = summaries[name];
    if (!result) throw new Error("Unknown fixture");
    return Promise.resolve(result);
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <Mods status={status} active={active} />
    </QueryClientProvider>,
  );
  return checks;
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("shows five compact columns with distinct upgrade and incompatible states", async () => {
  mount();
  const table = await screen.findByRole("table", {
    name: "Installed mod versions",
  });
  await screen.findByText("Incompatible");
  expect(
    within(table)
      .getAllByRole("columnheader")
      .map((node) => node.textContent),
  ).toEqual(["Name", "Current", "Latest", "Factorio min.", "Released"]);
  expect(screen.getByText("old mod").closest("tr")?.className).toBe(
    "mod-outdated",
  );
  expect(screen.getByText("unsupported mod").closest("tr")?.className).toBe(
    "mod-incompatible",
  );
  expect(screen.getByText("current mod").closest("tr")?.className).toBe("");
  expect(within(table).getByText("2026-10-19")).toBeTruthy();
  expect(within(table).getByText("2.1.30")).toBeTruthy();
  expect(within(table).getByText("Unavailable")).toBeTruthy();
  expect(screen.queryByText("Verbose author")).toBeNull();
  expect(screen.queryByText("Unnecessary long description")).toBeNull();
  expect(screen.queryByText("Dependencies")).toBeNull();
  expect(
    screen.queryByRole("button", { name: /Update current mod/ }),
  ).toBeNull();
});

it("updates the selected version with the current document revision and checks all again", async () => {
  const checks = mount();
  const update = vi
    .spyOn(api, "updateMod")
    .mockResolvedValue({ ...document, revision: "two" });
  const user = userEvent.setup();
  await user.click(
    await screen.findByRole("button", { name: "Update old mod to 1.2.0" }),
  );
  expect(update).toHaveBeenCalledWith(
    { name: "old", version: "1.2.0", revision: "revision-one" },
    expect.anything(),
  );
  await user.click(
    await screen.findByRole("button", { name: "Check all updates" }),
  );
  expect(checks.mock.calls.length).toBeGreaterThanOrEqual(6);
});

it("does not query release metadata while the Mods page is hidden", async () => {
  const checks = mount(false);
  await screen.findByRole("button", { name: "Installed mods" });
  expect(api.mods).not.toHaveBeenCalled();
  expect(checks).not.toHaveBeenCalled();
});
