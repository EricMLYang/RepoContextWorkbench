import { describe, expect, it } from "vitest";
import { langOf, tokenizeLine, type HlState } from "../lib/highlight";

const join = (t: { text: string }[]) => t.map((x) => x.text).join("");
const cls = (t: { cls: string; text: string }[]) =>
  t.filter((x) => x.cls).map((x) => x.cls + ":" + x.text);

describe("輕量語法上色", () => {
  it("langOf：副檔名與 md 語言標記", () => {
    expect(langOf("src/main.rs")).toBe("rust");
    expect(langOf("a/b.tsx")).toBe("ts");
    expect(langOf("python")).toBe("py");
    expect(langOf("README.md")).toBeNull();
  });
  it("片段串回去就是原本那一行；關鍵字、字串、數字、註解", () => {
    const st: HlState = { block: null };
    const line = 'fn main() { let s = "hi // no"; let n = 42; } // 說明';
    const t = tokenizeLine(line, "rust", st);
    expect(join(t)).toBe(line);
    expect(cls(t)).toEqual([
      "kw:fn",
      "kw:let",
      'str:"hi // no"',
      "kw:let",
      "num:42",
      "com:// 說明",
    ]);
  });
  it("Rust 生命週期不是字串；跨行區塊註解與 Python 三引號", () => {
    expect(cls(tokenizeLine("fn f<'a>(x: &'a str) {}", "rust", { block: null }))).toEqual([
      "kw:fn",
    ]);
    const st: HlState = { block: null };
    expect(cls(tokenizeLine("x = 1 /* 開始", "ts", st))).toEqual(["num:1", "com:/* 開始"]);
    expect(st.block).toBe("*/");
    expect(cls(tokenizeLine("中間 */ return", "ts", st))).toEqual(["com:中間 */", "kw:return"]);
    const py: HlState = { block: null };
    tokenizeLine('def f():  """說明', "py", py);
    expect(py.block).toBe('"""');
    expect(cls(tokenizeLine('  結尾""" + x', "py", py))).toEqual(['str:  結尾"""']);
  });
});
