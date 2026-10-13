import { useState } from "react";
import { AsyncButton } from "./AsyncButton";

/**
 * 首次設定：桌面殼找不到 spine（`--spine`、`REPOENGINE_SPINE`、`~/.config/repo-context/config.json` 都沒有）。
 * 填路徑 → 已有的用 `set_spine`、還沒有的用 `create_spine`（只建在新路徑或空資料夾）；殼寫進使用者設定檔，下次啟動就記得。
 */
export function SpineSetup(props: {
  setSpine: (path: string) => Promise<unknown>;
  createSpine: (path: string) => Promise<unknown>;
}) {
  const [path, setPath] = useState("~/repo-spine");
  const [error, setError] = useState("");
  const p = path.trim();
  return (
    <main className="spinesetup">
      <div className="emptystate">
        <h2>選擇 spine 目錄</h2>
        <p>
          spine 是工作台自己的資料 repo：登記了哪些 repo、分組、判斷與交接都存在這裡。
          工作台只讀你的 repo，不會在裡面寫任何東西。整台電腦一個 spine 就夠了。
        </p>
        <form
          onSubmit={(e) => {
            e.preventDefault();
          }}
        >
          <input
            aria-label="spine 路徑"
            placeholder="~/repo-spine"
            value={path}
            autoFocus
            onChange={(e) => {
              setPath(e.target.value);
              setError("");
            }}
          />
          <AsyncButton
            className="primary"
            disabled={!p}
            title="在這個路徑建立新的 spine（新路徑或空資料夾）；已經是 spine 就直接用"
            run={() => props.createSpine(p)}
            onError={setError}
          >
            建立新的 spine
          </AsyncButton>
          <AsyncButton disabled={!p} run={() => props.setSpine(p)} onError={setError}>
            使用既有的 spine
          </AsyncButton>
        </form>
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
      </div>
    </main>
  );
}
