import { memo, type HTMLAttributes, type Ref } from "react";
import type { GroupInfo, RepoState, TermInfo } from "../../api/types";
import { Icon } from "../../lib/icons";
import {
  groupActivity,
  groupMeta,
  sessionInGroup,
  sessionNeedsYou,
  type Scope,
} from "../../lib/view";
import { useSortable } from "../../lib/useSortable";
import { useSnapshot, useStore, useUi, useUiState } from "../../store/context";
import { SessionRow } from "../sessions/SessionRow";

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
  onRepoMenu: (id: string, x: number, y: number) => void;
  /** 這個組的會話（執行中的一直顯示；已結束的展開才顯示）。 */
  sessions?: TermInfo[];
  /** 組員拖曳排序存檔（沒給＝不能排）。 */
  onReorder?: (order: string[]) => Promise<void>;
  /** 整張牌可拖（牌組排序）：ref 量位置、手把事件掛在標題列。 */
  drag?: { ref: Ref<HTMLDivElement>; handle: HTMLAttributes<HTMLDivElement>; dragging: boolean };
}) {
  const {
    entry: g,
    scope,
    opened,
    lastStates,
    onSelect,
    onEdit,
    onToggle,
    onRepo,
    onRepoMenu,
  } = props;
  const sort = useSortable(g.members, props.onReorder || (async () => {}));
  const members = props.onReorder ? sort.order : g.members;
  const key = g.name || "*";
  const adhoc = g.name === "__adhoc__";
  const on = adhoc
    ? Array.isArray(scope.repos)
    : !Array.isArray(scope.repos) && scope.group === g.name;
  // 只有一個 repo 的臨時範圍＝就叫那個 repo（#73）
  const title = adhoc
    ? g.members.length === 1
      ? g.members[0]
      : "臨時工作組"
    : g.name || "所有 repo";
  const sessions = props.sessions || [];
  const live = sessions.filter((s) => s.alive);
  const needs = live.filter(sessionNeedsYou).length;
  const shownSessions = opened ? sessions : live;
  return (
    <div
      className={"gcard" + (on ? " on" : "") + (props.drag?.dragging ? " dragging" : "")}
      data-key={key}
      ref={props.drag?.ref}
    >
      <div className="ghead" {...props.drag?.handle}>
        <button
          type="button"
          className="gselect"
          aria-label={"選擇牌組：" + title}
          aria-pressed={on}
          title={title + " · " + groupActivity(g.members, lastStates)}
          onClick={() => onSelect(g)}
        >
          <Icon name="stack" />
          <span className="gcopy">
            <span className="gname">{title}</span>
            <span className="gmeta">
              {groupMeta(g.members, lastStates)}
              {live.length ? " · " + live.length + " 個會話" : ""}
              {needs ? <span className="gneeds">{" · " + needs + " 個等你"}</span> : null}
            </span>
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
          ? members.map((r) => (
              <button
                key={r}
                type="button"
                className={"member ghost" + (sort.dragging === r ? " dragging" : "")}
                title={
                  r + (props.onReorder ? "（拖曳排序；右鍵：更多動作）" : "（右鍵：更多動作）")
                }
                {...(props.onReorder ? sort.rowProps(r) : {})}
                onClick={() => onRepo(r)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  onRepoMenu(r, e.clientX, e.clientY);
                }}
              >
                <Icon name="file" />
                <span>{r}</span>
              </button>
            ))
          : null}
      </div>
      {shownSessions.length ? (
        <div className="gsessions" aria-label={title + " 的會話"}>
          {shownSessions.map((s) => (
            <SessionRow key={s.sid} s={s} inGroup />
          ))}
        </div>
      ) : null}
    </div>
  );
});

/** 牌組列：列身只切組（aria-pressed），✎ 編輯，箭頭只展開（aria-expanded）；牌組與組員都能拖曳排序（Alt+↑↓）。 */
export function Deck() {
  const store = useStore();
  const ui = useUi();
  const view = useUiState();
  const { state, scope, lastStates } = useSnapshot();
  const names = state.groups.map((g) => g.name);
  const groupSort = useSortable(names, async (order) => {
    try {
      await store.action("reorder", { kind: "groups", order });
      await store.refreshState();
    } catch (e) {
      store.toast((e as Error).message);
      throw e;
    }
  });
  const reorder = (kind: "members" | "repos", group?: string) => async (order: string[]) => {
    try {
      await store.action("reorder", group ? { kind, order, group } : { kind, order });
      await store.refreshState();
    } catch (e) {
      store.toast((e as Error).message);
      throw e;
    }
  };
  const byName = new Map(state.groups.map((g) => [g.name, g]));
  const terms = state.terms || [];
  const adhoc = Array.isArray(scope.repos);
  const card = (g: DeckEntry, extra: Partial<Parameters<typeof GroupCard>[0]> = {}) => (
    <GroupCard
      key={g.name || "*"}
      entry={g}
      scope={scope}
      opened={view.openGroups.has(g.name || "*")}
      lastStates={lastStates}
      onSelect={(e) =>
        e.name === "__adhoc__" ? ui.openPicker() : store.setScope({ group: e.name, repos: null })
      }
      onEdit={(grp) => ui.openPicker(grp)}
      onToggle={ui.toggleGroup}
      onRepo={ui.openRepo}
      onRepoMenu={ui.repoMenu}
      {...extra}
    />
  );
  return (
    <div id="deck" className="deck">
      {card({ name: null, members: state.repos }, { onReorder: reorder("repos") })}
      {groupSort.order.map((name) => {
        const grp = byName.get(name);
        if (!grp) return null;
        const { ref, onPointerDown, onKeyDown, onClickCapture } = groupSort.rowProps(name);
        return card(
          { name, members: grp.members, group: grp },
          {
            sessions: terms.filter((s) => sessionInGroup(s, name)),
            onReorder: reorder("members", name),
            drag: {
              ref,
              handle: { onPointerDown, onKeyDown, onClickCapture },
              dragging: groupSort.dragging === name,
            },
          },
        );
      })}
      {adhoc
        ? card(
            { name: "__adhoc__", members: store.scopeRepos },
            {
              onReorder: async (order) => store.setScope({ group: null, repos: order }),
            },
          )
        : null}
    </div>
  );
}
