import { describe, expect, it } from "vitest";
import { resumeCommand } from "../components/sessions/PastSessions";

describe("過去的會話（F-72）", () => {
  it("複製的接續指令：先進原目錄（路徑有空白也安全）再 claude --resume", () => {
    const s = {
      id: "02908d1e-7843",
      repo: "a",
      cwd: "/Users/me/My Repo",
      title: "修登入",
      prompt: null,
      reply: null,
      branch: null,
      started: null,
      updated: "2026-10-16 01:00",
      live: false,
      path: "/x.jsonl",
    };
    expect(resumeCommand(s)).toBe('cd "/Users/me/My Repo" && claude --resume 02908d1e-7843');
  });
});
