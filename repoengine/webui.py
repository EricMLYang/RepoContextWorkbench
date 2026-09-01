"""S4 監控台雛形（P14-lite）＋碰撞輸入框（S2-lite）＋組切換與臨時組合（S3/P2-lite）。

可拋棄殼的最小網頁版（v2 §9-8：耗材原則下最快拼出「事件列＋按鈕」的形態）：
stdlib http.server 綁 127.0.0.1，零新依賴；瀏覽器就是視窗。

殼原則（v2 §1）在這裡的落實：
- /api/state  輕量輪詢（純脊椎/registry 檔案讀）＝「殼只 poll 脊椎未讀」
- /api/scan   才跑 P3 採集（開頁/切組/勾選/手動），高頻輪詢不打 git subprocess
- 所有行動按鈕都只是呼叫引擎原語再落脊椎；UI 顯示的一切內容都來自脊椎

版面裁定（2026-09-01 L4 回饋）：repo 狀態表＋勾選框是**操作面板**（P2 臨時組合的介面），
放預設面不算違反「無事不報」——四判準管的是通知與事件流；勾選範圍同步過濾
「需要你判斷的」與碰撞選料，並可一鍵存成命名組。
"""
import json
import threading
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

from . import brief as _brief
from . import collect as _collect
from . import collide as _collide
from . import config as _config
from . import registry as _registry
from . import spine as _spine


def _ev_dict(ev):
    return {"date": ev.date, "time": ev.time, "type": ev.type,
            "source": ev.source, "tokens": ev.tokens, "body": ev.body,
            "header": ev.header()}


def build_state(spine_dir):
    """輕量狀態（純檔案讀，供 5 秒輪詢）。"""
    data = _registry.load(spine_dir)
    return {
        "groups": [{"name": g["name"], "members": g["members"]}
                   for g in data["groups"]],
        "repos": [r["id"] for r in data["repos"]],
        "unread": [_ev_dict(e) for e in _spine.get_unread(spine_dir)],
        "loops": [_ev_dict(e) for e in _spine.open_loops(spine_dir)],
        "stats": _spine.stats(spine_dir),
    }


def build_scan(spine_dir, group=None, repos=None):
    """P3 採集＋閾值問句＋audit（開頁/切組/勾選/手動才跑，因為會打 git）。
    repos 給定＝臨時組合（P2 免建組），優先於 group。"""
    cfg = _config.load(spine_dir)
    if repos:
        gname, entries = _registry.resolve_group(spine_dir, None, repos)
    elif group:
        gname, entries = _registry.resolve_group(spine_dir, group)
    else:
        entries = _registry.load(spine_dir)["repos"]
        gname = "全部"
    states = _collect.collect_group(entries)
    return {
        "group": gname,
        "states": states,
        "questions": _brief.build_questions(states, cfg["thresholds"]),
        "audit": _registry.audit(spine_dir),
    }


def _handle_action(spine_dir, action, payload):
    if action == "ack":
        _spine.ack_unread(spine_dir)
        return {"ok": True}
    if action == "collide":
        idea = (payload.get("idea") or "").strip()
        if not idea:
            return {"error": "想法不可為空"}
        repos = payload.get("repos") or None
        group = payload.get("group") or None
        cid = _collide.submit(spine_dir, idea, group=group, repos=repos,
                              source="monitor")
        if payload.get("wait"):
            j = _collide.run_judgement(spine_dir, cid)
            return {"ok": True, "cid": cid, "judgement": j}
        _collide.spawn_detached(spine_dir, cid)
        return {"ok": True, "cid": cid}
    if action == "ignore":
        etype, etime = payload.get("type"), payload.get("time")
        if etype not in _spine.EVENT_TYPES:
            return {"error": f"未知事件型別: {etype}"}
        _spine.append_event(spine_dir, "chosen", "monitor",
                            [f"ref:{etype}:{etime}"],
                            body=f"忽略並記錄：{payload.get('header', '')}")
        return {"ok": True}
    if action == "close_loop":
        num = payload.get("num", "")
        if not (num.startswith("#") and num[1:].isdigit()):
            return {"error": f"loop 編號不合法: {num}"}
        _spine.append_event(spine_dir, "open-loop", "monitor", [num],
                            body="closed → 從監控台關閉")
        return {"ok": True}
    if action == "save_group":
        name = (payload.get("name") or "").strip()
        members = payload.get("repos") or []
        if not name:
            return {"error": "組名不可為空"}
        if not members:
            return {"error": "至少勾選一個 repo"}
        _registry.add_group(spine_dir, name, members)
        return {"ok": True, "name": name}
    return {"error": f"未知 action: {action}"}


