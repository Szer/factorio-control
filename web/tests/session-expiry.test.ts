import { afterEach, expect, it, vi } from "vitest";
import { expireSessionAt } from "../src/sessionExpiry";

afterEach(() => {
  vi.useRealTimers();
});

it("keeps a 30-day session alive across the browser timer limit and expires at its deadline", () => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
  const lifetime = 30 * 24 * 60 * 60 * 1000;
  const expired = vi.fn();
  const cancel = expireSessionAt(lifetime / 1000, expired);
  vi.advanceTimersByTime(2_147_483_647);
  expect(expired).not.toHaveBeenCalled();
  vi.advanceTimersByTime(lifetime - 2_147_483_647 - 1);
  expect(expired).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(expired).toHaveBeenCalledTimes(1);
  cancel();
});

it("cancels the rearmed timer when the session changes", () => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
  const expired = vi.fn();
  const cancel = expireSessionAt(30 * 24 * 60 * 60, expired);
  vi.advanceTimersByTime(2_147_483_647);
  cancel();
  vi.advanceTimersByTime(30 * 24 * 60 * 60 * 1000);
  expect(expired).not.toHaveBeenCalled();
});
