import { describe, expect, it } from "vitest";
import { continueList, indent, toggleLinePrefix, toggleTask, wrap } from "../lib/mdedit";

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
