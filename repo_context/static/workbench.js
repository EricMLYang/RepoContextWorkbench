/* Local workbench. UI state is separate from engine state; all writes use existing APIs. */
'use strict';
const $ = id => document.getElementById(id);
const TOKEN = new URLSearchParams(location.search).get('token') || '';
const TIERNAME={active:'進行中',paused:'已暫停',dormant:'休眠',archived:'已封存'};
const KINDNAME = {interrupt:'需要立即處理',question:'需要你判斷',collision:'分析結果',event:'其他未讀',loop:'待跟進',pending:'處理中'};
const paths = {
  stack:'M3 7l9-4 9 4-9 4-9-4m0 5 9 4 9-4M3 17l9 4 9-4',
  check:'M5 12l4 4L19 6', alert:'M12 8v5m0 3v.1M10 3 2 19h20L14 3z',
  arrow:'M5 12h14m-5-5 5 5-5 5', clock:'M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',
  file:'M14 2H5v20h14V7zm0 0v5h5M8 12h8m-8 4h8',
  chat:'M4 4h16v12H9l-5 4z', chevron:'m9 5 7 7-7 7',
  edit:'M4 20h4L19 9l-4-4L4 16zM13 7l4 4'
};
function icon(name){const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
  svg.setAttribute('viewBox','0 0 24 24');svg.setAttribute('fill','none');svg.setAttribute('stroke','currentColor');svg.setAttribute('stroke-width','1.6');svg.setAttribute('stroke-linecap','round');svg.setAttribute('stroke-linejoin','round');svg.setAttribute('aria-hidden','true');
  const p=document.createElementNS(svg.namespaceURI,'path');p.setAttribute('d',paths[name]||paths.file);svg.append(p);return svg;}
function el(tag,cls,text){const n=document.createElement(tag);if(cls)n.className=cls;if(text!=null)n.textContent=text;return n;}
function button(text,fn,cls=''){const b=el('button',cls,text);b.type='button';b.onclick=fn;return b;}
function save(key,value){try{localStorage.setItem('workbench:'+key,JSON.stringify(value));}catch{}}
function load(key,fallback){try{return JSON.parse(localStorage.getItem('workbench:'+key))??fallback;}catch{return fallback;}}
let state={groups:[],repos:[],tiers:{},tags:{},tag_vocab:[],cards:[],terms:[],agents:[],recent:[],relations:[],rel_kinds:{},stats:{}};
const EMPTY_SCAN={states:[],question_cards:[],quiet:0,audit:[],recent_commits:[],summary:null};
let scan={...EMPTY_SCAN};
let scope=load('scope',{group:null,repos:null});
if(!scope||typeof scope!=='object'||(scope.repos!==null&&!Array.isArray(scope.repos)))scope={group:null,repos:null};
let openGroups=new Set(load('openGroups',[])), lastStates={}, activeTab='inbox';
let stateReady=false, scanReady=false, stateError='', scanError='', scanLoading=false, scanVersion=0, scanAt='', stateFlight=null;
let query='', filter='all', sortColumn='id', sortDirection=1;
const busyActions=new Set(), cardNodes=new Map();
const signatures=new Map();
function changed(name,value){const sig=JSON.stringify(value);if(signatures.get(name)===sig)return false;signatures.set(name,sig);return true;}
function toast(text){$('toast').textContent=text;$('toast').style.opacity='1';clearTimeout(toast.timer);toast.timer=setTimeout(()=>$('toast').style.opacity='0',3800);}
async function api(path,payload){let r;try{r=await fetch(path,{headers:{'X-Auth':TOKEN,...(payload!==undefined?{'Content-Type':'application/json'}:{})},...(payload!==undefined?{method:'POST',body:JSON.stringify(payload)}:{})});}catch{throw new Error('無法連接工作台。請確認本機服務仍在執行，再重試。');}
  let data;try{data=await r.json();}catch{throw new Error('工作台回應不完整，請稍後重試。');}
  if(!r.ok||data.error)throw new Error(data.error||'操作未完成，請重試。');return data;}
function errorAt(id,message){$(id).textContent=message;$(id).hidden=!message;}
function scopeRepos(){if(Array.isArray(scope.repos))return scope.repos.filter(r=>state.repos.includes(r));if(scope.group)return state.groups.find(g=>g.name===scope.group)?.members.slice()||[];return state.repos.slice();}
function scopePayload(){if(Array.isArray(scope.repos))return{repos:scopeRepos()};return scope.group?{group:scope.group}:{};}
function scopeLabel(){return Array.isArray(scope.repos)?'臨時工作組':scope.group||'所有 repo';}
function scopeKey(){return JSON.stringify(scopePayload());}
function scanQuery(){const p=scopePayload();return p.repos?'?repos='+encodeURIComponent(p.repos.join(',')):p.group?'?group='+encodeURIComponent(p.group):'';}
function matchesScope(item){if(!scope.group&&!Array.isArray(scope.repos))return true;return(item.scope_repos||[]).some(r=>scopeRepos().includes(r));}
function isUnscoped(item){return['global','unclassified'].includes(item.scope_kind);}
function matchesSearch(item){return!query||JSON.stringify(item).toLocaleLowerCase().includes(query);}
function setScope(next){scope=next;save('scope',scope);scanVersion++;scanReady=false;scanError='';scan={...EMPTY_SCAN};render();rescan();}
function scopedCards(){const set=new Set(scopeRepos());return scan.question_cards.filter(c=>set.has(c.repo)).map(c=>({...c,scope_label:c.repo,scope_repos:[c.repo],scope_kind:'repo'})).concat(state.cards.filter(c=>matchesScope(c)&&!isUnscoped(c)));}
function renderConnection(){const err=stateError||scanError;$('connection').textContent=err?'連線或更新異常':scanLoading?'正在更新…':stateReady?'已連接 · 本機':'正在連接…';$('connection').classList.toggle('offline',!!err);
  $('syncerror').hidden=!err;$('syncerror').textContent=err?(err+' 目前可能顯示上次成功取得的資料。可按「更新狀態」重試。'):'';
  $('rescan').disabled=scanLoading;$('scantime').textContent=scanLoading?'更新中…':scanAt?'更新於 '+scanAt:'尚未更新';$('scanwhen').textContent=scanAt?'上次成功更新 '+scanAt:'';}
function renderHeader(){const repos=scopeRepos(), local=scopedCards();const todo=local.filter(c=>c.kind!=='pending').length;
  $('scopename').textContent=scopeLabel();$('scopekind').textContent=scope.group?'常用牌組':Array.isArray(scope.repos)?'臨時範圍':'跨組總覽';
  $('worksummary').textContent=stateReady?`${repos.length} 個 repo · 範圍內 ${todo} 件待處理 · ${local.filter(c=>c.kind==='pending').length} 件分析中`:'正在載入工作空間…';
  $('inboxcount').textContent=stateReady?todo:'—';$('scanscope').textContent='範圍：'+scopeLabel();$('ideascope').textContent='分析範圍：'+scopeLabel()+' · '+repos.length+' 個 repo';
  $('newagent').disabled=!stateReady||!repos.length;$('ideatoggle').disabled=!stateReady||!repos.length;$('briefbtn').disabled=!stateReady||!repos.length;$('ackbtn').disabled=!stateReady||!state.unread?.length;}
