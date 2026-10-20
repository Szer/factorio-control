import { TestAccess } from "./access";
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
import type { ReactElement } from "react";
import { api, ApiError } from "../src/api";
import type { Status } from "../src/api";
import { operations } from "../src/operationsApi";
import ServerControls from "../src/components/ServerControls";
import { ArchiveSaveButton, SaveTools } from "../src/components/SaveTools";
import Console from "../src/pages/Console";
const stopped: Status = {
  state: "stopped",
  since: 1,
  active_save: null,
  started_at: null,
  binary_available: true,
  version: "2.1.23",
  error: null,
};
function mount(element: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <TestAccess>{element}</TestAccess>
    </QueryClientProvider>,
  );
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
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("resolves latest at server start instead of sending a stale preview name", async () => {
  vi.spyOn(api, "selection").mockResolvedValue({
    revision: "one",
    save: null,
    load_latest: true,
  });
  vi.spyOn(api, "saves").mockResolvedValue([
    { name: "_autosave1.zip", revision: "one", size_bytes: 1, modified_at: 1 },
  ]);
  const start = vi
    .spyOn(api, "start")
    .mockResolvedValue({ ...stopped, state: "starting" });
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
  expect(
    within(dialog).getByText("the newest save, including autosaves"),
  ).toBeTruthy();
  await user.click(
    within(dialog).getByRole("button", { name: "Start server" }),
  );
  await waitFor(() => {
    expect(start).toHaveBeenCalledWith(undefined);
  });
});

it("requires a separate confirmation for emergency force stop", async () => {
  vi.spyOn(api, "selection").mockResolvedValue({
    revision: "one",
    save: "world.zip",
    load_latest: false,
  });
  const kill = vi.spyOn(operations, "forceStop").mockResolvedValue(stopped);
  mount(<ServerControls status={{ ...stopped, state: "stopping" }} />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Force stop server" }));
  expect(kill).not.toHaveBeenCalled();
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Force stop without saving",
    }),
  );
  await waitFor(() => {
    expect(kill).toHaveBeenCalledOnce();
  });
});

it("keeps an archive confirmation open when a stale file revision is rejected", async () => {
  const archive = vi
    .spyOn(operations, "archiveSave")
    .mockRejectedValue(
      new ApiError("Save changed; refresh before archiving", 409),
    );
  mount(
    <ArchiveSaveButton
      name="world.zip"
      revision="stale"
      disabled={false}
      reason={undefined}
    />,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Archive world.zip" }));
  expect(archive).not.toHaveBeenCalled();
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Archive save",
    }),
  );
  await screen.findByText("Save changed; refresh before archiving");
  expect(screen.getByRole("dialog")).toBeTruthy();
  expect(archive).toHaveBeenCalledWith(
    { name: "world.zip", revision: "stale" },
    expect.anything(),
  );
});

it("does not imply a successful restore when the destination already exists", async () => {
  vi.spyOn(operations, "archivedSaves").mockResolvedValue([
    {
      id: "one",
      name: "world.zip",
      revision: "one",
      modified_at: 1,
      size_bytes: 1,
    },
  ]);
  vi.spyOn(operations, "restoreSave").mockRejectedValue(
    new ApiError("A save with that name already exists", 409),
  );
  mount(<SaveTools stopped />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Archived" }));
  await user.click(await screen.findByRole("button", { name: "Restore" }));
  await screen.findByText("A save with that name already exists");
  expect(screen.getByText("world.zip")).toBeTruthy();
});

it("requires scripting acknowledgement and clears it when the command is edited", async () => {
  vi.spyOn(api, "logs").mockResolvedValue([]);
  const send = vi
    .spyOn(operations, "command")
    .mockResolvedValue({ accepted: true });
  mount(<Console interval={60000} running />);
  const user = userEvent.setup();
  const input = screen.getByRole("textbox", {
    name: "Server command or chat message",
  });
  await user.type(input, "/c game.speed = 1");
  const submit = screen.getByRole("button", { name: "Send" });
  expect(submit.hasAttribute("disabled")).toBe(true);
  await user.click(screen.getByRole("checkbox"));
  await user.type(input, " ");
  expect(submit.hasAttribute("disabled")).toBe(true);
  await user.click(screen.getByRole("checkbox"));
  await user.click(submit);
  await waitFor(() => {
    expect(send).toHaveBeenCalledWith(
      { command: "/c game.speed = 1 ", acknowledge_scripting: true },
      expect.anything(),
    );
  });
  await screen.findByText("Command sent.");
});
