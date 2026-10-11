import { useState } from "react";
import type { RepoState } from "../../api/types";
import {
  CORE_COLUMNS,
  MORE_COLUMNS,
  matchesSearch,
  repoCell,
  sortRepos,
} from "../../lib/view";
import { useSnapshot, useUi } from "../../store/context";

/** Repo 分頁：6 個核心欄位，可加 5 個進階欄位；可排序（aria-sort）、首欄固定。 */
export function ReposPane({ query, active }: { query: string; active: boolean }) {
  const { scan, scanReady, scanError } = useSnapshot();
  const ui = useUi();
  const [more, setMore] = useState(false);
  const [sortColumn, setSortColumn] = useState<keyof RepoState>("id");
  const [sortDirection, setSortDirection] = useState<1 | -1>(1);
  const rows = sortRepos(
    scan.states.filter((s) => matchesSearch(s, query)),
    sortColumn,
    sortDirection,
  );
  const columns = more ? [...CORE_COLUMNS, ...MORE_COLUMNS] : CORE_COLUMNS;
  return (
    <section id="tab-repos" className={"tabpane" + (active ? " on" : "")} aria-label="Repo 狀態">
      <div className="sectionintro">
        <div>
          <h2>Repo 狀態</h2>
          <p className="muted small">先看需要關注的變化；點 repo 在右側欄看，右鍵看更多動作。</p>
        </div>
        <label className="small muted">
          <input
            id="morecols"
            type="checkbox"
            checked={more}
            onChange={(e) => setMore(e.target.checked)}
          />{" "}
          顯示進階欄位
        </label>
      </div>
      <div className="tblwrap" id="deeptable">
        {!rows.length ? (
          <div className="emptyline">
            {scanError ? "更新失敗，請重試" : !scanReady ? "等待 repo 狀態…" : "沒有符合條件的 repo"}
          </div>
        ) : (
          <table>
            <thead>
              <tr>
                {columns.map(([key, label]) => (
                  <th
                    key={key}
                    scope="col"
                    aria-sort={
                      sortColumn === key ? (sortDirection === 1 ? "ascending" : "descending") : "none"
                    }
                  >
                    <button
                      type="button"
                      className="ghost"
                      onClick={() => {
                        setSortDirection(sortColumn === key ? (-sortDirection as 1 | -1) : 1);
                        setSortColumn(key);
                      }}
                    >
                      {label + (sortColumn === key ? (sortDirection === 1 ? " ↑" : " ↓") : "")}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((s) => (
                <tr key={s.id}>
                  {columns.map(([key]) => (
                    <td key={key} title={key === "tier" && s.note ? s.note : undefined}>
                      {key === "id" ? (
                        <button
                          type="button"
                          className="ghost"
                          title={s.id + "（右鍵：更多動作）"}
                          aria-pressed={ui.panelRepo === s.id}
                          onClick={() => ui.openRepo(s.id)}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            ui.repoMenu(s.id, e.clientX, e.clientY);
                          }}
                        >
                          {s.id}
                        </button>
                      ) : (
                        repoCell(s, key)
                      )}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </section>
  );
}
