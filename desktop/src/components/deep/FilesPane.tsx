import { useState } from "react";
import { DetailLayout, useAutoFocus, useDetail } from "../../dialogs/Modal";
import { Onboarding } from "../../dialogs/misc";
import { scopeLabel } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";
import { RepoRoot } from "./FileTree";
import { addFolder, removeFromScope } from "./repoActions";

/** 沒有原生選資料夾（瀏覽器版）時：手打路徑。 */
function AddFolderDialog() {
  const store = useStore();
  const { close } = useDetail();
  const ref = useAutoFocus<HTMLInputElement>();
  const [path, setPath] = useState("");
  return (
    <DetailLayout
      actions={
        <AsyncButton
          className="primary"
          disabled={!path.trim()}
          run={async () => {
            await addFolder(store, path.trim());
            close();
          }}
        >
          加入
        </AsyncButton>
      }
    >
      <label className="field">
        repo 資料夾的路徑
        <input
          ref={ref}
          value={path}
          placeholder="例如 /Users/you/Coding/my-repo"
          onChange={(e) => setPath(e.target.value)}
        />
      </label>
    </DetailLayout>
  );
}

/** 檔案分頁：目前範圍的每個 repo 是一個根目錄（像 IDE 的多根工作區）。 */
export function FilesPane({ query, active }: { query: string; active: boolean }) {
  const store = useStore();
  const ui = useUi();
  const { scope, state, stateReady } = useSnapshot();
  const repos = store.scopeRepos.filter((r) => !query || r.toLocaleLowerCase().includes(query));
  const group = scope.group ? state.groups.find((g) => g.name === scope.group) : undefined;
  const adhoc = !scope.group && Array.isArray(scope.repos);

  return (
    <section id="tab-files" className={"tabpane" + (active ? " on" : "")} aria-label="檔案">
      <div className="sectionintro">
        <div>
          <h2>檔案</h2>
          <p className="muted small">
            「{scopeLabel(scope)}」的 repo 都在這裡，像 IDE
            的工作區資料夾；每一列都能複製絕對或相對路徑。
          </p>
        </div>
        <div className="head-actions">
          <AsyncButton
            id="addfolder"
            className="primary small"
            disabled={!stateReady}
            onError={(m) => store.toast(m)}
            run={async () => {
              if (!store.backend.pickFolder) {
                ui.openDetail({
                  title: "加入資料夾",
                  eyebrow: "ADD FOLDER",
                  content: <AddFolderDialog />,
                });
                return;
              }
              const path = await store.backend.pickFolder();
              if (path) await addFolder(store, path);
            }}
          >
            ＋ 加入資料夾
          </AsyncButton>
          <button
            type="button"
            className="small"
            disabled={!stateReady}
            onClick={() =>
              ui.openDetail({
                title: "掃描資料夾，一次加入多個 repo",
                eyebrow: "ADD REPOS",
                content: <Onboarding mode="add" />,
              })
            }
          >
            掃描資料夾…
          </button>
        </div>
      </div>
      {!repos.length ? (
        <div className="emptyline">
          {!stateReady
            ? "正在載入…"
            : query
              ? "沒有符合條件的 repo"
              : "這個範圍還沒有 repo。按「＋ 加入資料夾」加一個。"}
        </div>
      ) : (
        <div className="filetree">
          {repos.map((id) => (
            <RepoRoot
              key={id}
              id={id}
              defaultOpen={repos.length === 1}
              onRootMenu={(e) => {
                e.preventDefault();
                ui.repoMenu(id, e.clientX, e.clientY);
              }}
              actions={
                group || adhoc ? (
                  <AsyncButton
                    className="ghost small"
                    title={"從這組移除 " + id + "（登記保留，檔案不動）"}
                    onError={(m) => store.toast(m)}
                    run={() => removeFromScope(store, id)}
                  >
                    從這組移除
                  </AsyncButton>
                ) : null
              }
            />
          ))}
        </div>
      )}
    </section>
  );
}
