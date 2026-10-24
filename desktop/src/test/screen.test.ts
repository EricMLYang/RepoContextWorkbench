import { afterEach, describe, expect, it, vi } from "vitest";
import { ScreenReporter } from "../store/screen";
import { WorkbenchStore } from "../store/store";
import { mockBackend } from "./fixtures";

function setup() {
  vi.useFakeTimers();
  const backend = mockBackend();
  const store = new WorkbenchStore(backend, { group: "產品", repos: null });
  const r = new ScreenReporter(store);
  const sent = () =>
    (backend.action as ReturnType<typeof vi.fn>).mock.calls.filter((c) => c[0] === "screen");
  return { r, sent };
}

describe("回報畫面（agent 被問「根據畫面」才讀）", () => {
  afterEach(() => vi.useRealTimers());

  it("停下來才送一次；跟上次一樣不送", async () => {
    const { r, sent } = setup();
    r.setBase({ file: null, tab: "inbox", panel: null });
    r.setBase({ file: null, tab: "notes", panel: { repo: "impl", tab: "changes" } });
    await vi.advanceTimersByTimeAsync(600);
    expect(sent()).toEqual([
      [
        "screen",
        {
          view: { kind: "workspace", tab: "notes" },
          group: "產品",
          panel: { repo: "impl", tab: "changes" },
        },
      ],
    ]);
    r.schedule();
    await vi.advanceTimersByTimeAsync(600);
    expect(sent()).toHaveLength(1);
  });

  it("開著檔：用那個分頁量的細節；分頁收起後只報看法", async () => {
    const { r, sent } = setup();
    const file = { repo: "impl", path: "src/app.rs" };
    r.setDetail("impl", "src/app.rs", () => ({ mode: "source", lines: [10, 42] }));
    r.setBase({ file, tab: "inbox", panel: null });
    await vi.advanceTimersByTimeAsync(600);
    expect(sent().at(-1)?.[1]).toEqual({
      view: { kind: "file", ...file, mode: "source", lines: [10, 42] },
      group: "產品",
    });
    // 別的檔的量法不會套到這個檔
    r.setDetail("impl", "src/app.rs", null);
    r.setDetail("impl", "other.rs", () => ({ mode: "edit", unsaved: true }));
    await vi.advanceTimersByTimeAsync(600);
    expect(sent().at(-1)?.[1]).toEqual({
      view: { kind: "file", ...file, mode: "view" },
      group: "產品",
    });
  });
});
