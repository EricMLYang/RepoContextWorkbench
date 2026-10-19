import { describe, expect, it } from "vitest";
import { gitMarks } from "../lib/gitMarks";

describe("gitMarks：檔案照狀態、資料夾取最重、沒 push 最輕", () => {
  it("檔案自己的狀態；上層資料夾一路標到根下第一層", () => {
    const m = gitMarks([{ path: "a/b/c.md", status: "untracked" }], []);
    expect(m.get("a/b/c.md")).toBe("untracked");
    expect(m.get("a/b")).toBe("untracked");
    expect(m.get("a")).toBe("untracked");
    expect(m.has("")).toBe(false);
  });
  it("資料夾取最重的：修改壓過新檔、衝突壓過修改", () => {
    const m = gitMarks(
      [
        { path: "src/x.rs", status: "untracked" },
        { path: "src/y.rs", status: "modified" },
        { path: "src/z.rs", status: "conflicted" },
      ],
      [],
    );
    expect(m.get("src")).toBe("conflicted");
    expect(m.get("src/x.rs")).toBe("untracked");
  });
  it("沒 push 的檔標 unpushed；同一個檔又改了就算還沒 commit 的；資料夾也不會被沒 push 蓋回去", () => {
    const m = gitMarks([{ path: "docs/a.md", status: "modified" }], ["docs/a.md", "docs/b.md"]);
    expect(m.get("docs/a.md")).toBe("modified");
    expect(m.get("docs/b.md")).toBe("unpushed");
    expect(m.get("docs")).toBe("modified");
  });
});
