import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Markdown } from "../lib/markdown";
import { resolveRel } from "../components/viewer/FileViewer";

afterEach(() => cleanup());

const hooks = () => ({
  onLink: vi.fn(),
  image: vi.fn((_src: string, alt: string, key: string) => <span key={key}>[img {alt}]</span>),
});

describe("Markdown → React", () => {
  it("標題、清單、引言、程式碼、表格、行內語法", () => {
    const h = hooks();
    const { container } = render(
      <Markdown
        hooks={h}
        text={[
          "---",
          "title: x",
          "---",
          "# 標題",
          "",
          "一段 **粗** 和 *斜* 還有 `code` 與 snake_case_name。",
          "",
          "- 一",
          "  - 巢狀",
          "- [x] 做完",
          "",
          "1. 甲",
          "2. 乙",
          "",
          "> 引言",
          "",
          "```rust",
          "fn main() {}",
          "```",
          "",
          "| A | B |",
          "|---|---|",
          "| 1 | 2 |",
          "",
          "[連結](docs/a.md) ![圖](logo.png)",
        ].join("\n")}
      />,
    );
    expect(container.querySelector(".mdfront")!.textContent).toBe("title: x");
    expect(container.querySelector("h1")!.textContent).toBe("標題");
    expect(container.querySelector("strong")!.textContent).toBe("粗");
    expect(container.querySelector("em")!.textContent).toBe("斜");
    expect(container.textContent).toContain("snake_case_name");
    expect(container.querySelector("ul ul li")!.textContent).toBe("巢狀");
    expect((container.querySelector("li.task input") as HTMLInputElement).checked).toBe(true);
    expect(container.querySelectorAll("ol li").length).toBe(2);
    expect(container.querySelector("blockquote")!.textContent).toBe("引言");
    expect(container.querySelector("pre.mdcode code")!.textContent).toBe("fn main() {}");
    expect(container.querySelectorAll("td").length).toBe(2);
    fireEvent.click(container.querySelector("a")!);
    expect(h.onLink).toHaveBeenCalledWith("docs/a.md");
    expect(h.image).toHaveBeenCalledWith("logo.png", "圖", expect.any(String));
  });

  it("原始 HTML 只當文字，不會變成元素", () => {
    const { container } = render(
      <Markdown
        hooks={hooks()}
        text={'<script>alert(1)</script>\n\n<img src=x onerror="alert(1)">'}
      />,
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("<script>alert(1)</script>");
  });
});

describe("resolveRel", () => {
  it("相對、上一層、repo 根目錄、跳出 repo", () => {
    expect(resolveRel("docs/guide/a.md", "b.md")).toBe("docs/guide/b.md");
    expect(resolveRel("docs/guide/a.md", "../img/x.png#y")).toBe("docs/img/x.png");
    expect(resolveRel("docs/a.md", "/README.md")).toBe("README.md");
    expect(resolveRel("a.md", "../../etc/passwd")).toBeNull();
    expect(resolveRel("a.md", "%E4%B8%AD.md")).toBe("中.md");
  });
});
