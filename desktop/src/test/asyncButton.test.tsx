import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AsyncButton } from "../components/AsyncButton";

describe("AsyncButton：執行中鎖住；超過 2 秒顯示已過秒數；結束就收掉", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  it("慢動作顯示「· N 秒」，完成後消失、按鈕解鎖", async () => {
    let finish: () => void = () => {};
    const run = vi.fn(() => new Promise<void>((r) => (finish = r)));
    render(
      <AsyncButton run={run} onError={() => {}}>
        請 Agent 審查
      </AsyncButton>,
    );
    const btn = screen.getByRole("button");
    await act(async () => fireEvent.click(btn));
    expect((btn as HTMLButtonElement).disabled).toBe(true);
    expect(btn.getAttribute("aria-busy")).toBe("true");
    expect(btn.textContent).toBe("請 Agent 審查");
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });
    expect(btn.textContent).toBe("請 Agent 審查");
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(btn.textContent).toBe("請 Agent 審查 · 3 秒");
    await act(async () => {
      finish();
    });
    expect(btn.textContent).toBe("請 Agent 審查");
    expect((btn as HTMLButtonElement).disabled).toBe(false);
    expect(btn.hasAttribute("aria-busy")).toBe(false);
  });
});
