/**
 * CSV／TSV 預覽（F-110）：照 RFC 4180 拆欄——雙引號包住的欄位可以有分隔符號、換行，`""` 是一個引號。
 * 只拆前 `maxRows` 列（大檔先看頭），回傳有沒有截掉。
 */
export function parseDelimited(
  text: string,
  delim: string,
  maxRows = 1000,
): { rows: string[][]; more: boolean } {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;
  let i = 0;
  const s = text.replace(/^\uFEFF/, "");
  const endRow = () => {
    row.push(cell);
    cell = "";
    // 檔尾的空行不算一列
    if (!(row.length === 1 && row[0] === "")) rows.push(row);
    row = [];
  };
  while (i < s.length) {
    const c = s[i];
    if (quoted) {
      if (c === '"') {
        if (s[i + 1] === '"') {
          cell += '"';
          i += 2;
          continue;
        }
        quoted = false;
      } else cell += c;
      i++;
      continue;
    }
    if (c === '"' && cell === "") quoted = true;
    else if (c === delim) {
      row.push(cell);
      cell = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && s[i + 1] === "\n") i++;
      endRow();
      if (rows.length >= maxRows) return { rows, more: i + 1 < s.length };
    } else cell += c;
    i++;
  }
  if (cell !== "" || row.length) endRow();
  return { rows, more: false };
}

/** 這個檔用表格預覽嗎（副檔名），用哪個分隔符號。 */
export function tableDelimiter(path: string): string | null {
  if (/\.csv$/i.test(path)) return ",";
  if (/\.tsv$/i.test(path)) return "\t";
  return null;
}