function renderDeck(){const data=[{name:null,members:state.repos},...state.groups];if(Array.isArray(scope.repos))data.push({name:'__adhoc__',members:scopeRepos()});
  if(!changed('deck',[data,scope,[...openGroups],lastStates]))return;
  const parent=$('deck'), existing=new Map([...parent.children].map(n=>[n.dataset.key,n]));
  data.forEach((g,i)=>{const key=g.name||'*';let n=existing.get(key);existing.delete(key);
    const sig=JSON.stringify([g,scope,[...openGroups],g.members.map(r=>lastStates[r])]);
    if(!n){n=el('div','gcard');n.dataset.key=key;}if(parent.children[i]!==n)parent.insertBefore(n,parent.children[i]||null);  // 改名後的卡留在原位
    const on=g.name==='__adhoc__'?Array.isArray(scope.repos):!Array.isArray(scope.repos)&&scope.group===g.name;
    n.classList.toggle('on',on);if(n.dataset.sig===sig)return;n.dataset.sig=sig;
    // Keep the selection and expansion buttons stable across polling.
    let head=n.querySelector('.ghead');if(!head){head=el('div','ghead');const select=button('',()=>{},'gselect');select.append(icon('stack'),el('span','gcopy'));head.append(select);if(g.name&&g.name!=='__adhoc__'){const ed=button('',()=>{},'gedit ghost');ed.append(icon('edit'));head.append(ed);}head.append(button('›',()=>{},'gexpand ghost'));n.append(head,el('div','members'));}
    const select=head.children[0], expand=head.querySelector('.gexpand'), edit=head.querySelector('.gedit'), copy=select.querySelector('.gcopy');
    const title=g.name==='__adhoc__'?'臨時工作組':g.name||'所有 repo';select.setAttribute('aria-label','選擇牌組：'+title);select.setAttribute('aria-pressed',String(on));select.title=title;
    select.onclick=()=>g.name==='__adhoc__'?openPicker():setScope({group:g.name,repos:null});
    if(edit){edit.setAttribute('aria-label','編輯牌組：'+title);edit.title='編輯牌組（成員／名稱／刪除）';edit.onclick=()=>openPicker(g);}
    const count=g.members.reduce((a,r)=>a+(lastStates[r]?.commits_7d||0),0), bad=g.members.filter(r=>lastStates[r]?.note||!lastStates[r]?.exists&&lastStates[r]).length;
    copy.replaceChildren(el('span','gname',title),el('span','gmeta',`${g.members.length} repo${g.members.length&&g.members.every(r=>lastStates[r]?.commits_7d!=null)?' · 7 天 '+count+' commits':' · 尚未完整採集'}${bad?' · '+bad+' 項需檢查':''}`));
    const opened=openGroups.has(key);expand.textContent=opened?'⌄':'›';expand.setAttribute('aria-label',(opened?'收合':'展開')+title);expand.setAttribute('aria-expanded',String(opened));
    expand.onclick=()=>{opened?openGroups.delete(key):openGroups.add(key);save('openGroups',[...openGroups]);renderDeck();};
    const members=n.querySelector('.members');members.hidden=!opened;if(opened&&changed('members:'+key,[g.members,state.tiers]))members.replaceChildren(...g.members.map(r=>{const b=button('',()=>showRepo(r),'member ghost');b.append(icon('file'),el('span','',r));b.title=r;return b;}));});
  for(const n of existing.values())n.remove();}
function actionLabel(a){if(a.action==='route_collision')return a.payload.dest==='incubator'?'移入孵化區':'存入 '+a.payload.dest.replace(/^(repo|group):/,'');
  return{route_pick:'選擇儲存位置',ignore:'忽略並記錄',collide_prefill:'分析這個問題',close_loop:'完成跟進',collide_rerun:'重新分析'}[a.action]||a.label;}
function primaryAction(c){return(c.actions||[]).find(a=>a.action==='route_collision')||(c.actions||[]).find(a=>!['ignore','defer','route_pick'].includes(a.action));}
function buildCard(c){const d=el('article','card '+c.kind);d.dataset.key=c.key;const h=el('div','cardhead'),ic=el('span','cardicon');
  ic.append(c.kind==='pending'?el('span','spin'):icon({interrupt:'alert',question:'alert',collision:'check',loop:'clock'}[c.kind]||'file'));
  const copy=el('div','cardcopy');copy.append(el('h3','ttl',c.title));const meta=el('div','meta');
  if(c.verdict)meta.append(el('span','pill '+(c.verdict==='真增量'?'green':'amber'),c.verdict));
  meta.append(el('span','',c.scope_label||c.repo||'未分類'));
  if(c.time)meta.append(el('time','',(c.date?c.date+' ':'')+c.time));if(c.due)meta.append(el('span','pill amber','到期 '+c.due));copy.append(meta);
  const desc=c.judgement?.['理由']||c.lines?.[0];if(desc&&c.kind!=='pending')copy.append(el('p','description',desc));h.append(ic,copy);d.append(h);
  const acts=el('div','acts');const primary=primaryAction(c);
  if(primary){const b=button(actionLabel(primary),()=>runAction(c,primary,d),'primary');b.disabled=busyActions.has(c.key);acts.append(b);}
  if(c.kind!=='pending')acts.append(button('查看詳情',()=>showCard(c),'ghost'));
  if((c.actions||[]).some(a=>a!==primary))acts.append(button('更多 ···',()=>showCard(c,true),'ghost more'));
  if(acts.children.length)d.append(acts);const err=el('p','error carderror');err.hidden=true;err.setAttribute('role','alert');d.append(err);return d;}
function sumItem(label,item,fallback){const row=el('div','sumrow');row.append(el('div','sumlabel',label));const copy=el('div','sumcopy');
  if(item){copy.append(el('p','sumtext',item.text),el('p','sumsrc','依據：'+item.source+(item.at?' · '+item.at:'')));}
  else copy.append(el('p','sumtext missing',fallback||'目前沒有可引用的依據。'));
  row.append(copy);return row;}
function sumChanges(s){const row=el('div','sumrow');row.append(el('div','sumlabel','變化'));const copy=el('div','sumcopy');
  copy.append(el('p','sumsrc','範圍：'+(s.changes_window||'近期')));
  if(s.changes?.length)for(const c of s.changes.slice(0,5))copy.append(el('p','sumtext',c.text),el('p','sumsrc','依據：'+c.source+(c.at?' · '+c.at:'')));
  else copy.append(el('p','sumtext missing',s.changes_gap||'本次採集沒有看到變化。'));
  row.append(copy);return row;}
function renderSummary(){const s=scan.summary,root=$('summary');
  if(!changed('summary',[s,scanReady,scanError,scanAt,scopeKey(),stateReady&&state.repos.length]))return;
  if(!stateReady||!state.repos.length){root.replaceChildren(el('p','small muted','登記 repo 後，這裡會顯示這組在做什麼、上次做到哪裡。'));return;}
  if(!s||!scanReady||scanError){root.replaceChildren(el('p','small muted',scanError?'工作摘要需要一次成功的更新才顯示；目前無法確認現況。':'正在整理本組工作摘要…'));return;}
  const head=el('div','sumhead');head.append(el('h2','','本組工作摘要'),el('span','spacer'),el('span','small muted','採集於 '+(scanAt||s.at)));
  const body=el('div','sumbody');
  body.append(sumItem('目前目標',s.goal,s.goal_gap),sumItem('上次進度',s.progress,s.progress_gap),
              sumChanges(s),sumItem('可接續的一步',s.next,s.next_gap));
  const acts=el('div','sumacts');
  acts.append(button('接續上次工作',()=>openSession({task:s.resume_task,origin:'resume:'+(s.saved_group||s.group)}),'primary'));
  if(s.next&&s.next.text.startsWith('接續 #'))acts.append(button('查看這件未結',()=>showTab('inbox'),'ghost small'));
  acts.append(button(s.goal?'修改目標':'寫下這組的目標',()=>goalDialog(s),'ghost'));
  acts.append(el('span','small muted','摘要只引用登記的目標與脊椎留痕；沒有依據的欄位會直說沒有。'));
  root.replaceChildren(head,body,acts);}
