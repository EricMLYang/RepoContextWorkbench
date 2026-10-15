import { describe, expect, it } from "vitest";
import { anchorNotes, diffLineRefs } from "../lib/view";

const diff = [
  "diff --git a/x.rs b/x.rs",
  "--- a/x.rs",
  "+++ b/x.rs",
  "@@ -3,3 +3,4 @@ fn a()",
  " keep",
  "-old",
  "+new1",
  "+new2",
  "\\ No newline at end of file",
  "@@ -20,1 +21,1 @@",
  " tail",
];

describe("差異逐行留言（F-75）", () => {
  it("照 hunk 標頭數出每行的位置；標頭與 hunk 行不能留言", () => {
    expect(diffLineRefs(diff)).toEqual([
      null,
      null,
      null,
      null,
      "第 3 行",
      "原第 4 行（刪除）",
      "第 4 行（新增）",
      "第 5 行（新增）",
      null,
      null,
      "第 21 行",
    ]);
  });
  it("差異重讀後照內容找回留言的行；不在了＝-1", () => {
    const refs = diffLineRefs(diff);
    const notes = [
      { where: "第 4 行（新增）", code: "+new1", note: "a" },
      { where: "第 9 行（新增）", code: "+new2", note: "位置變了、內容還在" },
      { where: "第 1 行", code: "+gone", note: "c" },
    ];
    expect(anchorNotes(notes, diff, refs)).toEqual([6, 7, -1]);
  });
});
