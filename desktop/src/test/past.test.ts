import { describe, expect, it } from "vitest";
import { resumeCommand } from "../components/sessions/PastSessions";
import { agentKindOf } from "../components/sessions/SessionDialogs";
import { frameShown, frameTime } from "../components/sessions/Transcript";

describe("過去的會話（F-72）", () => {
  const base = {
    repo: "a",
    title: "修登入",
    prompt: null,
    reply: null,
    branch: null,
    started: null,
    updated: "2026-10-16 01:00",
    live: false,
    path: "/x.jsonl",
  };
  it("複製的接續指令用引擎組好的那一行：Claude Code 是 claude --resume、Codex 是 codex resume", () => {
    expect(
      resumeCommand({
        ...base,
        id: "02908d1e-7843",
        cwd: "/Users/me/My Repo",
        agent: "claude",
        resume: "cd '/Users/me/My Repo' && claude --resume 02908d1e-7843",
      }),
    ).toBe("cd '/Users/me/My Repo' && claude --resume 02908d1e-7843");
    expect(
      resumeCommand({
        ...base,
        id: "01a1da06-c73f",
        cwd: "/r/app",
        agent: "codex",
        resume: "cd /r/app && codex resume 01a1da06-c73f",
      }),
    ).toBe("cd /r/app && codex resume 01a1da06-c73f");
  });
  it("舊引擎沒給接續指令時退回 claude --resume（路徑有空白也安全）", () => {
    expect(
      resumeCommand({
        ...base,
        id: "02908d1e-7843",
        cwd: "/Users/me/My Repo",
        agent: "claude",
        resume: "",
      }),
    ).toBe('cd "/Users/me/My Repo" && claude --resume 02908d1e-7843');
  });
  it("agent 是哪一家以引擎給的為準；沒給就照名字認 Claude Code", () => {
    expect(agentKindOf("cx", { cx: "codex" })).toBe("codex");
    expect(agentKindOf("claude")).toBe("claude");
    expect(agentKindOf("codex")).toBe("other");
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
