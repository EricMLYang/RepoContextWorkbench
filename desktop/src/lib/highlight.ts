/**
 * 輕量語法上色（檢視器的程式碼與 md 程式碼區塊）：只分註解、字串、數字、關鍵字，一行一行切，
 * 跨行的只追 C 系的區塊註解與 Python 的三引號字串。不求完整，求「一眼看得出結構」又不加依賴。
 * 純畫面：不改內容，切出來的片段串回去就是原本那一行。
 */

export type Lang =
  "rust" | "ts" | "py" | "go" | "sh" | "json" | "yaml" | "toml" | "css" | "c" | "java" | "sql";

const EXT: [RegExp, Lang][] = [
  [/\.rs$/i, "rust"],
  [/\.(tsx?|jsx?|mjs|cjs|mts|cts)$/i, "ts"],
  [/\.py$/i, "py"],
  [/\.go$/i, "go"],
  [/\.(sh|zsh|bash|fish)$/i, "sh"],
  [/\.json5?$/i, "json"],
  [/\.ya?ml$/i, "yaml"],
  [/\.toml$/i, "toml"],
  [/\.(css|scss|less)$/i, "css"],
  [/\.(c|h|cc|cpp|hpp|cs)$/i, "c"],
  [/\.(java|kt|kts|swift)$/i, "java"],
  [/\.sql$/i, "sql"],
];

/** md 程式碼區塊的語言標記也認。 */
const ALIAS: Record<string, Lang> = {
  rust: "rust",
  rs: "rust",
  ts: "ts",
  tsx: "ts",
  typescript: "ts",
  js: "ts",
  jsx: "ts",
  javascript: "ts",
  py: "py",
  python: "py",
  go: "go",
  sh: "sh",
  bash: "sh",
  zsh: "sh",
  shell: "sh",
  console: "sh",
  json: "json",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  css: "css",
  c: "c",
  cpp: "c",
  "c++": "c",
  csharp: "c",
  java: "java",
  kotlin: "java",
  swift: "java",
  sql: "sql",
};

export function langOf(pathOrTag: string | undefined): Lang | null {
  if (!pathOrTag) return null;
  const tag = ALIAS[pathOrTag.toLowerCase()];
  if (tag) return tag;
  return EXT.find(([re]) => re.test(pathOrTag))?.[1] ?? null;
}

const KW: Record<Lang, string> = {
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while Some None Ok Err",
  ts: "as async await break case catch class const continue default delete do else enum export extends false finally for from function if import in instanceof interface let new null of return static super switch this throw true try type typeof undefined var void while yield",
  py: "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield",
  go: "break case chan const continue default defer else fallthrough false for func go goto if import interface map nil package range return select struct switch true type var",
  sh: "case do done elif else esac export fi for function if in local return set then until while",
  json: "true false null",
  yaml: "true false null yes no",
  toml: "true false",
  css: "important",
  c: "auto break case char class const continue default do double else enum extern false float for goto if include int long namespace new null nullptr private protected public return short signed sizeof static struct switch template this true typedef union unsigned using virtual void volatile while",
  java: "abstract boolean break case catch class const continue default do else enum extends false final finally for fun if implements import in interface is let new null override package private protected public return static super switch this throw true try val var void when while",
  sql: "and as by create delete desc distinct drop from group having in insert into is join left limit not null on or order select set table union update values where",
};
const KW_SET: Partial<Record<Lang, Set<string>>> = {};
function keywords(lang: Lang): Set<string> {
  return (KW_SET[lang] ??= new Set(KW[lang].split(" ")));
}

const HASH_COMMENT = new Set<Lang>(["py", "sh", "yaml", "toml"]);
const SLASH_COMMENT = new Set<Lang>(["rust", "ts", "go", "c", "java", "css"]);
const CASE_INSENSITIVE = new Set<Lang>(["sql"]);

export interface Tok {
  /** "" 一般字；kw／str／com／num 上色。 */
  cls: "" | "kw" | "str" | "com" | "num";
  text: string;
}

/** 跨行狀態：在區塊註解或三引號字串裡。 */
export interface HlState {
  block: null | "*/" | '"""' | "'''";
}

/** 一行 → 片段。`state` 會被改（帶到下一行）。 */
export function tokenizeLine(line: string, lang: Lang, state: HlState): Tok[] {
  const out: Tok[] = [];
  const push = (cls: Tok["cls"], text: string) => {
    if (!text) return;
    const last = out[out.length - 1];
    if (last && last.cls === cls) last.text += text;
    else out.push({ cls, text });
  };
  let i = 0;
  // 上一行留下來的區塊
  if (state.block) {
    const end = line.indexOf(state.block);
    const cls = state.block === "*/" ? "com" : "str";
    if (end < 0) {
      push(cls, line);
      return out;
    }
    push(cls, line.slice(0, end + state.block.length));
    i = end + state.block.length;
    state.block = null;
  }
  const kw = keywords(lang);
  while (i < line.length) {
    const rest = line.slice(i);
    const c = line[i];
    // 註解
    if (
      (HASH_COMMENT.has(lang) && c === "#") ||
      (lang === "sql" && rest.startsWith("--")) ||
      (SLASH_COMMENT.has(lang) && rest.startsWith("//") && lang !== "css")
    ) {
      push("com", rest);
      break;
    }
    if ((SLASH_COMMENT.has(lang) || lang === "c") && rest.startsWith("/*")) {
      const end = rest.indexOf("*/", 2);
      if (end < 0) {
        push("com", rest);
        state.block = "*/";
        break;
      }
      push("com", rest.slice(0, end + 2));
      i += end + 2;
      continue;
    }
    // Python 三引號
    if (lang === "py" && (rest.startsWith('"""') || rest.startsWith("'''"))) {
      const q = rest.slice(0, 3) as '"""' | "'''";
      const end = rest.indexOf(q, 3);
      if (end < 0) {
        push("str", rest);
        state.block = q;
        break;
      }
      push("str", rest.slice(0, end + 3));
      i += end + 3;
      continue;
    }
    // 字串（同一行內；反斜線跳脫）
    if (c === '"' || c === "'" || (c === "`" && lang === "ts")) {
      // Rust 的生命週期 'a 不是字串
      if (c === "'" && lang === "rust" && /^'[A-Za-z_]\w*(?!')/.test(rest) && !/^'.'/.test(rest)) {
        push("", c);
        i++;
        continue;
      }
      let j = 1;
      while (j < rest.length && rest[j] !== c) j += rest[j] === "\\" ? 2 : 1;
      push("str", rest.slice(0, j + 1));
      i += j + 1;
      continue;
    }
    // 數字
    const num = /^(0x[0-9a-fA-F_]+|\d[\d_]*(\.\d+)?([eE][+-]?\d+)?)/.exec(rest);
    if (num && !/\w/.test(line[i - 1] ?? "")) {
      push("num", num[0]);
      i += num[0].length;
      continue;
    }
    // 字詞
    const word = /^[A-Za-z_$][\w$]*/.exec(rest);
    if (word) {
      const w = word[0];
      push(kw.has(CASE_INSENSITIVE.has(lang) ? w.toLowerCase() : w) ? "kw" : "", w);
      i += w.length;
      continue;
    }
    push("", c);
    i++;
  }
  return out;
}