function goalDialog(s){const saved=s.saved_group;const body=showDetail(saved?'這組在做什麼？':'目標需要先有牌組','GROUP GOAL');
  if(!saved){body.append(el('p','dialogmessage',(s.goal_gap||'')+'\n\n用左欄的「＋」把目前範圍存成牌組，就能記下目標。'));
    $('detailactions').append(button('知道了',()=>$('detaildialog').close(),'primary'));return;}
  body.append(el('p','detailmeta','牌組：'+saved+'　目標只有你能寫；工作台不會用 commit 數替你推測進度。'));
  const f=el('label','field','一句話：這組現在要達成什麼？'),input=el('input');input.maxLength=300;input.value=s.goal?.text||'';f.append(input);body.append(f);
  $('detailactions').append(button('儲存目標',async e=>{const b=e.currentTarget;b.disabled=true;try{await api('/api/set_goal',{group:saved,text:input.value.trim()});$('detaildialog').close();toast('目標已儲存');await refreshState();await rescan();}catch(err){errorAt('detailerror',err.message);}finally{b.disabled=false;}},'primary'));input.focus();}
function renderCards(){const root=$('cards');const local=scopedCards();const extras=state.cards.filter(c=>isUnscoped(c)||c.kind==='interrupt'&&!matchesScope(c));
  const accept=c=>matchesSearch(c)&&(filter==='all'||c.kind===filter);const sections=[];
  const globalWarnings=extras.filter(c=>c.kind==='interrupt'&&accept(c));if(globalWarnings.length)sections.push({key:'global',title:'全域警示',note:'跨組顯示，操作只影響卡片標示的來源。',cards:globalWarnings});
  for(const kind of ['interrupt','question','collision','loop','event','pending']){const cards=local.filter(c=>c.kind===kind&&accept(c));if(cards.length)sections.push({key:kind,title:KINDNAME[kind],cards});}
  const unclassified=extras.filter(c=>c.kind!=='interrupt'&&accept(c));if(unclassified.length)sections.push({key:'unclassified',title:'全域與未分類',note:'這些事件未限定於目前牌組。',cards:unclassified,collapsed:true});
  const keep=new Set();let previous=null;
  for(const section of sections){const key='section:'+section.key;keep.add(key);let rec=cardNodes.get(key);
    if(!rec){const n=el(section.collapsed?'details':'section','cardsection');n.append(el(section.collapsed?'summary':'div','secttl'),el('p','lane-note'),el('div','sectioncards'));rec={el:n};cardNodes.set(key,rec);}
    rec.el.children[0].textContent=section.title+' · '+section.cards.length;rec.el.children[1].textContent=section.note||'';rec.el.children[1].hidden=!section.note;
    const parent=rec.el.children[2];let prev=null;
    for(const c of section.cards){keep.add(c.key);let item=cardNodes.get(c.key);const sig=JSON.stringify([c,busyActions.has(c.key)]);
      if(!item){item={el:buildCard(c),sig};cardNodes.set(c.key,item);}else if(item.sig!==sig&&!busyActions.has(c.key)){const n=buildCard(c);item.el.replaceWith(n);item={el:n,sig};cardNodes.set(c.key,item);}
      if(prev?prev.nextSibling!==item.el:parent.firstChild!==item.el)parent.insertBefore(item.el,prev?prev.nextSibling:parent.firstChild);prev=item.el;}
    if(previous?previous.nextSibling!==rec.el:root.firstChild!==rec.el)root.insertBefore(rec.el,previous?previous.nextSibling:root.firstChild);previous=rec.el;}
  for(const[key,rec]of cardNodes)if(!keep.has(key)){rec.el.remove();cardNodes.delete(key);}
  const empty=$('allclear');empty.hidden=sections.some(s=>!['global','unclassified'].includes(s.key));let title='',text='',onboard=false;
  if(!stateReady){title=stateError?'無法載入工作空間':'正在載入工作空間';text=stateError?'請確認本機服務，再按更新狀態。':'正在取得 repo 與事件。';}
  else if(!state.repos.length){title='建立你的第一個工作範圍';text='選一個放 repo 的資料夾，工作台會掃出裡面的 git repo，讓你直接建第一組；也可以用 registry add／registry scan 從命令列登記。';onboard=true;}
  else if(!scanReady||scanError){title=scanError?'無法確認目前狀態':'正在檢查這組 repo';text=scanError?'更新成功後，才能確認是否有待處理事項。':'收件匣會在採集完成後更新。';}
  else if(query||filter!=='all'){title='沒有符合條件的事項';text='試著清除搜尋或選擇其他類型。';}
  else{title='目前沒有待處理事項';text='新的分析結果與需要判斷的變化會出現在這裡。';}
  if(changed('empty',[title,text,onboard])){empty.replaceChildren(icon('check'),el('h2','',title),el('p','',text));
    if(onboard)empty.append(button('選擇資料夾，掃描 repo',openOnboarding,'primary'));}
  $('quietline').textContent=scanReady&&!scanError?`本次檢查 ${scan.states.length} 個 repo · ${scan.quiet} 個未觸發提醒條件`:'';}
function showDetail(title,eyebrow='DETAILS'){if($('detaildialog').open)$('detaildialog').close();$('detailtitle').textContent=title;$('detaileyebrow').textContent=eyebrow;$('detailbody').replaceChildren();$('detailactions').replaceChildren();errorAt('detailerror','');$('detaildialog').showModal();return $('detailbody');}
$('detailclose').onclick=()=>$('detaildialog').close();
function fullText(title,text,meta=''){const body=showDetail(title);body.append(el('p','detailmeta',meta),el('pre','',text));$('detailactions').append(button('複製內容',async()=>{try{await navigator.clipboard.writeText(text);toast('已複製內容');}catch{errorAt('detailerror','無法存取剪貼簿，可直接選取上方文字複製。');}},''),button('關閉',()=>$('detaildialog').close(),'primary'));}
function showCard(c,more=false){const body=showDetail(c.title,KINDNAME[c.kind]);body.append(el('p','detailmeta',`${c.scope_label||c.repo||'未分類'} · ${c.date||''} ${c.time||''}`));
  const text=c.judgement?Object.entries(c.judgement).map(([k,v])=>k+'：'+v).join('\n\n'):(c.lines||[]).join('\n');body.append(el('pre','',text||'選擇下方動作，繼續處理這件事。'));
  body.append(button('複製完整內容',async()=>{try{await navigator.clipboard.writeText(c.title+'\n'+text);toast('已複製完整內容與來源');}catch{errorAt('detailerror','請直接選取上方內容複製。');}},'ghost small'));
  const primary=primaryAction(c);for(const a of c.actions||[]){const b=button(actionLabel(a),()=>runAction(c,a,null,b),a===primary?'primary':a.action==='ignore'?'ghost':'');$('detailactions').append(b);}
}
async function runAction(c,a,dom,trigger){if(busyActions.has(c.key))return;
  if(a.action==='term_create'){openSession({...a.payload,...(c.scope_kind==='global'?{group:null,repos:null}:{})});return;}
  if(a.action==='route_pick'){openDestination(c);return;}
  if(a.action==='collide_prefill'){if($('idea').value.trim()&&$('idea').value!==a.payload.text){const body=showDetail('保留現有草稿？');body.append(el('p','dialogmessage','輸入框已有草稿。可將這個問題附加在草稿後，再一起分析。'));$('detailactions').append(button('附加問題',()=>{$('idea').value+='\n\n'+a.payload.text;save('draft',$('idea').value);$('detaildialog').close();showComposer();},'primary'));}else{$('idea').value=a.payload.text||'';save('draft',$('idea').value);$('detaildialog').close();showComposer();}return;}
  if(a.action==='tier'&&a.payload?.tier==='paused'&&!a.payload.resume_when){const body=showDetail('設定復工條件');const label=el('label','field','什麼條件下恢復工作？'),input=el('input');label.append(input);body.append(label);$('detailactions').append(button('暫停 repo',()=>{if(!input.value.trim()){errorAt('detailerror','請填寫復工條件。');return;}runAction(c,{...a,payload:{...a.payload,resume_when:input.value.trim()}},null);},'primary'));input.focus();return;}
  busyActions.add(c.key);const controls=dom?[...dom.querySelectorAll('button')]:[...$('detailactions').querySelectorAll('button')];controls.forEach(b=>b.disabled=true);
  const err=dom?.querySelector('.carderror');if(err)err.hidden=true;errorAt('detailerror','');
  try{const r=await api('/api/'+a.action,a.payload||{});if($('detaildialog').open)$('detaildialog').close();toast(actionLabel(a)+'：完成'+(r.file?' · '+r.file.split(/[\\/]/).pop():r.until?' · 下次提醒 '+r.until:r.due?' · '+r.due:''));await refreshState();if(['tier','defer','remove_repo'].includes(a.action))await rescan();}
  catch(e){if(err){err.textContent=e.message;err.hidden=false;}else errorAt('detailerror',e.message);}
  finally{busyActions.delete(c.key);controls.forEach(b=>b.disabled=false);renderCards();}}