def _make_handler(spine_dir):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):  # 安靜，不刷終端
            pass

        def _json(self, obj, code=200):
            body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            u = urlparse(self.path)
            try:
                if u.path == "/":
                    body = PAGE.encode("utf-8")
                    self.send_response(200)
                    self.send_header("Content-Type", "text/html; charset=utf-8")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                elif u.path == "/api/state":
                    self._json(build_state(spine_dir))
                elif u.path == "/api/scan":
                    q = parse_qs(u.query)
                    self._json(build_scan(
                        spine_dir,
                        group=(q.get("group") or [None])[0],
                        repos=(q.get("repos") or [None])[0]))
                else:
                    self._json({"error": "not found"}, 404)
            except Exception as e:  # 失敗必須浮出，不准白屏
                self._json({"error": str(e)}, 500)

        def do_POST(self):
            u = urlparse(self.path)
            try:
                n = int(self.headers.get("Content-Length") or 0)
                payload = json.loads(self.rfile.read(n) or b"{}")
                if not u.path.startswith("/api/"):
                    return self._json({"error": "not found"}, 404)
                result = _handle_action(spine_dir, u.path[len("/api/"):], payload)
                self._json(result, 400 if "error" in result else 200)
            except Exception as e:
                self._json({"error": str(e)}, 500)

    return Handler


def make_server(spine_dir, port=0):
    """只綁 127.0.0.1（不變式：不對外暴露）。port=0＝隨機可用埠。"""
    return ThreadingHTTPServer(("127.0.0.1", port), _make_handler(spine_dir))


def serve(spine_dir, port=8765, open_browser=True):
    srv = make_server(spine_dir, port)
    url = f"http://127.0.0.1:{srv.server_address[1]}/"
    print(f"監控台：{url}（Ctrl+C 結束）")
    if open_browser:
        threading.Timer(0.3, lambda: webbrowser.open(url)).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("\n監控台已關閉")
    finally:
        srv.server_close()


