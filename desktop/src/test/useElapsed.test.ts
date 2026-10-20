import { describe, expect, it } from "vitest";
import { elapsedLabel } from "../lib/useElapsed";

describe("elapsedLabel：超過門檻才顯示，兩分鐘以上改用分", () => {
  it("門檻以下空字串、秒、分", () => {
    expect(elapsedLabel(1)).toBe("");
    expect(elapsedLabel(2)).toBe(" · 2 秒");
    expect(elapsedLabel(119)).toBe(" · 119 秒");
    expect(elapsedLabel(120)).toBe(" · 2 分");
    expect(elapsedLabel(20, 30)).toBe("");
    expect(elapsedLabel(45, 30)).toBe(" · 45 秒");
  });
});
