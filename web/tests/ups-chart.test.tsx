import { afterEach, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { UpsChart } from "../src/pages/Overview";
import type { Telemetry } from "../src/api";

afterEach(cleanup);

function telemetry(history: Telemetry["history"]): Telemetry {
  return {
    enabled: true,
    available: true,
    sampled_at: 1200,
    players: [],
    research: [],
    ups: 55,
    target_ups: 90,
    history,
    activity: [],
    error: null,
  };
}
function point(
  timestamp: number,
  target: number | null,
  state: Telemetry["history"][number]["state"] = "active",
): Telemetry["history"][number] {
  return {
    timestamp,
    ups: state === "active" ? 55 : null,
    target_ups: target,
    state,
  };
}
function targetPath(container: HTMLElement): string | null | undefined {
  return container.querySelector("path.chart-target")?.getAttribute("d");
}

it("steps at each recorded target change without rewriting history when the current target changes", () => {
  const data = telemetry([
    point(900, 60),
    point(1000, 30),
    point(1100, 90),
    point(1200, 90),
  ]);
  const { container, rerender } = render(
    <UpsChart telemetry={data} now={1200} />,
  );
  const expected = "M0.00,53.33 H300.00 V106.67 H600.00 V0.00 H900.00 V0.00 ";
  expect(targetPath(container)).toBe(expected);
  expect(container.querySelectorAll("line.chart-target")).toHaveLength(0);
  expect(container.querySelectorAll(".chart-target-point")).toHaveLength(3);
  expect(screen.getByText(/Target 30.0 UPS/)).toBeTruthy();
  expect(screen.getByLabelText("UPS chart legend").textContent).toContain(
    "Target UPS",
  );
  rerender(<UpsChart telemetry={{ ...data, target_ups: 42 }} now={1200} />);
  expect(targetPath(container)).toBe(expected);
});

it("keeps targets while paused or collecting and breaks the line across unavailable samples", () => {
  const { container } = render(
    <UpsChart
      telemetry={telemetry([
        point(900, 60),
        point(950, 30, "paused"),
        point(1000, null, "unavailable"),
        point(1100, 90, "collecting"),
        point(1200, 90),
      ])}
      now={1200}
    />,
  );
  expect(targetPath(container)).toBe(
    "M0.00,53.33 H150.00 V106.67 M600.00,0.00 H900.00 V0.00 ",
  );
  expect(container.querySelector(".chart-line")?.getAttribute("d")).toBe(
    "M0.00,62.22 M900.00,62.22 ",
  );
});

it("uses the preceding target at the window boundary and preserves older changes in wider ranges", () => {
  const data = telemetry([point(800, 120), point(950, 60), point(1200, 60)]);
  const { container } = render(<UpsChart telemetry={data} now={1200} />);
  expect(targetPath(container)).toBe(
    "M0.00,0.00 H150.00 V80.00 H900.00 V80.00 ",
  );
  fireEvent.click(screen.getByRole("button", { name: "15m" }));
  expect(targetPath(container)).toBe(
    "M500.00,0.00 H650.00 V80.00 H900.00 V80.00 ",
  );
  fireEvent.click(screen.getByRole("button", { name: "1h" }));
  expect(targetPath(container)).toBe(
    "M800.00,0.00 H837.50 V80.00 H900.00 V80.00 ",
  );
});

it("does not invent targets before observations, during gaps, or from the current target alone", () => {
  const { container, rerender } = render(
    <UpsChart telemetry={telemetry([])} now={1200} />,
  );
  expect(targetPath(container)).toBe("");
  expect(screen.getByText("Waiting for active tick samples")).toBeTruthy();
  rerender(
    <UpsChart
      telemetry={telemetry([point(800, null, "unavailable"), point(1000, 60)])}
      now={1200}
    />,
  );
  expect(targetPath(container)).toBe("M300.00,0.00 ");
  rerender(
    <UpsChart
      telemetry={telemetry([
        point(800, 120),
        point(900, 60),
        point(1200, 60),
        point(1300, 180),
      ])}
      now={1200}
    />,
  );
  expect(targetPath(container)).toBe("M0.00,0.00 H900.00 V0.00 ");
});

it("shows target-only history while paused without an empty-chart overlay", () => {
  const { container } = render(
    <UpsChart
      telemetry={{
        ...telemetry([point(900, 60, "paused"), point(1200, 30, "paused")]),
        ups: null,
        target_ups: 30,
      }}
      now={1200}
    />,
  );
  expect(targetPath(container)).toBe("M0.00,0.00 H900.00 V80.00 ");
  expect(screen.queryByText("Waiting for active tick samples")).toBeNull();
});
