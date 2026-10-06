import { useState } from "react";
import { AsyncButton } from "./AsyncButton";

/**
 * 首次設定：桌面殼找不到 spine（`--spine`、`REPOENGINE_SPINE`、`~/.config/repo-context/config.json` 都沒有）。
 * 填路徑 → `set_spine`（殼驗證是 spine 後寫進使用者設定檔，下次啟動就記得）。
 */
export function SpineSetup({ setSpine }: { setSpine: (path: string) => Promise<unknown> }) {
  const [path, setPath] = useState("");
  const [error, setError] = useState("");
  return (
    <main className="spinesetup">
      <div className="emptystate">
        <h2>選擇 spine 目錄</h2>
        <p>
          工作台的資料（registry、脊椎事件）放在 spine repo 裡。填入它的路徑——例如
          <code> ~/spine-repo</code>；還沒有的話先在終端機跑 <code>ctx init &lt;路徑&gt;</code>。
        </p>
        <form
          onSubmit={(e) => {
            e.preventDefault();
          }}
        >
          <input
            aria-label="spine 路徑"
            placeholder="~/spine-repo"
            value={path}
            autoFocus
            onChange={(e) => {
              setPath(e.target.value);
              setError("");
            }}
          />
          <AsyncButton
            className="primary"
            disabled={!path.trim()}
            run={() => setSpine(path.trim())}
            onError={setError}
          >
            使用這個 spine
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
