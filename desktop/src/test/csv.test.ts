import { describe, expect, it } from "vitest";
import { parseDelimited, tableDelimiter } from "../lib/csv";

describe("CSV／TSV 預覽（F-110）", () => {
  it("引號裡的逗號、換行、雙引號；CRLF；BOM；檔尾空行", () => {
    const { rows, more } = parseDelimited(
      '\uFEFFname,note\r\n"王, 小明","說""好""\n第二行"\r\nb,\n\n',
      ",",
    );
    expect(rows).toEqual([
      ["name", "note"],
      ["王, 小明", '說"好"\n第二行'],
      ["b", ""],
    ]);
    expect(more).toBe(false);
  });
  it("只拆前 N 列，說有沒有截掉", () => {
    const r = parseDelimited("a\nb\nc\n", ",", 2);
    expect(r.rows).toEqual([["a"], ["b"]]);
    expect(r.more).toBe(true);
    expect(parseDelimited("a\tb", "\t").rows).toEqual([["a", "b"]]);
  });
  it("依副檔名決定分隔符號", () => {
    expect(tableDelimiter("x/data.CSV")).toBe(",");
    expect(tableDelimiter("a.tsv")).toBe("\t");
    expect(tableDelimiter("a.md")).toBeNull();
  });
});
