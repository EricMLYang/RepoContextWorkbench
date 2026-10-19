import { beforeEach, describe, expect, it } from "vitest";
import { imeGuard, imeKey } from "../lib/ime";

const key = (k: string, isComposing = false) => ({
  key: k,
  // 跟真的事件同一個時間基準（jsdom 的 Event.timeStamp 不一定是 performance.now()）
  timeStamp: new KeyboardEvent("keydown", { key: k }).timeStamp,
  nativeEvent: { isComposing },
});

describe("imeKey：組字中一律略過；WebKit 選字確認的 Enter 只略過一次", () => {
  beforeEach(() => {
    imeGuard.webkit = true;
  });
  it("isComposing 就是輸入法的", () => {
    expect(imeKey(key("Enter", true))).toBe(true);
    expect(imeKey(key("a", true))).toBe(true);
  });
  it("compositionend 之後緊接的 Enter 當作確認選字，再按一次才是真的 Enter", () => {
    window.dispatchEvent(new CompositionEvent("compositionend"));
    expect(imeKey(key("Enter"))).toBe(true);
    expect(imeKey(key("Enter"))).toBe(false);
  });
  it("compositionend 之後按別的鍵不受影響；Chrome 系不做這個判斷", () => {
    window.dispatchEvent(new CompositionEvent("compositionend"));
    expect(imeKey(key("a"))).toBe(false);
    imeGuard.webkit = false;
    window.dispatchEvent(new CompositionEvent("compositionend"));
    expect(imeKey(key("Enter"))).toBe(false);
  });
});
