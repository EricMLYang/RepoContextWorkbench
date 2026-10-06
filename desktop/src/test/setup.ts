// jsdom 沒有 <dialog> 的 showModal／close：補最小行為（open 屬性＋close 事件）。
if (typeof HTMLDialogElement !== "undefined") {
  const proto = HTMLDialogElement.prototype as HTMLDialogElement & Record<string, unknown>;
  if (!proto.showModal) {
    proto.showModal = function (this: HTMLDialogElement) {
      this.setAttribute("open", "");
    };
  }
  if (!proto.close || !String(proto.close).includes("dispatch")) {
    proto.close = function (this: HTMLDialogElement) {
      if (!this.hasAttribute("open")) return;
      this.removeAttribute("open");
      this.dispatchEvent(new Event("close"));
    };
  }
}

// xterm 載入時會探測 canvas；jsdom 沒有，回 null 即可（測試不畫終端）。
if (typeof HTMLCanvasElement !== "undefined")
  HTMLCanvasElement.prototype.getContext = (() => null) as never;

if (typeof window !== "undefined" && !window.matchMedia) {
  window.matchMedia = ((q: string) => ({
    matches: false,
    media: q,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
