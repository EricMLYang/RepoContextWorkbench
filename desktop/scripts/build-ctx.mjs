// 打包前把 Rust 版 ctx 編好，放成 Tauri externalBin 要的檔名（src-tauri/binaries/ctx-<target triple>）。
// 打包後它會在 .app 的 Contents/MacOS/ctx——App 的背景碰撞判定、會話 .mcp.json 都靠 exe::ctx_exe
// 找「同目錄的 ctx」，不會落到 PATH 上的 Python 版。
import { execFileSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..", "..");
const env = { ...process.env, PATH: `${process.env.PATH}:${join(homedir(), ".cargo", "bin")}` };
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, env, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });

const triple = /^host: (\S+)$/m.exec(run("rustc", ["-vV"]))?.[1];
if (!triple) throw new Error("rustc -vV 找不到 host triple");
run("cargo", ["build", "--release", "-p", "ctx-cli"]);

const ext = process.platform === "win32" ? ".exe" : "";
const outDir = join(here, "..", "src-tauri", "binaries");
mkdirSync(outDir, { recursive: true });
const dest = join(outDir, `ctx-${triple}${ext}`);
copyFileSync(join(root, "target", "release", `ctx${ext}`), dest);
chmodSync(dest, 0o755);
console.log(`ctx → ${dest}`);
