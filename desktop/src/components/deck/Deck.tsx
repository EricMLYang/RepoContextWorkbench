import { memo } from "react";
import type { GroupInfo, RepoState } from "../../api/types";
import { Icon } from "../../lib/icons";
import { groupMeta, scopeRepos, type Scope } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { repoDetailSpec } from "../deep/RepoDetail";

interface DeckEntry {
  name: string | null; // null＝所有 repo；"__adhoc__"＝臨時組
  members: string[];
  group?: GroupInfo;
}

const GroupCard = memo(function GroupCard(props: {
  entry: DeckEntry;
  scope: Scope;
  opened: boolean;
  lastStates: Record<string, RepoState>;
  onSelect: (e: DeckEntry) => void;
  onEdit: (g: GroupInfo) => void;
  onToggle: (key: string) => void;
  onRepo: (id: string) => void;
}) {
  const { entry: g, scope, opened, lastStates, onSelect, onEdit, onToggle, onRepo } = props;
  const key = g.name || "*";
  const adhoc = g.name === "__adhoc__";
  const on = adhoc ? Array.isArray(scope.repos) : !Array.isArray(scope.repos) && scope.group === g.name;
  const title = adhoc ? "臨時工作組" : g.name || "所有 repo";
  return (
    <div className={"gcard" + (on ? " on" : "")} data-key={key}>
      <div className="ghead">
        <button
          type="button"
          className="gselect"
          aria-label={"選擇牌組：" + title}
          aria-pressed={on}
          title={title}
          onClick={() => onSelect(g)}
        >
          <Icon name="stack" />
          <span className="gcopy">
            <span className="gname">{title}</span>
            <span className="gmeta">{groupMeta(g.members, lastStates)}</span>
          </span>
        </button>
        {g.group ? (
          <button
            type="button"
            className="gedit ghost"
            aria-label={"編輯牌組：" + title}
            title="編輯牌組（成員／名稱／刪除）"
            onClick={() => onEdit(g.group!)}
          >
            <Icon name="edit" />
          </button>
        ) : null}
        <button
          type="button"
          className="gexpand ghost"
          aria-label={(opened ? "收合" : "展開") + title}
          aria-expanded={opened}
          onClick={() => onToggle(key)}
        >
          {opened ? "⌄" : "›"}
        </button>
      </div>
      <div className="members" hidden={!opened}>
        {opened
          ? g.members.map((r) => (
              <button key={r} type="button" className="member ghost" title={r} onClick={() => onRepo(r)}>
                <Icon name="file" />
                <span>{r}</span>
              </button>
            ))
          : null}
      </div>
    </div>
  );
});

/** 牌組列：列身只切組（aria-pressed），✎ 編輯，箭頭只展開（aria-expanded）。 */
export function Deck() {
  const store = useStore();
  const ui = useUi();
  const { state, scope, lastStates } = useSnapshot();
  const data: DeckEntry[] = [
    { name: null, members: state.repos },
    ...state.groups.map((g) => ({ name: g.name, members: g.members, group: g })),
  ];
  if (Array.isArray(scope.repos)) data.push({ name: "__adhoc__", members: scopeRepos(scope, state) });
  return (
    <div id="deck" className="deck">
      {data.map((g) => (
        <GroupCard
          key={g.name || "*"}
          entry={g}
          scope={scope}
          opened={ui.openGroups.has(g.name || "*")}
          lastStates={lastStates}
          onSelect={(e) =>
            e.name === "__adhoc__" ? ui.openPicker() : store.setScope({ group: e.name, repos: null })
          }
          onEdit={(grp) => ui.openPicker(grp)}
          onToggle={ui.toggleGroup}
          onRepo={(id) => ui.openDetail(repoDetailSpec(id))}
        />
      ))}
    </div>
  );
}