function openDestination(c){const body=showDetail('選擇儲存位置','分析結果');const label=el('label','field','存入'),select=el('select');select.append(new Option('孵化區','incubator'));state.groups.forEach(g=>select.append(new Option('牌組 · '+g.name,'group:'+g.name)));state.repos.forEach(r=>select.append(new Option('Repo · '+r,'repo:'+r)));label.append(select);body.append(label);$('detailactions').append(button('儲存',()=>runAction(c,{action:'route_collision',payload:{cid:c.cid,dest:select.value}},null),'primary'));}
function renderRepos(){const rows=scan.states.filter(matchesSearch).slice().sort((a,b)=>{const x=a[sortColumn],y=b[sortColumn];return(typeof x==='number'&&typeof y==='number'?x-y:String(x??'').localeCompare(String(y??'')))*sortDirection;});
  if(!changed('repos',[rows,sortColumn,sortDirection,$('morecols').checked,scanReady,scanError]))return;const wrap=$('deeptable');wrap.replaceChildren();
  if(!rows.length){wrap.append(el('div','emptyline',scanError?'更新失敗，請重試':!scanReady?'等待 repo 狀態…':'沒有符合條件的 repo'));return;}
  const columns=[['id','Repo'],['tier','狀態'],['branch','分支'],['dirty','未提交'],['commits_7d','7 天活動'],['agents','Agent']];if($('morecols').checked)columns.push(['dirty_days','未提交天數'],['last_commit_days','距末次 commit'],['ahead','未推送'],['behind','落後'],['worktrees','Worktree']);
  const table=el('table'),thead=el('thead'),head=el('tr');for(const[key,label]of columns){const th=el('th');th.setAttribute('scope','col');th.setAttribute('aria-sort',sortColumn===key?(sortDirection===1?'ascending':'descending'):'none');th.append(button(label+(sortColumn===key?(sortDirection===1?' ↑':' ↓'):''),()=>{sortDirection=sortColumn===key?-sortDirection:1;sortColumn=key;renderRepos();},'ghost'));head.append(th);}thead.append(head);table.append(thead);const tb=el('tbody');
  for(const s of rows){const tr=el('tr');for(const[key]of columns){const td=el('td');if(key==='id'){const b=button(s.id,()=>showRepo(s.id),'ghost');b.title=s.id;td.append(b);}else if(key==='tier'){td.textContent=s.note?'需檢查':(TIERNAME[s.tier]||s.tier);if(s.note)td.title=s.note;}else if(key==='agents')td.textContent=(s.agents||[]).join('、')||'—';else td.textContent=s[key]==null?'—':typeof s[key]==='number'?String(Math.round(s[key]))+(key==='dirty'?' 檔':key==='commits_7d'?' commits':''):s[key];tr.append(td);}tb.append(tr);}table.append(tb);wrap.append(table);}
function showRepo(id){const s=lastStates[id]||{id};const body=showDetail(id,'REPO DETAILS'),dl=el('dl');for(const[label,value]of [['分支',s.branch],['狀態',TIERNAME[s.tier]||s.tier],['未提交',s.dirty==null?'尚未採集':s.dirty+' 檔'],['最近 commit',s.last_subject],['7 天 / 30 天',(s.commits_7d??'—')+' / '+(s.commits_30d??'—')],['未推送 / 落後',(s.ahead??'—')+' / '+(s.behind??'—')],['Worktree',s.worktrees],['路徑',s.path],['備註',s.note]]){const row=el('div','detailrow');row.append(el('dt','',label),el('dd','',value??'—'));dl.append(row);}body.append(dl);
  const field=el('label','field','標籤（以逗號分隔）'),input=el('input');input.value=(state.tags[id]||[]).join(', ');field.append(input);body.append(field);
  $('detailactions').append(button('儲存標籤',async e=>{const b=e.currentTarget;b.disabled=true;try{const next=[...new Set(input.value.split(/[,，]/).map(x=>x.trim()).filter(Boolean))],old=state.tags[id]||[];await api('/api/tag',{id,add:next.filter(t=>!old.includes(t)),remove:old.filter(t=>!next.includes(t))});await refreshState();toast('標籤已儲存');$('detaildialog').close();}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}}),button('開啟 Agent 會話',()=>openSession({repo:id,repos:[id],origin:'repo:'+id}),'primary'));}
function recordRow(title,meta,time,onClick){const row=el('div','record');row.append(el('time','recordtime',time));const copy=el('div','recordcopy');copy.append(button(title,onClick,'recordtitle ghost'),el('div','recordmeta',meta));row.append(copy);return row;}
function renderActivity(){const commits=(scan.recent_commits||[]).filter(matchesSearch);if(changed('commits',commits))$('recentcommits').replaceChildren(...(commits.length?commits.map(c=>recordRow(c.subject,c.repo+' · '+c.hash,c.date,()=>fullText(c.subject,c.subject,c.repo+' · '+c.hash+' · '+c.date))):[el('p','emptyline','目前範圍沒有符合條件的 commit')]));
  const p=scan.pulse;$('pulse').textContent=p?`近 7 天 ${p.commits_7d} commits · 30 天 ${p.commits_30d} commits${p.most_active?' · 最活躍 '+p.most_active:''}`:'等待採集';
  const renderEvents=(id,list)=>{if(!changed(id,list))return;$(id).replaceChildren(...(list.length?list.map(e=>recordRow(e.body.split('\n')[0]||e.type,`${e.scope_label} · ${e.type} · ${e.source}`,e.date+' '+e.time,()=>fullText(e.body.split('\n')[0]||e.type,e.body,e.date+' '+e.time+' · '+e.scope_label))):[el('p','emptyline','沒有符合條件的事件')]));};
  const local=state.recent.filter(e=>matchesScope(e)&&!isUnscoped(e)&&matchesSearch(e)),global=state.recent.filter(e=>isUnscoped(e)&&matchesSearch(e));renderEvents('recent',local);renderEvents('recentglobal',global);$('globalactivitycount').textContent=global.length;$('globalactivity').hidden=!global.length;
  const s=state.stats;$('statsraw').textContent=s.collisions?`所有組累計：${s.collisions} 次分析 · ${s.collisions_with_outcome} 次成果回連${s.spawned?' · '+s.spawned+' 個新 repo':''}`:'';}
