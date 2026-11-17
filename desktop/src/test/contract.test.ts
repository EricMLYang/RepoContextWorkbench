// 前端型別 ⇄ 引擎輸出的契約（N-52 第 3 批）。
// `get_state`／`get_scan` 在引擎裡大多是手組的 JSON，沒有 Rust 型別可以生成 TypeScript；
// 改成：引擎在固定的 testkit spine 上跑一次，輸出存成 fixtures/engine-contract.json
// （crates/ctx-core/tests/ipc_contract.rs 產生，形狀過期時那邊會失敗），這裡用 TypeScript 編譯器讀 types.ts，
// 驗宣告為必有的欄位都在、型別對；物件與陣列元素往下驗到 MAX_DEPTH 層。
// 失敗時：引擎改了欄位就改 types.ts；types.ts 寫錯了（宣告成必有、實際沒有）就改成選填。
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const MAX_DEPTH = 8;

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
const source = ts.createSourceFile(
  "types.ts",
  read("../api/types.ts"),
  ts.ScriptTarget.Latest,
  true,
);
const fixture = JSON.parse(read("./fixtures/engine-contract.json")) as Record<string, unknown>;

const interfaces = new Map<string, ts.InterfaceDeclaration[]>();
const aliases = new Map<string, ts.TypeAliasDeclaration>();
for (const st of source.statements) {
  if (ts.isInterfaceDeclaration(st))
    interfaces.set(st.name.text, [...(interfaces.get(st.name.text) ?? []), st]);
  if (ts.isTypeAliasDeclaration(st)) aliases.set(st.name.text, st);
}

type Kind = "string" | "number" | "boolean" | "null" | "array" | "object";
const kindOf = (v: unknown): Kind =>
  v === null ? "null" : Array.isArray(v) ? "array" : (typeof v as Kind);

/** 一個屬性：必有與否、型別節點。 */
interface Prop {
  optional: boolean;
  type: ts.TypeNode | undefined;
}

/** 介面的屬性（含 extends 來的）。 */
function propsOf(name: string, seen = new Set<string>()): Map<string, Prop> {
  const out = new Map<string, Prop>();
  if (seen.has(name)) return out;
  seen.add(name);
  for (const decl of interfaces.get(name) ?? []) {
    for (const h of decl.heritageClauses ?? []) {
      for (const t of h.types) {
        if (ts.isIdentifier(t.expression))
          propsOf(t.expression.text, seen).forEach((p, k) => out.set(k, p));
      }
    }
    for (const m of decl.members) {
      if (
        ts.isPropertySignature(m) &&
        m.name &&
        (ts.isIdentifier(m.name) || ts.isStringLiteral(m.name))
      ) {
        out.set(m.name.text, { optional: !!m.questionToken, type: m.type });
      }
    }
  }
  return out;
}