PAGE = r"""<!doctype html>
<html lang="zh-Hant"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>監控台</title>
<style>
  :root{--bg:#f6f5f1;--card:#ffffff;--ink:#1f2328;--sub:#6a707a;--line:#e4e2db;
        --accent:#3b6ea5;--warn:#a5581a;--red:#a52a2a;--ok:#3d7a4f;--dim:#b7bcc4;}
  *{box-sizing:border-box}
  body{margin:0;background:var(--bg);color:var(--ink);
       font:14px/1.6 "Segoe UI","Microsoft JhengHei",system-ui,sans-serif}
  header{display:flex;align-items:center;gap:12px;padding:10px 18px;
         background:var(--card);border-bottom:1px solid var(--line);
         position:sticky;top:0;flex-wrap:wrap;z-index:2}
  header h1{font-size:16px;margin:0;font-weight:600}
  #badge{background:var(--red);color:#fff;border-radius:10px;padding:0 8px;
         font-size:12px;line-height:20px;display:none}
  select,button,input{font:inherit;border:1px solid var(--line);border-radius:6px;
         background:var(--card);color:var(--ink);padding:4px 10px}
  button{cursor:pointer}
  button:hover{border-color:var(--accent);color:var(--accent)}
  button.small{font-size:12px;padding:2px 8px}
  main{max-width:920px;margin:0 auto;padding:14px 18px 60px}
  section{background:var(--card);border:1px solid var(--line);border-radius:8px;
          padding:12px 16px;margin-top:14px}
  section h2{font-size:13px;margin:0 0 8px;color:var(--sub);font-weight:600;
             letter-spacing:.05em;display:flex;gap:10px;align-items:center;flex-wrap:wrap}
  .row{display:flex;gap:8px;align-items:flex-start;padding:8px 0;
       border-top:1px solid var(--line)}
  .row:first-of-type{border-top:none}
  .meta{color:var(--sub);font-size:12px;white-space:nowrap;padding-top:2px}
  .body{flex:1;white-space:pre-wrap;word-break:break-word}
  .tag{font-size:11px;border:1px solid var(--line);border-radius:4px;
       padding:0 5px;color:var(--sub)}
  #allclear{color:var(--ok);font-size:15px;padding:6px 0}
  .warn{color:var(--warn)} .red{color:var(--red)}
  #collidebox{display:flex;gap:8px}
  #collidebox input{flex:1}
  #toast{position:fixed;bottom:20px;left:50%;transform:translateX(-50%);
         background:var(--ink);color:#fff;border-radius:6px;padding:8px 16px;
         opacity:0;transition:opacity .3s;pointer-events:none;max-width:80%;z-index:3}
  table{border-collapse:collapse;width:100%;font-size:13px}
  th,td{text-align:left;padding:4px 10px 4px 0;border-bottom:1px solid var(--line)}
  th{color:var(--sub);font-weight:600;white-space:nowrap}
  td.num,th.num{text-align:right}
  tr.off td{color:var(--dim)}
  tr.badrow td{color:var(--warn)}
  #deep{display:none}
  .statsline{color:var(--sub);font-size:12px;margin-top:8px}
  .scopehint{font-weight:400;color:var(--sub)}
</style></head><body>
<header>
  <h1>監控台</h1><span id="badge"></span>
  <select id="group"></select>
  <button id="rescan" class="small">重新採集</button>
  <button id="deepbtn" class="small">深看</button>
  <button id="ackbtn" class="small">全部已讀</button>
  <span class="meta" id="scantime"></span>
</header>
<main>
  <section>
    <h2>碰撞台 <span class="scopehint" id="collidescope"></span></h2>
    <div id="collidebox">
      <input id="idea" placeholder="想法丟進來，Enter 送出（判定走未讀回程）">
      <button id="collidebtn">丟進碰撞</button>
    </div>
  </section>
  <div id="allclear" hidden>✅ 一切正常。</div>
  <section id="qsec" hidden>
    <h2>需要你判斷的</h2><div id="questions"></div>
  </section>
  <section>
    <h2>Repo 選取與狀態（勾選＝納入判斷與碰撞）
      <button id="checkall" class="small">全選</button>
      <button id="checknone" class="small">全不選</button>
      <input id="newgroup" placeholder="組名" style="width:110px" class="small">
      <button id="savegroup" class="small">勾選存成組</button>
    </h2>
    <div id="repotable"></div>
  </section>
  <section id="usec" hidden>
    <h2>未讀事件</h2><div id="unread"></div>
  </section>
  <section id="lsec" hidden>
    <h2>未結 open loops</h2><div id="loops"></div>
  </section>
  <section id="deep">
    <h2>深看（audit 與量測）</h2>
    <div id="auditbox"></div>
    <div class="statsline" id="stats"></div>
  </section>
</main>
<div id="toast"></div>
<script>
const $=id=>document.getElementById(id);
let curGroup=localStorage.getItem("group")||"";
let groupsData=[];          // [{name,members}]
let scopeRepos=[];          // 目前 scope 內的 repo id（依組）
let checked=new Set();      // 勾選中的 repo id
let lastStates=[];

function loadChecked(){
  try{const raw=localStorage.getItem("checked:"+curGroup);
    if(raw)return new Set(JSON.parse(raw));}catch(e){}
  return null;
}
function saveChecked(){
  try{localStorage.setItem("checked:"+curGroup,
    JSON.stringify([...checked]));}catch(e){}
}
function toast(msg){const t=$("toast");t.textContent=msg;t.style.opacity=1;
  setTimeout(()=>t.style.opacity=0,2500);}
async function api(path,opts){const r=await fetch(path,opts);
  const j=await r.json();
  if(j.error){toast("錯誤："+j.error);throw new Error(j.error);}return j;}
function el(tag,cls,text){const e=document.createElement(tag);
  if(cls)e.className=cls;if(text!==undefined)e.textContent=text;return e;}
const post=o=>({method:"POST",headers:{"Content-Type":"application/json"},
  body:JSON.stringify(o)});

function scopeReposFor(g){
  if(!g)return allRepos.slice();
  const hit=groupsData.find(x=>x.name===g);
  return hit?hit.members.slice():[];
}
let allRepos=[];

function selectedRepos(){return scopeRepos.filter(r=>checked.has(r));}
function scanParam(){
  const sel=selectedRepos();
  if(sel.length&&sel.length!==scopeRepos.length)
    return "repos="+encodeURIComponent(sel.join(","));
  return curGroup?("group="+encodeURIComponent(curGroup)):"";
}

function renderState(st){
  groupsData=st.groups;allRepos=st.repos;
  const n=st.unread.length;
  $("badge").style.display=n?"inline-block":"none";
  $("badge").textContent=n?("未讀 "+n):"";
  const sel=$("group");
  const want=st.groups.length+1;
  if(sel.options.length!==want){
    sel.innerHTML="";sel.append(new Option("（全部）",""));
    st.groups.forEach(g=>sel.append(new Option(g.name,g.name)));
    sel.value=curGroup;
  }
  // 未讀事件列
  const u=$("unread");u.innerHTML="";
  st.unread.forEach(ev=>{
    const row=el("div","row");
    row.append(el("span","meta",ev.date+" "+ev.time));
    row.append(el("span","tag",ev.type+" ["+ev.source+"]"));
    row.append(el("div","body",ev.body||ev.tokens.join(" ")));
    const btn=el("button","small","忽略並記錄");
    btn.onclick=async()=>{await api("/api/ignore",
      post({type:ev.type,time:ev.time,header:ev.header}));
      toast("已忽略並記錄");refreshState();};
    row.append(btn);u.append(row);
  });
  $("usec").hidden=!n;
  // open loops
  const L=$("loops");L.innerHTML="";
  st.loops.forEach(ev=>{
    const num=(ev.tokens.find(t=>t.startsWith("#"))||"#?");
    const due=(ev.tokens.find(t=>t.startsWith("due:"))||"").slice(4);
    const row=el("div","row");
    row.append(el("span","meta",num+(due?"（due "+due+"）":"")));
    row.append(el("div","body",(ev.body||"").split("\n")[0]));
    const btn=el("button","small","關閉");
    btn.onclick=async()=>{await api("/api/close_loop",post({num}));
      toast("已關閉 "+num);refreshState();};
    row.append(btn);L.append(row);
  });
  $("lsec").hidden=!st.loops.length;
  $("stats").textContent="碰撞 "+st.stats.collisions+" 次｜outcome 回連 "
    +st.stats.collisions_with_outcome+" 次｜靈感命中率 "
    +(st.stats.hit_rate==null?"n/a":Math.round(st.stats.hit_rate*100)+"%");
  updateAllClear();
}

function renderRepoTable(){
  const d=$("repotable");d.innerHTML="";
  const tb=el("table");
  const hd=el("tr");
  [["",""],["repo",""],["tier",""],["dirty","num"],["dirty天","num"],
   ["末commit天","num"],["note",""]].forEach(([t,c])=>{
    const th=el("th",c,t);hd.append(th);});
  tb.append(hd);
  const byId={};lastStates.forEach(s=>byId[s.id]=s);
  scopeRepos.forEach(id=>{
    const s=byId[id];
    const tr=el("tr",checked.has(id)?"":"off");
    const td=el("td");
    const cb=document.createElement("input");cb.type="checkbox";
    cb.checked=checked.has(id);
    cb.onchange=()=>{cb.checked?checked.add(id):checked.delete(id);
      saveChecked();renderRepoTable();updateCollideScope();rescan();};
    td.append(cb);tr.append(td);
    tr.append(el("td","",id));
    if(s){
      const bad=(s.dirty_days!=null&&s.dirty_days>=4)||!s.exists||s.note;
      if(bad&&checked.has(id))tr.classList.add("badrow");
      tr.append(el("td","",s.tier));
      const f=(v)=>v==null?"-":v.toFixed(1);
      tr.append(el("td","num",String(s.dirty)));
      tr.append(el("td","num",f(s.dirty_days)));
      tr.append(el("td","num",f(s.last_commit_days)));
      tr.append(el("td","",s.note||""));
    }else{
      tr.append(el("td","","…"));
      tr.append(el("td","num","-"));tr.append(el("td","num","-"));
      tr.append(el("td","num","-"));tr.append(el("td","","採集中"));
    }
    tb.append(tr);
  });
  d.append(tb);
}

function updateCollideScope(){
  const sel=selectedRepos();
  $("collidescope").textContent=
    sel.length===scopeRepos.length
      ?(curGroup?("組："+curGroup):"範圍：全部")
      :("臨時組合："+sel.join(", "));
}

function renderScan(sc){
  $("scantime").textContent="採集於 "+new Date().toLocaleTimeString();
  // 記住已知狀態（含未勾選 repo 的舊值）
  sc.states.forEach(s=>{
    const i=lastStates.findIndex(x=>x.id===s.id);
    if(i>=0)lastStates[i]=s;else lastStates.push(s);
  });
  const q=$("questions");q.innerHTML="";
  sc.questions.forEach((t,i)=>{const row=el("div","row");
    row.append(el("span","meta",(i+1)+"."));
    row.append(el("div","body warn",t));q.append(row);});
  $("qsec").hidden=!sc.questions.length;
  renderRepoTable();
  const a=$("auditbox");a.innerHTML="";
  sc.audit.forEach(t=>a.append(el("div","red","🔴 "+t)));
  if(!sc.audit.length)a.append(el("div","","audit 零紅字 ✅"));
  updateAllClear();
}

function updateAllClear(){
  const clear=$("qsec").hidden&&$("usec").hidden&&$("lsec").hidden;
  $("allclear").hidden=!clear;
}

async function refreshState(){try{
  renderState(await api("/api/state"));
  if(!scopeRepos.length)resetScope();
}catch(e){}}

function resetScope(){
  scopeRepos=scopeReposFor(curGroup);
  const saved=loadChecked();
  checked=saved?new Set([...saved].filter(r=>scopeRepos.includes(r)))
               :new Set(scopeRepos);
  if(!checked.size)checked=new Set(scopeRepos);
  renderRepoTable();updateCollideScope();
}

async function rescan(){try{
  const qp=scanParam();
  renderScan(await api("/api/scan"+(qp?"?"+qp:"")));
}catch(e){}}

$("group").onchange=e=>{curGroup=e.target.value;
  try{localStorage.setItem("group",curGroup);}catch(err){}
  resetScope();rescan();};
$("rescan").onclick=rescan;
$("deepbtn").onclick=()=>{const d=$("deep");
  d.style.display=d.style.display==="block"?"none":"block";};
$("ackbtn").onclick=async()=>{await api("/api/ack",post({}));
  toast("已全部標記已讀");refreshState();};
$("checkall").onclick=()=>{checked=new Set(scopeRepos);saveChecked();
  renderRepoTable();updateCollideScope();rescan();};
$("checknone").onclick=()=>{checked=new Set();saveChecked();
  renderRepoTable();updateCollideScope();};
$("savegroup").onclick=async()=>{
  const name=$("newgroup").value.trim();
  const sel=selectedRepos();
  if(!name){toast("先填組名");return;}
  const r=await api("/api/save_group",post({name,repos:sel}));
  toast("組已建："+r.name);$("newgroup").value="";refreshState();
};
async function submitIdea(){
  const v=$("idea").value.trim();if(!v)return;
  $("idea").value="";
  const sel=selectedRepos();
  const payload={idea:v};
  if(sel.length&&sel.length!==scopeRepos.length)payload.repos=sel;
  else if(curGroup)payload.group=curGroup;
  const r=await api("/api/collide",post(payload));
  toast("已丟進碰撞 "+r.cid+"（判定回來會出現在未讀）");
  refreshState();
}
$("collidebtn").onclick=submitIdea;
$("idea").addEventListener("keydown",e=>{if(e.key==="Enter")submitIdea();});

(async()=>{await refreshState();resetScope();rescan();})();
setInterval(refreshState,5000);   // 殼只 poll 脊椎：輕量輪詢
setInterval(rescan,120000);       // P3 採集低頻；手動按鈕隨時可補
</script></body></html>
"""