const RELTASK={'pm-of':(a,b)=>`檢查規劃與實作是否一致：${a} 是 ${b} 的 PM。請比對 ${a} 的規劃／需求文件與 ${b} 的實作與驗收條件，列出不一致、缺漏與已過期的部分，每條標明來自哪個 repo 的哪份檔，再跟我確認要改哪一邊。`,
  'feeds':(a,b)=>`檢查供料是否還新：${a} 供料給 ${b}。請確認 ${b} 目前用的內容是不是 ${a} 的最新版本，列出落差與需要重新供料的部分，標明來源檔案。`,
  'derived-from':(a,b)=>`檢查衍生落差：${a} 衍生自 ${b}。請比對兩邊自分家後的差異，說明哪些該回流、哪些是刻意分開，標明來源檔案。`,
  'upstream-of':(a,b)=>`檢查上游影響：${a} 是 ${b} 的上游。請看 ${a} 近期變更有沒有影響 ${b}，列出需要跟進的項目與依據。`,
  'sibling-topic':(a,b)=>`比對同主題內容：${a} 與 ${b} 講同一件事。請找出重複、矛盾與只有一邊有的內容，標明來源檔案，再建議怎麼收斂。`};
function relationTask(r){return (RELTASK[r.kind]||((a,b)=>`釐清 ${a} 與 ${b} 的關係現在是否還成立：比對兩邊近期內容，說明依據，再建議要不要調整這條關係。`))(r.from,r.to);}
function renderRelations(){const members=new Set(scopeRepos());const list=state.relations.filter(r=>(members.has(r.from)||members.has(r.to))&&matchesSearch(r));if(!changed('relations',[list,state.rel_kinds]))return;
  $('reltable').replaceChildren(...(list.length?list.map(r=>{const n=el('div','relation'),copy=el('div','relationcopy'),name=el('div','relationname');name.append(document.createTextNode(r.from),el('span','',(state.rel_kinds[r.kind]?.forward||r.kind)+' →'),document.createTextNode(r.to));copy.append(name,el('p','',r.note||'尚未填寫說明'));const acts=el('div','relacts');acts.append(button('用這條關係開工作',()=>openSession({repos:[r.from,r.to],task:relationTask(r),origin:'relation:'+r.from+'>'+r.to}),'small'),button('管理',()=>manageRelation(r),'ghost small'));n.append(copy,acts);return n;}):[el('div','emptystate','還沒有符合範圍的關係。新增一條，讓組內角色更清楚。')]));}
function relationForm(){const body=showDetail('新增 Repo 關係','RELATIONSHIP');const fields={};for(const[key,title,items]of [['a','來源 repo',scopeRepos()],['kind','如何連結',Object.keys(state.rel_kinds)],['b','目標 repo',state.repos]]){const field=el('label','field',title),select=el('select');items.forEach(v=>select.append(new Option(key==='kind'?state.rel_kinds[v].forward:v,v)));fields[key]=select;field.append(select);body.append(field);}
  if(fields.a.value===fields.b.value&&fields.b.options.length>1)fields.b.selectedIndex=fields.b.selectedIndex===0?1:0;
  const field=el('label','field','說明（選填）'),note=el('input');field.append(note);body.append(field);$('detailactions').append(button('新增關係',async e=>{if(fields.a.value===fields.b.value){errorAt('detailerror','來源與目標需要是不同 repo。');return;}const b=e.currentTarget;b.disabled=true;try{await api('/api/relate',{a:fields.a.value,b:fields.b.value,kind:fields.kind.value,note:note.value.trim()});$('detaildialog').close();toast('關係已建立');await refreshState();}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}},'primary'));}
function manageRelation(r){const body=showDetail('管理關係');body.append(el('p','dialogmessage',`${r.from} ${state.rel_kinds[r.kind]?.forward||r.kind} → ${r.to}\n${r.note||''}\n\n移除只會解除這條關係，repo 與檔案會保留。`));$('detailactions').append(button('取消',()=>$('detaildialog').close()),button('移除這條關係',async e=>{const b=e.currentTarget;b.disabled=true;try{await api('/api/unrelate',{a:r.from,b:r.to,kind:r.kind});$('detaildialog').close();await refreshState();toast('關係已移除');}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}},'danger'));}
function render(){renderHeader();renderDeck();renderSummary();renderCards();renderRepos();renderActivity();renderRelations();syncTerms();renderConnection();}
async function refreshState(){if(stateFlight)return stateFlight;stateFlight=(async()=>{try{const next=await api('/api/state');state=next;stateReady=true;stateError='';
    if(scope.group&&!state.groups.some(g=>g.name===scope.group)||Array.isArray(scope.repos)&&!scope.repos.some(r=>state.repos.includes(r))){scope={group:null,repos:null};save('scope',scope);scanReady=false;scanVersion++;queueMicrotask(rescan);toast('原工作範圍已不存在，已返回所有 repo');}
    render();return true;}catch(e){stateError=e.message;renderConnection();renderCards();return false;}finally{stateFlight=null;}})();return stateFlight;}
async function rescan(){const version=++scanVersion,key=scopeKey();scanLoading=true;renderConnection();
  if(!stateReady){await refreshState();if(!stateReady){scanLoading=false;renderConnection();return;}}
  if(!scopeRepos().length){scanLoading=false;scanReady=true;scanAt='';render();return;}
  try{const result=await api('/api/scan'+scanQuery());if(version!==scanVersion||key!==scopeKey())return;scan=result;scanReady=true;scanError='';scanAt=new Date().toLocaleTimeString('zh-TW',{hour12:false,hour:'2-digit',minute:'2-digit'});for(const s of scan.states)lastStates[s.id]=s;
    if(changed('audit',scan.audit))$('auditbox').replaceChildren(...(scan.audit.length?scan.audit.map(t=>el('p','',t)):[el('p','','登記檢查通過')]));$('auditcount').textContent=scan.audit.length?' · '+scan.audit.length+' 項':' · 通過';}
  catch(e){if(version===scanVersion)scanError=e.message;}
  finally{if(version===scanVersion){scanLoading=false;render();}}}
function showTab(tab){activeTab=tab;document.querySelectorAll('#tabs button').forEach(b=>{const on=b.dataset.tab===tab;b.classList.toggle('on',on);if(on)b.setAttribute('aria-current','page');else b.removeAttribute('aria-current');});document.querySelectorAll('.tabpane').forEach(p=>p.classList.toggle('on',p.id==='tab-'+tab));$('cardfilter').hidden=tab!=='inbox';$('briefbtn').hidden=tab!=='inbox';$('ackbtn').hidden=tab!=='inbox';query='';$('search').value='';render();}
document.querySelectorAll('#tabs button').forEach(b=>b.onclick=()=>showTab(b.dataset.tab));
document.querySelector('.brand').onclick=e=>{e.preventDefault();setScope({group:null,repos:null});showTab('inbox');};
$('search').oninput=()=>{query=$('search').value.trim().toLocaleLowerCase();renderCards();renderRepos();renderActivity();renderRelations();};
$('cardfilter').onchange=()=>{filter=$('cardfilter').value;renderCards();};$('morecols').onchange=renderRepos;$('reladd').onclick=relationForm;
$('rescan').onclick=async()=>{await refreshState();await rescan();};
function showComposer(){$('composer').hidden=false;$('idea').focus();}
$('ideatoggle').onclick=showComposer;$('ideaclose').onclick=()=>$('composer').hidden=true;
$('idea').value=load('draft','');if($('idea').value)$('composer').hidden=false;$('idea').oninput=()=>save('draft',$('idea').value);
let sendingIdea=false;
async function submitIdea(){if(sendingIdea)return;const value=$('idea').value.trim(),payload=scopePayload();if(!value){errorAt('ideaerror','先寫下一個想法，再開始分析。');return;}if(!scopeRepos().length){errorAt('ideaerror','請先選擇至少一個 repo。');return;}
  sendingIdea=true;$('collidebtn').disabled=true;$('collidebtn').textContent='正在送出…';errorAt('ideaerror','');
  try{await api('/api/collide',{idea:value,...payload});if($('idea').value.trim()===value){$('idea').value='';save('draft','');}$('collidebtn').textContent='已送出';toast('想法已送出，分析結果會回到收件匣');await refreshState();}
  catch(e){errorAt('ideaerror',e.message+' 草稿已保留。');}
  finally{sendingIdea=false;$('collidebtn').disabled=false;$('collidebtn').textContent='分析想法';}}