/** 型別節點驗一個值；回傳不合的地方（空＝合）。認不得的型別寫法一律放行。 */
function check(t: ts.TypeNode | undefined, v: unknown, at: string, depth: number): string[] {
  if (!t) return [];
  if (ts.isParenthesizedTypeNode(t)) return check(t.type, v, at, depth);
  if (ts.isUnionTypeNode(t)) {
    const tries = t.types.map((m) => check(m, v, at, depth));
    return tries.some((e) => e.length === 0) ? [] : (tries.find((e) => e.length > 0) ?? []);
  }
  if (ts.isIntersectionTypeNode(t)) {
    // `string & {}`（開放的字串聯集）→ 只看 string；物件的交集 → 每一塊都要合
    return t.types.flatMap((m) =>
      ts.isTypeLiteralNode(m) && m.members.length === 0 ? [] : check(m, v, at, depth),
    );
  }
  const want = (k: Kind) => (kindOf(v) === k ? [] : [`${at}：要 ${k}，實際是 ${kindOf(v)}`]);
  switch (t.kind) {
    case ts.SyntaxKind.StringKeyword:
      return want("string");
    case ts.SyntaxKind.NumberKeyword:
      return want("number");
    case ts.SyntaxKind.BooleanKeyword:
      return want("boolean");
    case ts.SyntaxKind.NullKeyword:
      return want("null");
  }
  if (ts.isLiteralTypeNode(t)) {
    const l = t.literal;
    if (l.kind === ts.SyntaxKind.NullKeyword) return want("null");
    if (ts.isStringLiteral(l))
      return v === l.text ? [] : [`${at}：要 "${l.text}"，實際是 ${JSON.stringify(v)}`];
    if (ts.isNumericLiteral(l)) return want("number");
    if (l.kind === ts.SyntaxKind.TrueKeyword || l.kind === ts.SyntaxKind.FalseKeyword)
      return want("boolean");
    return [];
  }
  if (ts.isArrayTypeNode(t)) {
    if (!Array.isArray(v)) return want("array");
    return depth >= MAX_DEPTH
      ? []
      : v.flatMap((x, i) => check(t.elementType, x, `${at}[${i}]`, depth + 1));
  }
  if (ts.isTypeLiteralNode(t)) {
    if (kindOf(v) !== "object") return want("object");
    const props = new Map<string, Prop>();
    for (const m of t.members) {
      if (ts.isPropertySignature(m) && m.name && ts.isIdentifier(m.name)) {
        props.set(m.name.text, { optional: !!m.questionToken, type: m.type });
      }
    }
    return checkObject(props, v as Record<string, unknown>, at, depth);
  }
  if (ts.isTypeReferenceNode(t) && ts.isIdentifier(t.typeName)) {
    const name = t.typeName.text;
    if (name === "Record") {
      if (kindOf(v) !== "object") return want("object");
      const valueType = t.typeArguments?.[1];
      return depth >= MAX_DEPTH
        ? []
        : Object.entries(v as Record<string, unknown>).flatMap(([k, x]) =>
            check(valueType, x, `${at}.${k}`, depth + 1),
          );
    }
    if (name === "Array") {
      if (!Array.isArray(v)) return want("array");
      return [];
    }
    if (interfaces.has(name)) {
      if (kindOf(v) !== "object") return want("object");
      return checkObject(propsOf(name), v as Record<string, unknown>, at, depth);
    }
    const alias = aliases.get(name);
    if (alias && !alias.typeParameters) return check(alias.type, v, at, depth);
  }
  return [];
}

function checkObject(
  props: Map<string, Prop>,
  v: Record<string, unknown>,
  at: string,
  depth: number,
): string[] {
  if (depth >= MAX_DEPTH) return [];
  const out: string[] = [];
  props.forEach((p, k) => {
    if (!(k in v)) {
      if (!p.optional) out.push(`${at}.${k}：types.ts 說必有，引擎沒給`);
      return;
    }
    out.push(...check(p.type, v[k], `${at}.${k}`, depth + 1));
  });
  return out;
}

describe("引擎輸出符合 types.ts", () => {
  it.each([
    ["get_state", "WorkbenchState"],
    ["get_scan", "ScanResult"],
  ])("%s → %s", (cmd, iface) => {
    expect(interfaces.has(iface)).toBe(true);
    const out = fixture[cmd];
    expect(kindOf(out)).toBe("object");
    expect(checkObject(propsOf(iface), out as Record<string, unknown>, cmd, 0)).toEqual([]);
  });

  it("驗得出不合（規則本身）", () => {
    const fake = { groups: "x" } as unknown as Record<string, unknown>;
    const errs = checkObject(propsOf("WorkbenchState"), fake, "s", 0);
    expect(errs.some((e) => e.startsWith("s.groups：要 array"))).toBe(true);
    expect(errs.some((e) => e.startsWith("s.repos：types.ts 說必有"))).toBe(true);
    // 往下驗：真的 fixture 改壞一個巢狀欄位也抓得到
    const st = structuredClone(fixture.get_state) as { terms: Record<string, unknown>[] };
    st.terms[0].sid = 5;
    const nested = checkObject(
      propsOf("WorkbenchState"),
      st as unknown as Record<string, unknown>,
      "s",
      0,
    );
    expect(nested).toContain("s.terms[0].sid：要 string，實際是 number");
  });
});
