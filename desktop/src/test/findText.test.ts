import { describe, expect, it } from "vitest";
import { findRanges } from "../lib/findText";

describe("findRanges：檔案裡找字", () => {
  it("上色切成好幾段也找得到；不同行不會連起來；行號不算", () => {
    const root = document.createElement("div");
    root.innerHTML =
      '<div class="cl"><span aria-hidden="true">1</span><span><span class="tk-kw">fn</span> main</span></div>' +
      '<div class="cl"><span aria-hidden="true">2</span><span>ab</span></div>' +
      '<div class="cl"><span aria-hidden="true">3</span><span>cd</span></div>';
    const r = findRanges(root, "fn main");
    expect(r.map((x) => x.toString())).toEqual(["fn main"]);
    expect(findRanges(root, "bc")).toHaveLength(0);
    expect(findRanges(root, "1")).toHaveLength(0);
    expect(findRanges(root, "N")).toHaveLength(2);
  });
});
