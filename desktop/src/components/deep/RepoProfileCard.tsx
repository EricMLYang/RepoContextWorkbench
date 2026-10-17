import { useEffect, useState } from "react";
import type { RepoProfile } from "../../api/types";
import { useSnapshot, useStore } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/**
 * 右側欄的 repo 名片（F-57）：一句話介紹、協作角色、搜尋關鍵字——同組的 agent 開場就讀這張。
 * 存在 spine 的 registry，不寫進 repo。沒填介紹時顯示從 README 截的推測。
 * 引擎每次 state 變動都重讀一次（agent 可能剛用 describe_repo 改過）；讀失敗保留上次的。
 */
export function RepoProfileCard({ id }: { id: string }) {
  const store = useStore();
  const { state } = useSnapshot();
  const [profile, setProfile] = useState<RepoProfile | null>(null);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    let live = true;
    store
      .action("repo_profile", { id })
      .then((p) => {
        if (!live) return;
        setProfile(p);
        setError("");
      })
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [store, id, state]);

  const shown = profile?.id === id ? profile : null;
  return (
    <section className="panelsection profilecard" aria-label={id + " 的名片"}>
      <div className="profilehead">
        <p className="eyebrow">名片</p>
        {shown?.role ? <span className="pill">{shown.role}</span> : null}
        <span className="spacer" />
        {shown && !editing ? (
          <button
            type="button"
            className="ghost small"
            title="介紹、角色、關鍵字存在 spine，不寫進 repo"
            onClick={() => setEditing(true)}
          >
            {shown.description_source === "registry" || shown.role ? "編輯" : "寫名片"}
          </button>
        ) : null}
      </div>
      {error ? (
        <p className="error small" role="alert">
          讀不到名片：{error}
        </p>
      ) : null}
      {!shown ? (
        error ? null : (
          <p className="muted small panelnote">正在讀名片…</p>
        )
      ) : editing ? (
        <ProfileForm
          profile={shown}
          onDone={(p) => {
            if (p) {
              setProfile(p);
              store.toast(p.changed ? "名片已儲存" : "名片沒有變動");
            }
            setEditing(false);
          }}
        />
      ) : (
        <>
          <ProfileView profile={shown} />
          <ClassPicker profile={shown} onSaved={setProfile} />
        </>
      )}
    </section>
  );
}

function ProfileView({ profile }: { profile: RepoProfile }) {
  const guessed = profile.description_source === "readme";
  return (
    <>
      {profile.description ? (
        <p className={"profiledesc" + (guessed ? " guessed" : "")}>
          {profile.description}
          {guessed ? <span className="muted small">（從 README 推測，還沒寫介紹）</span> : null}
        </p>
      ) : (
        <p className="muted small panelnote">
          還沒有介紹。寫一句「這個 repo 是什麼、別的 repo 來這裡找什麼」，同組的 agent
          開場就看得到。
        </p>
      )}
      {profile.keywords?.length ? (
        <ul className="profilekw" aria-label="關鍵字">
          {profile.keywords.map((k) => (
            <li key={k}>{k}</li>
          ))}
        </ul>
      ) : null}
    </>
  );
}

/**
 * 類別（2026-10-18）：決定工作台與 agent 怎麼對待這個 repo（開場告訴 agent 這類的規則；參考類不採集、不搜程式碼）。
 * 選了就存（走引擎 repo_class，留痕）；存的時候鎖住，失敗把錯誤顯示出來、選單回到原值。
 */
function ClassPicker(props: { profile: RepoProfile; onSaved: (p: RepoProfile) => void }) {
  const { profile, onSaved } = props;
  const store = useStore();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  if (!profile.classes?.length) return null;
  const current = profile.class?.id ?? "";
  return (
    <div className="profileclass">
      <label className="field inline">
        類別
        <select
          value={current}
          disabled={busy}
          aria-describedby={profile.class ? "classrule-" + profile.id : undefined}
          onChange={async (e) => {
            const next = e.target.value;
            setBusy(true);
            setError("");
            try {
              const p = await store.action("repo_class", { id: profile.id, class: next });
              onSaved(p);
              store.toast(
                p.class ? `${profile.id} 類別：${p.class.label}` : `${profile.id} 改回未分類`,
              );
            } catch (err) {
              setError((err as Error).message);
            } finally {
              setBusy(false);
            }
          }}
        >
          <option value="">未分類（照程式對待）</option>
          {profile.classes.map((c) => (
            <option key={c.id} value={c.id} title={"怎樣算可靠：" + c.reliable}>
              {c.label}
            </option>
          ))}
        </select>
      </label>
      {profile.class ? (
        <p id={"classrule-" + profile.id} className="muted small panelnote">
          {"給 agent 的規則：" + profile.class.rule}
        </p>
      ) : null}
      {error ? (
        <p className="error small" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

function ProfileForm(props: {
  profile: RepoProfile;
  onDone: (p: (RepoProfile & { changed: boolean }) | null) => void;
}) {
  const { profile, onDone } = props;
  const store = useStore();
  const guess = profile.description_source === "readme" ? profile.description || "" : "";
  const [description, setDescription] = useState(
    profile.description_source === "registry" ? profile.description || "" : "",
  );
  const [role, setRole] = useState(profile.role || "");
  const [keywords, setKeywords] = useState((profile.keywords || []).join(", "));
  const [error, setError] = useState("");
  return (
    <div className="profileform">
      <label className="field">
        介紹
        <textarea
          rows={3}
          value={description}
          placeholder={guess ? "推測：" + guess : "這個 repo 是什麼、別的 repo 來這裡找什麼"}
          onChange={(e) => setDescription(e.target.value)}
        />
      </label>
      {guess && !description ? (
        <button type="button" className="ghost small" onClick={() => setDescription(guess)}>
          用推測的介紹改
        </button>
      ) : null}
      <label className="field">
        角色（一個短詞，例：知識庫、規格來源、實作）
        <input value={role} onChange={(e) => setRole(e.target.value)} />
      </label>
      <label className="field">
        關鍵字（逗號或頓號分隔）
        <input value={keywords} onChange={(e) => setKeywords(e.target.value)} />
      </label>
      {error ? (
        <p className="error small" role="alert">
          {error}
        </p>
      ) : null}
      <div className="panelactions">
        <button type="button" className="small" onClick={() => onDone(null)}>
          取消
        </button>
        <AsyncButton
          className="small primary"
          onError={setError}
          run={async () => {
            setError("");
            const p = await store.action("describe_repo", {
              id: profile.id,
              description,
              role,
              keywords,
            });
            onDone(p);
          }}
        >
          儲存名片
        </AsyncButton>
      </div>
    </div>
  );
}
