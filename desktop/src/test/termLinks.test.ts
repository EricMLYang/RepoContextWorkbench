import { describe, expect, it } from "vitest";
import { findInLines, findPathHits, lineCells } from "../lib/termLinks";

const texts = (line: string) => findPathHits(line).map((h) => h.text);

describe("終端輸出裡的路徑（F-95）", () => {
  it("認得相對、絕對、帶行欄的路徑與單一檔名:行", () => {
    expect(texts("Edited src/lib/view.ts:12:5 and ./docs/x.md")).toEqual([
      "src/lib/view.ts:12:5",
      "./docs/x.md",
    ]);
    expect(texts("error at /Users/me/r/a.rs(30,2).")).toEqual(["/Users/me/r/a.rs(30,2)"]);
    expect(texts("see README.md:4 or ~/notes/todo.txt")).toEqual([
      "README.md:4",
      "~/notes/todo.txt",
    ]);
  });

  it("網址、and/or、沒有行號的單一字詞不算", () => {
    expect(texts("open https://example.com/a/b.html now")).toEqual([]);
    expect(texts("yes and/or no; version 1.2.3; main.rs is fine")).toEqual([]);
  });

  it("位置對得回原字串", () => {
    const line = "→ src/a.rs:3 改好了";
    const [h] = findPathHits(line);
    expect(line.slice(h.start, h.end)).toBe("src/a.rs:3");
  });

  it("全形字佔兩格：字串位置換算成第幾格", () => {
    const cells = [
      { c: "改", w: 2 },
      { c: "", w: 0 },
      { c: " ", w: 1 },
      { c: "a", w: 1 },
    ];
    const line = {
      length: cells.length,
      getCell: (x: number) => ({ getChars: () => cells[x].c, getWidth: () => cells[x].w }),
    };
    const { text, cellOf } = lineCells(line);
    expect(text).toBe("改 a");
    expect(cellOf).toEqual([0, 2, 3]);
  });
});

describe("終端內搜尋（F-111）", () => {
  it("每行所有出現位置、不分大小寫、空字串沒有結果", () => {
    expect(findInLines(["Error: x", "ok", "error error"], "error")).toEqual([
      { y: 0, start: 0, end: 5 },
      { y: 2, start: 0, end: 5 },
      { y: 2, start: 6, end: 11 },
    ]);
    expect(findInLines(["a"], "")).toEqual([]);
  });
});