$('collidebtn').onclick=submitIdea;$('idea').onkeydown=e=>{if(e.key==='Enter'&&(e.ctrlKey||e.metaKey)&&!e.isComposing){e.preventDefault();submitIdea();}};
$('briefbtn').onclick=async e=>{const b=e.currentTarget;b.disabled=true;try{const r=await api('/api/brief',scopePayload());fullText('今日簡報已儲存',r.text,r.file);await refreshState();}catch(e){toast(e.message);}finally{b.disabled=false;}};
$('ackbtn').onclick=()=>{const count=state.unread?.length||0,body=showDetail('將所有組的事件標為已讀？','ALL WORKSPACES');body.append(el('p','dialogmessage',`這會標記所有組目前的 ${count} 則未讀事件，包含其他牌組的提醒。\n\n處理中任務與待跟進事項會保留。標記已讀不代表完成處理。`));$('detailactions').append(button('取消',()=>$('detaildialog').close()),button('所有組標為已讀',async e=>{const b=e.currentTarget;b.disabled=true;try{await api('/api/ack',{});$('detaildialog').close();await refreshState();toast('所有組的事件已標為已讀');}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}},'primary'));};

// Native dialogs provide focus containment, Escape, and return focus.
for(const d of document.querySelectorAll('dialog'))d.addEventListener('click',e=>{if(e.target!==d)return;const r=d.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)d.close();});
document.querySelectorAll('[data-close]').forEach(b=>b.onclick=()=>$(b.dataset.close).close());
let pickSelected=new Set(),pickTags=new Set(),pickQuery='',pickerBusy=false,pickEditing=null;
function pickMatches(r){return r.toLocaleLowerCase().includes(pickQuery)&&(!pickTags.size||(state.tags[r]||[]).some(t=>pickTags.has(t)));}
// openPicker(group)＝編輯既有牌組：同一個選擇視窗，預勾現有成員，可改名、刪除。
function openPicker(group=null){pickEditing=group?group.name:null;pickSelected=new Set(group?group.members:scopeRepos());pickTags.clear();pickQuery='';$('picksearch').value='';$('pickname').value=group?group.name:'';errorAt('pickerror','');
  $('picktitle').textContent=group?'編輯牌組':'選擇 repo';$('picksub').textContent=group?'勾選＝這組的成員；取消勾選只是移出這組，repo 登記不受影響。':'組成臨時工作範圍，或儲存成常用牌組。';
  $('picknamelabel').firstChild.textContent=group?'牌組名稱 ':'儲存牌組名稱 ';$('picknamehint').hidden=!!group;$('pickdelete').hidden=!group;renderPicker();$('pickdialog').showModal();$('picksearch').focus();}
function renderPickCount(){$('pickcount').textContent='已選 '+pickSelected.size+' 個';$('pickok').disabled=pickerBusy||!pickSelected.size;$('pickok').textContent=pickEditing?'儲存變更':'套用範圍';$('pickdelete').disabled=pickerBusy;}
function renderPicker(){const used=[...new Set(Object.values(state.tags).flat())].sort();$('picktags').replaceChildren(...used.map(t=>{const b=button(t,()=>{pickTags.has(t)?pickTags.delete(t):pickTags.add(t);renderPicker();},'chip'+(pickTags.has(t)?' on':''));b.setAttribute('aria-pressed',String(pickTags.has(t)));return b;}));
  const visible=state.repos.filter(pickMatches).slice().sort();$('picklist').replaceChildren(...(visible.length?visible.map(r=>{const row=el('label','pickrow'),cb=el('input');cb.type='checkbox';cb.checked=pickSelected.has(r);cb.setAttribute('aria-label','選取 '+r);cb.onchange=()=>{cb.checked?pickSelected.add(r):pickSelected.delete(r);renderPickCount();};const copy=el('span','pickcopy',r);copy.append(el('span','picktags',(state.tags[r]||[]).join(' · ')));row.append(cb,copy);return row;}):[el('p','emptyline','沒有符合篩選的 repo；已選項目仍會保留。')]));renderPickCount();}
$('adhocbtn').onclick=openPicker;$('picksearch').oninput=()=>{pickQuery=$('picksearch').value.toLocaleLowerCase();renderPicker();};$('pickall').onclick=()=>{state.repos.filter(pickMatches).forEach(r=>pickSelected.add(r));renderPicker();};$('picknone').onclick=()=>{pickSelected.clear();renderPicker();};
$('pickrelated').onclick=()=>{const before=new Set(pickSelected);for(const r of state.relations){if(before.has(r.from))pickSelected.add(r.to);if(before.has(r.to))pickSelected.add(r.from);}renderPicker();toast(pickSelected.size===before.size?'沒有其他相關 repo':'已加入 '+(pickSelected.size-before.size)+' 個相關 repo');};
$('pickform').onsubmit=async e=>{e.preventDefault();if(pickerBusy||!pickSelected.size)return;const repos=[...pickSelected],name=$('pickname').value.trim();if(pickEditing)return saveGroupEdit(repos,name);pickerBusy=true;renderPickCount();try{if(name){await api('/api/save_group',{name,repos});await refreshState();}setScope(name?{group:name,repos:null}:{group:null,repos});$('pickdialog').close();toast(name?'已儲存牌組「'+name+'」':'已套用 '+repos.length+' 個 repo');}catch(e){errorAt('pickerror',e.message);}finally{pickerBusy=false;renderPickCount();}};

async function saveGroupEdit(repos,name){const old=pickEditing;if(!name)return errorAt('pickerror','牌組名稱不可為空');pickerBusy=true;renderPickCount();
  try{const r=await api('/api/update_group',{name:old,repos,new_name:name});if(scope.group===old)setScope({group:r.name,repos:null});if(openGroups.has(old)&&r.name!==old){openGroups.delete(old);openGroups.add(r.name);save('openGroups',[...openGroups]);}
    await refreshState();$('pickdialog').close();toast('已更新牌組「'+r.name+'」 · '+r.members.length+' 個 repo');}catch(e){errorAt('pickerror',e.message);}finally{pickerBusy=false;renderPickCount();}}
$('pickdelete').onclick=()=>{const name=pickEditing,g=state.groups.find(x=>x.name===name);if(!g)return;$('pickdialog').close();
  const body=showDetail('刪除牌組「'+name+'」？','GROUP');body.append(el('p','dialogmessage',`只會刪除這個牌組的定義。\n\n裡面的 ${g.members.length} 個 repo 仍保留登記；這組過去的簡報與存料（groups/${name}/）也會留著。`));
  $('detailactions').append(button('取消',()=>{$('detaildialog').close();openPicker(g);}),button('刪除牌組',async e=>{const b=e.currentTarget;b.disabled=true;try{await api('/api/remove_group',{name});if(scope.group===name)setScope({group:null,repos:null});openGroups.delete(name);save('openGroups',[...openGroups]);$('detaildialog').close();await refreshState();toast('已刪除牌組「'+name+'」');}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}},'danger'));};

