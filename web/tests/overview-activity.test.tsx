import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import Overview from "../src/pages/Overview";
import type { Status } from "../src/api";
import { TestAccess } from "./access";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it("accepts and renders mod activity from the telemetry API with versions and newest events first", async () => {
  const telemetry = {
    enabled: false,
    available: false,
    sampled_at: null,
    players: [],
    research: null,
    ups: null,
    target_ups: null,
    history: [],
    error: null,
    activity: [
      { id: 1, timestamp: 100, kind: "mod_added", message: "demo 1.0.0" },
      {
        id: 2,
        timestamp: 100,
        kind: "mod_updated",
        message: "demo: 1.0.0 → 2.0.0",
      },
      { id: 3, timestamp: 101, kind: "mod_removed", message: "demo 2.0.0" },
    ],
  };
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>((input) => {
      const path =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.pathname
            : input.url;
      if (path === "/api/telemetry")
        return Promise.resolve(new Response(JSON.stringify(telemetry)));
      if (path === "/api/saves/selection")
        return Promise.resolve(
          new Response(
            JSON.stringify({
              revision: "one",
              save: "world.zip",
              load_latest: false,
            }),
          ),
        );
      return Promise.reject(new Error(`Unexpected request: ${path}`));
    }),
  );
  const status: Status = {
    state: "stopped",
    since: 1,
    active_save: null,
    started_at: null,
    binary_available: false,
    version: "2.1.23",
    error: null,
  };
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <TestAccess>
        <Overview status={status} interval={60_000} />
      </TestAccess>
    </QueryClientProvider>,
  );
  expect(await screen.findByText("demo: 1.0.0 → 2.0.0")).toBeTruthy();
  const panel = screen
    .getByRole("heading", { name: "Recent activity" })
    .closest("section");
  if (!panel) throw new Error("Missing Recent activity section");
  expect(within(panel).getByText("mod added")).toBeTruthy();
  expect(within(panel).getByText("mod updated")).toBeTruthy();
  expect(within(panel).getByText("mod removed")).toBeTruthy();
  expect(
    Array.from(
      panel.querySelectorAll(".activity-item p"),
      (item) => item.textContent,
    ),
  ).toEqual(["demo 2.0.0", "demo: 1.0.0 → 2.0.0", "demo 1.0.0"]);
  client.clear();
});
