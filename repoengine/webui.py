"""S4 監控台雛形（P14-lite）＋碰撞輸入框（S2-lite）＋組切換（S3-lite）。

可拋棄殼的最小網頁版（v2 §9-8：耗材原則下最快拼出「事件列＋按鈕」的形態）：
stdlib http.server 綁 127.0.0.1，零新依賴；瀏覽器就是視窗。

殼原則（v2 §1）在這裡的落實：
- /api/state  輕量輪詢（純脊椎/registry 檔案讀）＝「殼只 poll 脊椎未讀」
- /api/scan   才跑 P3 採集（開頁/切組/手動），高頻輪詢不打 git subprocess
- 所有行動按鈕都只是呼叫引擎原語再落脊椎；UI 顯示的一切內容都來自脊椎
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
        "groups": [g["name"] for g in data["groups"]],
        "unread": [_ev_dict(e) for e in _spine.get_unread(spine_dir)],
        "loops": [_ev_dict(e) for e in _spine.open_loops(spine_dir)],
        "stats": _spine.stats(spine_dir),
    }


def build_scan(spine_dir, group=None):
    """P3 採集＋閾值問句＋audit（開頁/切組/手動才跑，因為會打 git）。"""
    cfg = _config.load(spine_dir)
    if group:
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
        group = payload.get("group") or None
        cid = _collide.submit(spine_dir, idea, group=group, source="monitor")
        if payload.get("wait"):
            j = _collide.run_judgement(spine_dir, cid, group=group)
            return {"ok": True, "cid": cid, "judgement": j}
        _collide.spawn_detached(spine_dir, cid, group=group)
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
                    self._json(build_scan(spine_dir, (q.get("group") or [None])[0]))
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
        --accent:#3b6ea5;--warn:#a5581a;--red:#a52a2a;--ok:#3d7a4f;}
  *{box-sizing:border-box}
  body{margin:0;background:var(--bg);color:var(--ink);
       font:14px/1.6 "Segoe UI","Microsoft JhengHei",system-ui,sans-serif}
  header{display:flex;align-items:center;gap:12px;padding:10px 18px;
         background:var(--card);border-bottom:1px solid var(--line);
         position:sticky;top:0;flex-wrap:wrap}
  header h1{font-size:16px;margin:0;font-weight:600}
  #badge{background:var(--red);color:#fff;border-radius:10px;padding:0 8px;
         font-size:12px;line-height:20px;display:none}
  select,button,input{font:inherit;border:1px solid var(--line);border-radius:6px;
         background:var(--card);color:var(--ink);padding:4px 10px}
  button{cursor:pointer}
  button:hover{border-color:var(--accent);color:var(--accent)}
  button.small{font-size:12px;padding:2px 8px}
  main{max-width:880px;margin:0 auto;padding:14px 18px 60px}
  section{background:var(--card);border:1px solid var(--line);border-radius:8px;
          padding:12px 16px;margin-top:14px}
  section h2{font-size:13px;margin:0 0 8px;color:var(--sub);font-weight:600;
             letter-spacing:.05em}
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
         opacity:0;transition:opacity .3s;pointer-events:none;max-width:80%}
  table{border-collapse:collapse;width:100%;font-size:13px}
  th,td{text-align:left;padding:4px 10px 4px 0;border-bottom:1px solid var(--line)}
  th{color:var(--sub);font-weight:600}
  .num{text-align:right}
  #deep{display:none}
  .statsline{color:var(--sub);font-size:12px;margin-top:8px}
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
    <h2>碰撞台（送出即關，判定走未讀回程）</h2>
    <div id="collidebox">
      <input id="idea" placeholder="想法丟進來，Enter 送出">
      <button id="collidebtn">丟進碰撞</button>
    </div>
  </section>
  <div id="allclear" hidden>✅ 一切正常。</div>
  <section id="qsec" hidden>
    <h2>需要你判斷的</h2><div id="questions"></div>
  </section>
  <section id="usec" hidden>
    <h2>未讀事件</h2><div id="unread"></div>
  </section>
  <section id="lsec" hidden>
    <h2>未結 open loops</h2><div id="loops"></div>
  </section>
  <section id="deep">
    <h2>深看（全量）</h2>
    <div id="deeptable"></div>
    <div id="auditbox"></div>
    <div class="statsline" id="stats"></div>
  </section>
</main>
<div id="toast"></div>
<script>
const $=id=>document.getElementById(id);
let curGroup=localStorage.getItem("group")||"";
let lastScan=null;

function toast(msg){const t=$("toast");t.textContent=msg;t.style.opacity=1;
  setTimeout(()=>t.style.opacity=0,2500);}

async function api(path,opts){const r=await fetch(path,opts);
  const j=await r.json();
  if(j.error){toast("錯誤："+j.error);throw new Error(j.error);}return j;}

function el(tag,cls,text){const e=document.createElement(tag);
  if(cls)e.className=cls;if(text!==undefined)e.textContent=text;return e;}

function renderState(st){
  const n=st.unread.length;
  $("badge").style.display=n?"inline-block":"none";
  $("badge").textContent=n?("未讀 "+n):"";
  // 組選單
  const sel=$("group");
  if(sel.options.length!==st.groups.length+1){
    sel.innerHTML="";sel.append(new Option("（全部）",""));
    st.groups.forEach(g=>sel.append(new Option(g,g)));
    sel.value=curGroup;
  }
  // 未讀事件列
  const u=$("unread");u.innerHTML="";
  st.unread.forEach(ev=>{
    const row=el("div","row");
    row.append(el("span","meta",ev.date+" "+ev.time));
    row.append(el("span","tag",ev.type+" ["+ev.source+"]"));
    const b=el("div","body",ev.body||ev.tokens.join(" "));
    row.append(b);
    const btn=el("button","small","忽略並記錄");
    btn.onclick=async()=>{await api("/api/ignore",post({type:ev.type,time:ev.time,header:ev.header}));
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

function renderScan(sc){
  lastScan=sc;
  $("scantime").textContent="採集於 "+new Date().toLocaleTimeString();
  const q=$("questions");q.innerHTML="";
  sc.questions.forEach((t,i)=>{const row=el("div","row");
    row.append(el("span","meta",(i+1)+"."));
    row.append(el("div","body warn",t));q.append(row);});
  $("qsec").hidden=!sc.questions.length;
  // 深看表格
  const d=$("deeptable");d.innerHTML="";
  const tb=el("table");
  tb.innerHTML="<tr><th>repo</th><th>tier</th><th class=num>dirty</th>"+
    "<th class=num>dirty天</th><th class=num>末commit天</th><th>note</th></tr>";
  sc.states.forEach(s=>{
    const tr=el("tr");
    tr.append(el("td","",s.id));tr.append(el("td","",s.tier));
    const f=(v,dec)=>v==null?"-":(dec?v.toFixed(1):v);
    [["td num",s.dirty],["td num",f(s.dirty_days,1)],["td num",f(s.last_commit_days,1)]]
      .forEach(([c,v])=>{const td=el("td","num",String(v));tr.append(td);});
    tr.append(el("td","",s.note||""));tb.append(tr);
  });
  d.append(tb);
  const a=$("auditbox");a.innerHTML="";
  sc.audit.forEach(t=>a.append(el("div","red","🔴 "+t)));
  if(!sc.audit.length)a.append(el("div","","audit 零紅字 ✅"));
  updateAllClear();
}

function updateAllClear(){
  const clear=$("qsec").hidden&&$("usec").hidden&&$("lsec").hidden;
  $("allclear").hidden=!clear;
}

const post=o=>({method:"POST",headers:{"Content-Type":"application/json"},
  body:JSON.stringify(o)});

async function refreshState(){try{renderState(await api("/api/state"));}catch(e){}}
async function rescan(){try{
  renderScan(await api("/api/scan?group="+encodeURIComponent(curGroup)));}catch(e){}}

$("group").onchange=e=>{curGroup=e.target.value;
  try{localStorage.setItem("group",curGroup);}catch(err){}rescan();};
$("rescan").onclick=rescan;
$("deepbtn").onclick=()=>{const d=$("deep");
  d.style.display=d.style.display==="block"?"none":"block";};
$("ackbtn").onclick=async()=>{await api("/api/ack",post({}));
  toast("已全部標記已讀");refreshState();};
async function submitIdea(){
  const v=$("idea").value.trim();if(!v)return;
  $("idea").value="";
  const r=await api("/api/collide",post({idea:v,group:curGroup||null}));
  toast("已丟進碰撞 "+r.cid+"（判定回來會出現在未讀）");
  refreshState();
}
$("collidebtn").onclick=submitIdea;
$("idea").addEventListener("keydown",e=>{if(e.key==="Enter")submitIdea();});

refreshState();rescan();
setInterval(refreshState,5000);   // 殼只 poll 脊椎：輕量輪詢
setInterval(rescan,120000);       // P3 採集低頻；手動按鈕隨時可補
</script></body></html>
"""