// First run: pick a folder, preview repos, create the first group — all in the UI.
function openOnboarding(){let found=[],picked=new Set(),busy=false;
  const body=showDetail('建立第一個工作範圍','FIRST RUN');
  body.append(el('p','detailmeta','工作台只讀這個資料夾下一層有沒有 .git，掃描不會改動任何 repo。'));
  const df=el('label','field','放 repo 的資料夾'),dir=el('input');dir.placeholder='例如 /Users/you/Coding';df.append(dir);body.append(df);
  const list=el('div','onboardlist');body.append(list);
  const nf=el('label','field','第一組的名稱');const nameHint=el('span','muted small',' 選填，留空只登記 repo');nf.append(nameHint);const name=el('input');name.placeholder='例如：產品開發';name.maxLength=100;nf.append(name);nf.hidden=true;body.append(nf);
  const create=button('登記並建立這一組',async()=>{if(busy||!picked.size)return;busy=true;create.disabled=true;errorAt('detailerror','');
    try{const repos=found.filter(c=>picked.has(c.path));const r=await api('/api/register_repos',{repos,group:name.value.trim()});
      $('detaildialog').close();await refreshState();if(r.group)setScope({group:r.group,repos:null});else await rescan();
      toast('已登記 '+r.added.length+' 個 repo'+(r.group?'，並建立牌組「'+r.group+'」':''));}
    catch(e){errorAt('detailerror',e.message);}finally{busy=false;create.disabled=!picked.size;}},'primary');
  create.disabled=true;
  const renderFound=()=>{nf.hidden=!found.length;create.disabled=!picked.size;
    list.replaceChildren(...(found.length?found.map(c=>{const row=el('label','pickrow'),cb=el('input');cb.type='checkbox';cb.checked=picked.has(c.path);cb.setAttribute('aria-label','登記 '+c.id);
      cb.onchange=()=>{cb.checked?picked.add(c.path):picked.delete(c.path);create.disabled=!picked.size;};
      const copy=el('span','pickcopy',c.id);copy.append(el('span','picktags',c.path));row.append(cb,copy);return row;}):[el('p','emptyline','這個資料夾下一層沒有尚未登記的 git repo。')]));};
  const preview=button('預覽這個資料夾',async e=>{const b=e.currentTarget;b.disabled=true;errorAt('detailerror','');
    try{const r=await api('/api/scan_dir',{dir:dir.value.trim()});found=r.candidates;picked=new Set(found.map(c=>c.path));renderFound();}
    catch(err){found=[];picked=new Set();renderFound();errorAt('detailerror',err.message);}finally{b.disabled=false;}});
  dir.onkeydown=e=>{if(e.key==='Enter'&&!e.isComposing){e.preventDefault();preview.click();}};
  $('detailactions').append(preview,create);dir.focus();}

// Sessions retain their own immutable scope; creating one is an explicit form.
function taskEntries(){const out=[];const s=scan.summary;
  if(s)out.push(['接續上次工作',s.resume_task]);
  out.push(['整理本組現況','先讀情境卡，用 3 行說明：你拿到哪些脈絡、這次預計產出什麼、哪些動作需要我先確認，再問我要做什麼。']);
  const members=new Set(scopeRepos());
  const rel=state.relations.find(r=>members.has(r.from)&&members.has(r.to));
  if(rel)out.push([(state.rel_kinds[rel.kind]?.forward||rel.kind)+'：'+rel.from+' → '+rel.to,relationTask(rel)]);
  return out;}
function openSession(extra={}){const payload={...scopePayload(),...extra};if(extra.group){delete payload.repos;}if(extra.repo){delete payload.group;payload.repos=[extra.repo];}
  const body=showDetail('開啟 Agent 會話','NEW SESSION');const target=payload.repo||payload.group||(payload.repos?'臨時工作組 · '+payload.repos.length+' 個 repo':'所有 repo');body.append(el('p','detailmeta','工作範圍：'+target));
  body.append(el('p','sumsrc','Agent 開場會拿到：組情境卡（成員與關係、脈動、未結、最近事件）＋文件地圖，並被要求先說明拿到什麼、預計產出什麼、哪些動作要你確認。'));
  body.append(el('p','sumsrc','工作範圍是行為約定，不是工具層的權限限制——Agent 手上的工具碰得到組外檔案，所以它被要求越界前先問你。'));
  const field=el('label','field','使用的 Agent'),select=el('select');(state.agents||[]).forEach(a=>select.append(new Option(a,a)));select.value=load('agent',state.agents?.[0]||'claude');if(!select.value&&select.options.length)select.selectedIndex=0;field.append(select);body.append(field);
  const tf=el('label','field','這次想完成什麼？'),task=el('textarea');task.rows=4;task.value=extra.task||'';task.placeholder='留空時，Agent 會先整理這組 repo 的現況。';tf.append(task);body.append(tf);
  const hints=el('div','taskhints');hints.append(el('span','small muted','有情境的起手：'));
  for(const[label,text]of taskEntries())hints.append(button(label,()=>{task.value=text;task.focus();},'chip'));
  body.append(hints);
  const b=button('開始會話',async()=>{b.disabled=true;errorAt('detailerror','');try{save('agent',select.value);const r=await api('/api/term_create',{...payload,kind:'agent',agent:select.value,task:task.value.trim(),origin:payload.origin||'scope'});$('detaildialog').close();await refreshState();selectTerm(r.sid);toast('會話已開啟');}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}},'primary');b.disabled=!state.agents?.length;$('detailactions').append(b);}
$('newagent').onclick=()=>openSession({origin:'group:'+scopeLabel()});
$('newshell').onclick=async e=>{const b=e.currentTarget;b.disabled=true;try{const r=await api('/api/term_create',{kind:'shell'});await refreshState();selectTerm(r.sid);}catch(e){toast(e.message);}finally{b.disabled=false;}};
const terms=new Map();let activeSid=null,collapsed=true,termHeight=260;
// 程序活著不等於工作在推進：回報狀態只認脊椎留痕，沒有留痕就說沒有。
const STATUS_TXT={working:'處理中',waiting:'等你回應',blocked:'卡住了',done:'已交接'};
function reportLine(s){if(s.kind!=='agent')return '本機 Shell，不回報';
  if(s.agent_status)return 'Agent 回報「'+(STATUS_TXT[s.agent_status.status]||s.agent_status.status)+'」'+s.agent_status.at.slice(11)+(s.agent_status.text?'：'+s.agent_status.text:'');
  if(s.report)return '最近留痕 '+s.report.at+'（'+s.report.type+' · 來自 '+s.report.source+'）：'+s.report.text;
  return '尚無回報——這個會話期間，範圍內還沒有任何脊椎留痕';}
function sessionMeta(s){const proc=s.alive?'程序執行中':'程序已結束';
  const work=s.kind!=='agent'?'Shell':(s.agent_status?(STATUS_TXT[s.agent_status.status]||s.agent_status.status):(s.report?'最近留痕 '+s.report.at.slice(11):'尚無回報'));
  return proc+' · '+work+' · '+(s.scope||'本機 Shell');}
function syncTerms(){const list=state.terms||[],ids=new Set(list.map(s=>s.sid));for(const[sid,rec]of terms)if(!ids.has(sid)){disposeTerm(rec);terms.delete(sid);if(activeSid===sid)activeSid=null;}
  const parent=$('sessions');for(const s of list){let rec=terms.get(s.sid);if(!rec){rec={info:s,t:null,ws:null,el:null,connected:false};terms.set(s.sid,rec);}rec.info=s;
    if(!rec.row){rec.row=el('div','session');const b=button('',()=>selectTerm(s.sid),'ghost');b.append(el('span','dot'),el('span','sessioncopy'));rec.row.append(b);parent.append(rec.row);}
    rec.row.classList.toggle('on',activeSid===s.sid);const b=rec.row.firstChild;b.setAttribute('aria-pressed',String(activeSid===s.sid));
    b.title=s.title+'\n'+(s.task||'')+'\n範圍：'+(s.scope||'本機 Shell')+'\n'+reportLine(s)+'\n依據：'+(s.report_basis||'—');
    const sig=JSON.stringify(s);if(rec.sig!==sig){rec.sig=sig;const dot=b.querySelector('.dot');dot.classList.toggle('dead',!s.alive);dot.classList.toggle('silent',s.alive&&s.kind==='agent'&&!s.report&&!s.agent_status);dot.classList.toggle('needs',!!s.agent_status&&['waiting','blocked'].includes(s.agent_status.status));
      b.querySelector('.sessioncopy').replaceChildren(el('span','sessiontitle',s.title),el('span','sessionmeta',sessionMeta(s)));}}
  $('sessempty').hidden=!!list.length;$('sessiontotal').textContent=list.length||'';$('sesscount').textContent=list.filter(s=>s.alive).length+' 個執行中會話';updateTermBar();}
