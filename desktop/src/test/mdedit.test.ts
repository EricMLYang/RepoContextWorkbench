import { describe, expect, it } from "vitest";
import {
  continueList,
  indent,
  lineOffset,
  linkForDrop,
  mdHeadings,
  relPath,
  slug,
  textStats,
  toggleLinePrefix,
  toggleTask,
  wrap,
} from "../lib/mdedit";

describe("mdedit：HackMD 式編輯的純文字操作", () => {
  it("wrap：包住選取、再按一次取消；沒選字插入預設字並選起來", () => {
    expect(wrap("a word b", 2, 6, "**")).toEqual({ text: "a **word** b", start: 4, end: 8 });
    expect(wrap("a **word** b", 4, 8, "**")).toEqual({ text: "a word b", start: 2, end: 6 });
    expect(wrap("ab", 1, 1, "`", "`", "code")).toEqual({ text: "a`code`b", start: 2, end: 6 });
  });

  it("toggleLinePrefix：多行一起加，都有就一起拿掉", () => {
    const t = "one\ntwo\nthree";
    const on = toggleLinePrefix(t, 0, 7, "> ");
    expect(on.text).toBe("> one\n> two\nthree");
    expect(toggleLinePrefix(on.text, 0, on.end, "> ").text).toBe(t);
  });

  it("continueList：延續清單、數字遞增、待辦變沒勾；空項目結束清單", () => {
    expect(continueList("- a", 3)).toEqual({ text: "- a\n- ", start: 6, end: 6 });
    expect(continueList("  9. a", 6)!.text).toBe("  9. a\n  10. ");
    expect(continueList("- [x] done", 10)!.text).toBe("- [x] done\n- [ ] ");
    expect(continueList("- a\n- ", 6)).toEqual({ text: "- a\n", start: 4, end: 4 });
    expect(continueList("plain", 5)).toBeNull();
  });

  it("indent：清單行整行縮排、Shift+Tab 取消；一般文字插兩個空白", () => {
    expect(indent("- a", 3, 3, false)).toEqual({ text: "  - a", start: 5, end: 5 });
    expect(indent("  - a", 5, 5, true)).toEqual({ text: "- a", start: 3, end: 3 });
    expect(indent("ab", 1, 1, false)).toEqual({ text: "a  b", start: 3, end: 3 });
  });
});

describe("toggleTask：預覽點待辦方塊改原文", () => {
  it("照文件順序找第 n 個；跳過程式碼區塊與 front matter；巢狀與引言也算", () => {
    const t =
      "---\ntitle: x\n- [ ] 不算\n---\n- [ ] 一\n```\n- [ ] 程式碼\n```\n  - [x] 二\n> - [ ] 三";
    expect(toggleTask(t, 0)).toContain("- [x] 一");
    expect(toggleTask(t, 1)).toContain("  - [ ] 二");
    expect(toggleTask(t, 2)).toContain("> - [x] 三");
    expect(toggleTask(t, 3)).toBeNull();
    expect(toggleTask(t, 0)).toContain("- [ ] 程式碼");
  });
});

describe("拖檔進編輯區：插入連結", () => {
  it("relPath：同資料夾、往下、往上", () => {
    expect(relPath("docs/a.md", "docs/b.md")).toBe("b.md");
    expect(relPath("README.md", "docs/guide.md")).toBe("docs/guide.md");
    expect(relPath("docs/ideation/x.md", "docs/VISION.md")).toBe("../VISION.md");
    expect(relPath("a/b/c.md", "x/y.png")).toBe("../../x/y.png");
  });
  it("linkForDrop：同 repo 相對連結、圖片用 ![]()、別的 repo 寫行內碼", () => {
    expect(linkForDrop("a", "docs/a.md", { repo: "a", path: "docs/my file.md" })).toBe(
      "[my file.md](my%20file.md)",
    );
    expect(linkForDrop("a", "README.md", { repo: "a", path: "img/logo.png" })).toBe(
      "![logo.png](img/logo.png)",
    );
    expect(linkForDrop("a", "README.md", { repo: "b", path: "x.md" })).toBe("`b/x.md`");
  });
});

describe("slug：標題錨點", () => {
  it("小寫、拿掉標點、空白換 -、中文照留", () => {
    expect(slug("3. 問題清單與優先序")).toBe("3-問題清單與優先序");
    expect(slug("Hello, World!")).toBe("hello-world");
    expect(slug("UX-01／P1")).toBe("ux-01p1");
  });
});

describe("編輯中的大綱與字數", () => {
  it("mdHeadings 跳過程式碼區塊；lineOffset 找行首；textStats 不算空白", () => {
    const t = "# 一\n內容\n```\n# 不算\n```\n## 二 ##\n";
    expect(mdHeadings(t)).toEqual([
      { level: 1, text: "一", line: 0 },
      { level: 2, text: "二", line: 5 },
    ]);
    expect(lineOffset(t, 5)).toBe(t.indexOf("## 二"));
    expect(textStats("中文 ab\n第二行\n")).toEqual({ chars: 7, lines: 2 });
  });
});
