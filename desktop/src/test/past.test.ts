import { describe, expect, it } from "vitest";
import { resumeCommand } from "../components/sessions/PastSessions";
import { frameShown, frameTime } from "../components/sessions/Transcript";

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

describe("逐字稿（F-94）", () => {
  it("時間：同一天只寫時分，別天帶月日；壞的或沒有就空白", () => {
    const now = new Date(2026, 9, 21, 12, 0);
    expect(frameTime(new Date(2026, 9, 21, 9, 5).toISOString(), now)).toBe("09:05");
    expect(frameTime(new Date(2026, 9, 20, 23, 7).toISOString(), now)).toBe("10/20 23:07");
    expect(frameTime(null, now)).toBe("");
    expect(frameTime("壞的", now)).toBe("");
  });
  it("篩選：思考與壓縮摘要只在「全部」", () => {
    const f = (kind: "you" | "thinking") => ({ kind, text: "", at: null });
    expect(frameShown(f("thinking"), "all")).toBe(true);
    expect(frameShown(f("thinking"), "reply")).toBe(false);
    expect(frameShown(f("you"), "you")).toBe(true);
  });
});