function updateTermBar(){const rec=terms.get(activeSid);$('termtitle').textContent=rec?rec.info.title+' · '+(rec.info.scope||'本機 Shell')+' · '+reportLine(rec.info)+(!rec.connected&&rec.t?' · 連線已中斷（畫面可能不是最新）':''):'尚未選擇會話';$('termkill').disabled=!rec;$('termresult').disabled=!rec;$('termreconnect').hidden=!rec?.t||rec.connected;$('termempty').hidden=!!rec;}
function disposeTerm(rec){if(rec.ws){rec.ws.onclose=null;rec.ws.close();}rec.t?.dispose();rec.el?.remove();rec.row?.remove();}
function selectTerm(sid){const rec=terms.get(sid);if(!rec)return;activeSid=sid;if(collapsed)toggleTerm();for(const[id,r]of terms)r.el?.classList.toggle('on',id===sid);if(!rec.t)attachTerm(sid);else{fitTerm(rec);rec.t.focus();}syncTerms();}
function attachTerm(sid){const rec=terms.get(sid);if(!rec)return;if(rec.ws){rec.ws.onclose=null;rec.ws.close();}rec.t?.dispose();rec.el?.remove();
  const host=el('div','termhost on');$('termbody').append(host);const t=new Terminal({fontSize:13,fontFamily:'Cascadia Code,Consolas,Menlo,monospace',theme:{background:'#11151b',foreground:'#e9edf4',cursor:'#a7c5ff',selectionBackground:'#394966'},cursorBlink:true,scrollback:5000});const fit=new FitAddon.FitAddon();t.loadAddon(fit);t.open(host);
  const ws=new WebSocket((location.protocol==='https:'?'wss://':'ws://')+location.host+'/ws/term/'+sid+'?token='+TOKEN);ws.binaryType='arraybuffer';Object.assign(rec,{t,fit,ws,el:host,connected:false});
  ws.onmessage=e=>t.write(new Uint8Array(e.data));ws.onopen=()=>{rec.connected=true;fitTerm(rec);updateTermBar();t.focus();};ws.onclose=()=>{rec.connected=false;t.write('\r\n\x1b[2m[連線已中斷。可按「重新連線」，輸出仍保留。]\x1b[0m\r\n');updateTermBar();};ws.onerror=()=>{rec.connected=false;updateTermBar();};
  t.onData(d=>{if(ws.readyState===1)ws.send(JSON.stringify({t:'i',d}));});t.onResize(({cols,rows})=>{if(ws.readyState===1)ws.send(JSON.stringify({t:'r',c:cols,r:rows}));});
  for(const[id,r]of terms)r.el?.classList.toggle('on',id===sid);}
function fitTerm(rec){try{rec.fit?.fit();}catch{}}
function fitAll(){for(const rec of terms.values())if(rec.t)fitTerm(rec);}
$('termreconnect').onclick=()=>attachTerm(activeSid);
function sessionResult(sid,after){const rec=terms.get(sid);if(!rec)return;const body=showDetail('記錄會話結果','SESSION RESULT');body.append(el('p','detailmeta',rec.info.title+' · '+(rec.info.scope||'本機 Shell')+' · '+reportLine(rec.info)));
  const note=el('p','sumsrc','正在整理交接草稿…');body.append(note);
  const label=el('label','field','完成了什麼？還有哪些下一步？'),input=el('textarea');input.rows=9;label.append(input);body.append(label);
  api('/api/session_draft',{sid}).then(r=>{if(!input.value.trim())input.value=r.text;note.textContent=r.note+(r.traces?'':'（這個會話期間沒有留痕，草稿只有骨架。）');})
    .catch(e=>{note.textContent='無法整理草稿：'+e.message+' 可以直接自己填寫。';});$('detailactions').append(button('確認並儲存',async e=>{const b=e.currentTarget;if(!input.value.trim()){errorAt('detailerror','請填寫結果。');return;}b.disabled=true;try{await api('/api/session_result',{sid,text:input.value.trim()});$('detaildialog').close();toast('結果已儲存（記為 decision，不代表成果已被採納）');await refreshState();if(after)after();}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}},'primary'));input.focus();}
$('termresult').onclick=()=>sessionResult(activeSid);
function confirmKill(sid){const rec=terms.get(sid);if(!rec)return;const body=showDetail('結束這個會話程序？','SESSION');body.append(el('p','dialogmessage',rec.info.title+'\n\n這會停止程序並移除會話。若只想保留工作並收起畫面，請使用「收合」。'));$('detailactions').append(button('先記錄結果',()=>sessionResult(sid,()=>confirmKill(sid))),button('保留並收合',()=>{$('detaildialog').close();if(!collapsed)toggleTerm();}),button('結束程序',async e=>{const b=e.currentTarget;b.disabled=true;try{await api('/api/term_kill',{sid});$('detaildialog').close();await refreshState();toast('會話程序已結束');}catch(e){errorAt('detailerror',e.message);}finally{b.disabled=false;}},'danger'));}
$('termkill').onclick=()=>confirmKill(activeSid);
function setTermHeight(value){termHeight=Math.min(Math.max(value,100),Math.max(100,window.innerHeight-240));collapsed=false;$('termbody').hidden=false;document.documentElement.style.setProperty('--termh',termHeight+'px');$('termdrag').setAttribute('aria-valuenow',String(termHeight));$('termdrag').setAttribute('aria-valuemax',String(Math.max(100,window.innerHeight-240)));$('termtoggle').textContent='收合';$('termtoggle').setAttribute('aria-expanded','true');fitAll();}
function toggleTerm(){if(collapsed)setTermHeight(termHeight);else{collapsed=true;$('termbody').hidden=true;document.documentElement.style.setProperty('--termh','44px');$('termtoggle').textContent='展開';$('termtoggle').setAttribute('aria-expanded','false');}fitAll();}
$('termtoggle').onclick=toggleTerm;let dragging=false;$('termdrag').onpointerdown=e=>{dragging=true;$('termdrag').setPointerCapture(e.pointerId);e.preventDefault();};$('termdrag').onpointermove=e=>{if(dragging)setTermHeight(window.innerHeight-e.clientY-26);};$('termdrag').onpointerup=()=>dragging=false;$('termdrag').onpointercancel=()=>dragging=false;$('termdrag').onkeydown=e=>{if(['ArrowUp','ArrowDown'].includes(e.key)){e.preventDefault();setTermHeight(termHeight+(e.key==='ArrowUp'?24:-24));}};
window.addEventListener('resize',()=>{if(!collapsed)setTermHeight(termHeight);fitAll();});window.addEventListener('keydown',e=>{if(e.ctrlKey&&e.key==='`'&&!document.querySelector('dialog[open]')){e.preventDefault();toggleTerm();}});
(async()=>{await refreshState();await rescan();if(location.hash==='#deep')showTab('repos');if(location.hash==='#adhoc')openPicker();})();
setInterval(refreshState,5000);setInterval(()=>{if(!scanLoading)rescan();},120000);
