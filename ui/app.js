const $=id=>document.getElementById(id);
const storageGet=(kind,key)=>{try{return (kind==='local'?localStorage:sessionStorage).getItem(key);}catch{return null;}};
const storageSet=(kind,key,value)=>{try{(kind==='local'?localStorage:sessionStorage).setItem(key,value);}catch{}};
const escapeHTML=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const clip=(s,n)=>{s=String(s??'');return s.length>n?s.slice(0,n-1)+'…':s;};
const bytes=n=>n>=1e9?`${(n/1e9).toFixed(1)} GB`:n>=1e6?`${(n/1e6).toFixed(1)} MB`:`${Math.round((n||0)/1e3)} KB`;
const duration=n=>{n=Math.max(0,Math.floor(n||0));const h=Math.floor(n/3600),m=Math.floor(n%3600/60),s=String(n%60).padStart(2,'0');return h?`${h}:${String(m).padStart(2,'0')}:${s}`:`${m}:${s}`;};
const setText=(el,text)=>{if(el.textContent!==text)el.textContent=text;};
const isNum=n=>typeof n==='number'&&Number.isFinite(n);
const clock=ms=>new Date(ms).toLocaleTimeString(undefined,{hour:'numeric',minute:'2-digit'});
function when(s){
  const d=new Date(s);if(Number.isNaN(d.getTime()))return '';
  const now=new Date(),yesterday=new Date(now),same=(a,b)=>a.toDateString()===b.toDateString();yesterday.setDate(now.getDate()-1);
  const time=clock(d);
  if(same(d,now))return `Today, ${time}`;if(same(d,yesterday))return `Yesterday, ${time}`;
  return `${d.toLocaleDateString(undefined,{month:'short',day:'numeric',...(d.getFullYear()!==now.getFullYear()?{year:'numeric'}:{})})}, ${time}`;
}
const icons={
  all:'<rect x="3.5" y="5" width="17" height="14" rx="3"/><path d="m10.5 9.5 4 2.5-4 2.5z" fill="currentColor"/>',
  star:'<path d="m12 3.8 2.4 5 5.4.7-4 3.8 1 5.4L12 16l-4.8 2.7 1-5.4-4-3.8 5.4-.7z"/>',
  link:'<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
  plus:'<path d="M12 5v14M5 12h14"/>',
  archive:'<rect x="3.5" y="4.5" width="17" height="4.5" rx="1.5"/><path d="M5.5 9v9a1.5 1.5 0 0 0 1.5 1.5h10a1.5 1.5 0 0 0 1.5-1.5V9M10 13h4"/>',
  cog:'<path fill="currentColor" stroke="none" fill-rule="evenodd" d="M10.46 4.76L10.67 1.89 L13.33 1.89 L13.54 4.76A7.4 7.4 0 0 1 16.03 5.79L18.21 3.91 L20.09 5.79 L18.21 7.97A7.4 7.4 0 0 1 19.24 10.46L22.11 10.67 L22.11 13.33 L19.24 13.54A7.4 7.4 0 0 1 18.21 16.03L20.09 18.21 L18.21 20.09 L16.03 18.21A7.4 7.4 0 0 1 13.54 19.24L13.33 22.11 L10.67 22.11 L10.46 19.24A7.4 7.4 0 0 1 7.97 18.21L5.79 20.09 L3.91 18.21 L5.79 16.03A7.4 7.4 0 0 1 4.76 13.54L1.89 13.33 L1.89 10.67 L4.76 10.46A7.4 7.4 0 0 1 5.79 7.97L3.91 5.79 L5.79 3.91 L7.97 5.79A7.4 7.4 0 0 1 10.46 4.76Z M15.4 12a3.4 3.4 0 1 0-6.8 0a3.4 3.4 0 1 0 6.8 0Z"/>',
  menu:'<path d="M4 7h16M4 12h16M4 17h16"/>',
  search:'<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4 4"/>',
  back:'<path d="m14.5 6-6 6 6 6"/>',
  more:'<g fill="currentColor" stroke="none"><circle cx="5.5" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="18.5" cy="12" r="1.5"/></g>',
  close:'<path d="m6 6 12 12M18 6 6 18"/>',
  display:'<rect x="3.5" y="5" width="17" height="11" rx="2"/><path d="M9 19.5h6M12 16v3.5"/>',
  camera:'<rect x="3.5" y="7" width="11" height="10" rx="2.5"/><path d="m14.5 11 6-3v8l-6-3z"/>',
  mic:'<rect x="9" y="3.5" width="6" height="11" rx="3"/><path d="M6 11.5a6 6 0 0 0 12 0M12 17.5v3"/>',
  play:'<path d="M9 7.2v9.6l8-4.8z" fill="currentColor"/>',
  share:'<path d="M8 9H4v12h16V9h-4M12 15V2m-4 4 4-4 4 4"/>',
  folder:'<path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2.2h7a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z"/>',
  check:'<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  next:'<path d="m9.5 6 6 6-6 6"/>'
};
const svg=name=>`<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name]||''}</svg>`;
const shapeNames={circle:'Circle',rounded:'Rounded square',square:'Square',wide:'Wide 16:9',portrait:'Portrait 9:16'};
const poster='<svg class="poster-art" viewBox="0 0 160 90" aria-hidden="true"><rect class="p-frame" x="44" y="21" width="72" height="48" rx="6"/><path class="p-bar" d="M44 33h72"/><circle class="p-dot" cx="80" cy="51" r="9"/><path class="p-play" d="M77.5 47.2v7.6l6.4-3.8z"/></svg>';
document.querySelectorAll('[data-icon]').forEach(el=>el.innerHTML=svg(el.dataset.icon));
let state=null,view='all',selectedId=null,busy=false,queued=false,toastTimer,toastBox=null,toastMs=0,nameCallback,bridgeId=0,pollTimer,gridSignature='',recSignature='',folderMarkup='',lastMessage='',playerFile='',pushed=false,studioBusy=false;
const bridgePending=new Map();let hostOrigin=null;
const hashParts=location.hash.slice(1).split('/'),hashRoute=hashParts[1]||'';
const localKey=CONNECTION.key||hashParts[0]||storageGet('session','or-key');if(!CONNECTION.embedded&&localKey)storageSet('session','or-key',localKey);if(!CONNECTION.embedded&&location.hash)history.replaceState(null,'',location.pathname);
let denied=!CONNECTION.embedded&&!localKey;
const reach={failures:0,stopped:false,manual:false,loadedAt:null,error:''};
const retryDelays=[2000,5000,15000];
function rpc(method,params={},deadline=135000){return new Promise((resolve,reject)=>{const id=`or-${++bridgeId}`;const timer=setTimeout(()=>{bridgePending.delete(id);reject(Object.assign(new Error('The app did not respond. Reopen OpenRecorder and try again.'),{unreachable:true,timedOut:true}));},deadline);bridgePending.set(id,{resolve,reject,timer});parent.postMessage({jsonrpc:'2.0',id,method,params},hostOrigin||'*');});}
addEventListener('message',event=>{
  if(event.source!==parent)return;if(hostOrigin&&event.origin!==hostOrigin)return;
  const message=event.data;if(!message||message.jsonrpc!=='2.0')return;
  if(message.id&&bridgePending.has(message.id)){const request=bridgePending.get(message.id);bridgePending.delete(message.id);clearTimeout(request.timer);hostOrigin=event.origin;message.error?request.reject(new Error(message.error.message)):request.resolve(message.result);}
  if(message.method==='ui/notifications/tool-result'&&validState(message.params?.structuredContent)){
    // A pushed result is not proof about shares (a settings result carries a placeholder list): keep the last list the
    // host's own state read gave, and confirm by reading the host.
    commitState(message.params.structuredContent,true);reachOk();render();void refresh();}
  if(message.method==='ui/notifications/host-context-changed')applyHost(message.params);
  if(message.method==='ui/resource-teardown'){clearTimeout(pollTimer);clearInterval(tickTimer);$('player').pause();if(message.id)parent.postMessage({jsonrpc:'2.0',id:message.id,result:{}},hostOrigin||'*');}
});
function applyHost(context){const c=context?.hostContext||context;if(c?.theme&&!storageGet('local','or-theme'))document.documentElement.dataset.theme=c.theme;const insets=c?.safeAreaInsets;if(insets)document.body.style.padding=`${insets.top||0}px ${insets.right||0}px ${insets.bottom||0}px ${insets.left||0}px`;}
// Every request has an application deadline that covers the response body too, so a host that accepts a connection
// and then stalls can never leave the page "busy" for ever. Reads time out quickly and are retried (2, 5, 15 s, then
// manually). A timed-out change is UNCERTAIN: the host may have applied it, so it is never replayed automatically
// and the page re-reads the host before claiming success or rolling anything back.
const deadlines={read:7000,change:20000,share:45000};
const uncertainText='OpenRecorder didn’t answer in time, so it’s unknown whether the change was applied. Showing what is saved.';
function markUncertain(error,op,own){
  // A caller that reads the host back itself (settings, share) passes own=true; everyone else gets one refresh.
  if(op!=='state'&&error.unreachable){error.uncertain=true;error.detail=error.detail||error.message;error.message=uncertainText;if(!own&&typeof refresh==='function')Promise.resolve().then(()=>void refresh());}
  return error;
}
async function fetchWithin(url,options,ms){
  const controller=typeof AbortController==='function'?new AbortController():null;let timer,expired=false;
  const guard=new Promise((_,reject)=>{timer=setTimeout(()=>{expired=true;if(controller)controller.abort();reject(Object.assign(new Error('OpenRecorder did not answer in time.'),{unreachable:true,timedOut:true}));},ms);});
  const exchange=(async()=>{const response=await fetch(url,controller?{...options,signal:controller.signal}:options);let value,bodyError=null;try{value=await response.json();}catch(e){bodyError=e||new Error('unreadable body');}return{response,value,bodyError};})();
  exchange.catch(()=>{});
  try{return await Promise.race([exchange,guard]);}finally{clearTimeout(timer);if(expired)exchange.catch(()=>{});}
}
// A library snapshot is accepted only when every field the page reads has the shape the host serializes (see
// Store::snapshot / shares::snapshot in the host): authoritative fields must be present and typed, optional metadata
// only has to be well-typed when it appears. One validator guards every assignment of the library state, so a partial
// or malformed answer can never replace the last valid content, and an absent `shares` list is never read as "no shares".
const isStr=v=>typeof v==='string',isBool=v=>typeof v==='boolean',isObj=v=>!!v&&typeof v==='object'&&!Array.isArray(v),isNumber=v=>typeof v==='number'&&Number.isFinite(v);
const nonEmpty=v=>isStr(v)&&v.length>0,nullable=check=>v=>v===null||check(v),arrayOf=check=>v=>Array.isArray(v)&&v.every(check);
const shape=(required,optional={})=>v=>isObj(v)&&Object.entries(required).every(([k,check])=>k in v&&check(v[k]))&&Object.entries(optional).every(([k,check])=>!(k in v)||check(v[k]));
const fileShape=shape({name:isStr},{bytes:isNumber,width:isNumber,height:isNumber,duration:isNumber});
const recordingShape=shape({id:nonEmpty,title:isStr,status:isStr,files:arrayOf(fileShape)},{createdAt:v=>isNumber(v)||isStr(v),duration:isNumber,bytes:isNumber,folder:nullable(isStr),favorite:isBool,archived:isBool,settings:isObj,error:nullable(isStr),directory:isStr,thumbnailReady:isBool,finalized:isBool,capture:isObj,source:v=>v===null||isObj(v)||isStr(v)});
const folderShape=shape({id:nonEmpty,name:isStr});
// Every key here is always serialized by the host (defaults plus normalization): an answer without one is incomplete, so
// it can neither replace the settings nor prove that a change was or was not saved.
const settingsShape=shape({theme:isStr,quality:isNumber,fps:isNumber,keepOriginals:isBool,mirror:isBool,cameraShape:isStr,cameraSize:isNumber,countdownSeconds:isNumber,rememberInputs:isBool},{camera:isBool,microphone:isBool,showCamera:isBool,fpsChosen:isBool});
const activeShape=shape({status:isStr},{id:isStr,startedAt:isNumber,countdownRemaining:isNumber});
// The app's own report of capture (phase, generation, devices). Optional: an older host sends none, and Home then shows the
// previous New recording button. Only well-typed content is accepted, so a malformed report can never invent a timer.
const captureShape=shape({phase:isStr,generation:isNumber},{known:isBool,stale:isBool,ageMs:isNumber,elapsedMs:isNumber,fpsSet:isNumber,fpsDelivered:isNumber,sourceFps:isNumber,outputFrames:isNumber,sourceFrames:isNumber,countdown:isNumber,trayAvailable:isBool,trayNote:isStr,screenAllowed:isBool,screenState:isStr,appearance:shape({shape:isStr,size:isNumber,mirror:isBool,showCamera:isBool}),inputs:isObj,source:isObj,sources:arrayOf(isObj),devices:isObj,notice:isObj,capabilities:isObj});
const shareShape=shape({id:isStr,status:isStr},{shareId:isStr,url:isStr,accessCode:isStr,expiresAt:isNumber,never:isBool,error:nullable(isStr),unlockedSessions:isNumber,rateLimited:isBool});
const stateShape=shape({recordings:arrayOf(recordingShape),folders:arrayOf(folderShape),settings:settingsShape,root:isStr,active:nullable(activeShape),shares:arrayOf(shareShape)},{freeBytes:nullable(isNumber),totalBytes:isNumber,version:isStr,platform:isStr,capture:captureShape});
const validState=stateShape;
const incompleteState=()=>Object.assign(new Error('OpenRecorder sent an incomplete library.'),{unreachable:true});
// The one place the library state is assigned. `keepShares` is for pushes whose share list is not authoritative.
// The one place state.settings is assigned from a partial answer (a successful settings write).
const incompleteSettings=()=>Object.assign(new Error('OpenRecorder’s answer about the setting was incomplete.'),{unreachable:true,uncertain:true});
function commitSettings(settings){
  if(!settingsShape(settings))throw incompleteSettings();
  state.settings=settings;
}
function commitState(next,keepShares){
  if(!validState(next))throw incompleteState();
  state=keepShares&&state&&Array.isArray(state.shares)?{...next,shares:state.shares}:next;
  captureSeen=Date.now();
}
async function api(op,data={},own=false){
  if(CONNECTION.embedded){let result;try{result=await rpc('tools/call',{name:'openrecorder_action',arguments:{op,data}},op==='state'?10000:135000);}catch(e){throw markUncertain(e,op,own);}if(result.isError)throw new Error(result.content?.[0]?.text||'Operation failed.');try{return result.structuredContent||JSON.parse(result.content?.find(c=>c.type==='text')?.text||'{}');}catch(e){throw markUncertain(Object.assign(new Error('OpenRecorder’s answer could not be read.'),{unreachable:true,detail:String(e?.message||e)}),op,own);}}
  let exchange;try{exchange=await fetchWithin(`${CONNECTION.origin}/api/${op==='state'?'state':'action'}`,{method:op==='state'?'GET':'POST',headers:{Authorization:`Bearer ${localKey}`,'Content-Type':'application/json'},...(op==='state'?{}:{body:JSON.stringify({op,data})})},op==='state'?deadlines.read:op==='share'?deadlines.share:deadlines.change);}catch(cause){throw markUncertain(Object.assign(new Error('OpenRecorder did not answer.'),{unreachable:true,timedOut:!!cause?.timedOut,detail:String(cause?.message||cause)}),op,own);}
  const{response:result,value,bodyError}=exchange;
  // A refusal can still say something short; an unreadable refusal body just gets the generic text.
  if(!result.ok)throw Object.assign(new Error((!bodyError&&value&&typeof value.error==='string'&&value.error)||(result.status===401||result.status===403?'This Library link is no longer valid.':'Operation failed.')),{status:result.status});
  // Success headers with a body that was cut off or is not JSON is NOT a success: a change is uncertain (the host may
  // have applied it) and a read failed. Nothing is substituted.
  if(bodyError)throw markUncertain(Object.assign(new Error('OpenRecorder’s answer was cut off.'),{unreachable:true,detail:String(bodyError.message||bodyError)}),op,own);
  return value;
}
// What only the page itself may ask (poster and playback bytes, a fresh media address, the person's Record press). In an
// agent host it is an app-only tool that the host hides from the model; in the browser it is the owner operation.
const uiControl={supported:null};
async function uiApi(op,data={},deadline){
  if(CONNECTION.embedded){
    let result;try{result=await rpc('tools/call',{name:'openrecorder_ui',arguments:{op,data,ui:CONNECTION.uiKey}},deadline||(op==='media-read'?30000:20000));}catch(e){throw markUncertain(e,op,false);}
    if(result.isError){const text=result.content?.[0]?.text||'Operation failed.';if(/unknown tool|not an operation|method not found|not supported/i.test(text)){const was=uiControl.supported;uiControl.supported=false;if(was!==false&&state&&typeof render==='function')Promise.resolve().then(render);}throw new Error(text);}
    uiControl.supported=true;
    try{return result.structuredContent||JSON.parse(result.content?.find(c=>c.type==='text')?.text||'{}');}
    catch(e){throw markUncertain(Object.assign(new Error('OpenRecorder’s answer could not be read.'),{unreachable:true,detail:String(e?.message||e)}),op,false);}
  }
  return api(op,data,false);
}
// The embedded app reads media with the scoped read-only media key; the browser library uses its private session key.
function fileURL(id,name,download=false){
  const credential=CONNECTION.embedded?(CONNECTION.mediaKey?`media=${encodeURIComponent(CONNECTION.mediaKey)}`:''):(localKey?`key=${encodeURIComponent(localKey)}`:'');
  return credential?`${CONNECTION.origin}/file/${encodeURIComponent(id)}/${encodeURIComponent(name)}?${credential}${download?'&download=1':''}`:'';
}
function toast(message,undo,isError){
  const box=$(isError?'toastAlert':'toast');$(isError?'toast':'toastAlert').hidden=true;
  $(isError?'toastAlertText':'toastText').textContent=clip(message,240);
  if(!isError){$('toastUndo').hidden=!undo;$('toastUndo').onclick=undo?()=>{box.hidden=true;undo();}:null;}
  box.hidden=false;toastBox=box;toastMs=isError?8000:undo?8000:4000;armToast();
}
function armToast(){clearTimeout(toastTimer);toastTimer=setTimeout(()=>{if(toastBox)toastBox.hidden=true;},toastMs);}
['toast','toastAlert'].forEach(id=>{const box=$(id);box.addEventListener('mouseenter',()=>clearTimeout(toastTimer));box.addEventListener('mouseleave',armToast);box.addEventListener('focusin',()=>clearTimeout(toastTimer));box.addEventListener('focusout',armToast);});
function showError(error){toast(error.message||String(error),null,true);}
async function act(op,data){try{const value=await api(op,data);await refresh();return value;}catch(e){showError(e);return null;}}
function lockedView(kind){
  denied=true;clearTimeout(pollTimer);document.body.dataset.mode='locked';$('app').hidden=true;$('locked').hidden=false;
  $('lockedText').textContent=kind==='expired'?'This Library link is no longer valid. Open the Library again from OpenRecorder.':'This page needs the private link OpenRecorder creates. Choose Open Home in the OpenRecorder menu, or ask your agent to open the library.';
}
function reachOk(){reach.failures=0;reach.stopped=false;reach.manual=false;reach.loadedAt=Date.now();renderReach();}
function reachFail(e){
  if(e.status===401||e.status===403){if(!CONNECTION.embedded){lockedView('expired');return;}}
  reach.error=clip(`${e.status?`HTTP ${e.status} · `:''}${e.message||e}${e.detail?` · ${e.detail}`:''}`,500);
  if(reach.manual){reach.manual=false;reach.stopped=true;}else{reach.failures++;if(reach.failures>retryDelays.length)reach.stopped=true;}
  renderReach();
}
function renderReach(){
  const down=reach.failures>0||reach.stopped;
  $('reach').hidden=!down;$('content').classList.toggle('dimmed',down&&!!state);
  if(!down)return;
  $('reachText').textContent=`${state&&reach.loadedAt?`Showing what was loaded at ${clock(reach.loadedAt)}. `:'Nothing has loaded yet. '}${reach.stopped?'Checked 3 times, then stopped.':'Checking again shortly.'}`;
  $('reachRaw').textContent=reach.error;$('reachRetry').hidden=!reach.stopped;
}
function schedule(){
  clearTimeout(pollTimer);if(denied||reach.stopped)return;
  const delay=reach.failures?retryDelays[reach.failures-1]:state&&state.shares.some(s=>shareState(s)==='starting')?2000:4000;
  pollTimer=setTimeout(()=>{if(!document.hidden)void refresh();},delay);
}
async function refresh(){
  if(denied)return;if(busy){queued=true;return;}
  busy=true;try{commitState(await api('state'));reachOk();render();}catch(e){reachFail(e);}finally{busy=false;}
  if(queued){queued=false;return refresh();}schedule();
}
const shareLink=s=>s.url||s.shareUrl||s.link||'';
const shareCode=s=>s.accessCode||s.code||s.shortCode||s.shareCode||'';
const formatCode=c=>{const s=String(c).toUpperCase().replace(/[^A-Z0-9]/g,'');return s.length===8?`${s.slice(0,4)}-${s.slice(4)}`:String(c);};
function shareState(s){
  if(s.status==='failed')return /cancel/i.test(String(s.error))?'cancelled':'failed';
  if(shareLink(s)&&s.status!=='starting')return isNum(s.expiresAt)&&s.expiresAt<=Date.now()?'expired':'ready';
  return 'starting';
}
const liveShares=()=>state.shares.filter(s=>['starting','ready'].includes(shareState(s)));
const bestShare=id=>{const order=['ready','starting','failed','expired'],mine=state.shares.filter(s=>s.id===id&&shareState(s)!=='cancelled');return mine.sort((a,b)=>order.indexOf(shareState(a))-order.indexOf(shareState(b)))[0];};
const isReady=r=>['ready','interrupted'].includes(r.status);
const canShare=r=>r.status==='ready'&&!r.archived;
const folderName=id=>state.folders.find(f=>f.id===id)?.name;
const sectionTitle=()=>({all:'All recordings',favorites:'Favorites',shared:'Shared',archived:'Archived'}[view]||folderName(view.slice(7))||'All recordings');
function filtered(){
  const query=$('search').value.trim().toLowerCase();let records=state.recordings.filter(r=>r.status!=='cancelled');
  records=records.filter(r=>view==='archived'?r.archived:!r.archived);
  if(view==='favorites')records=records.filter(r=>r.favorite);else if(view==='shared')records=records.filter(r=>liveShares().some(s=>s.id===r.id));else if(view.startsWith('folder:'))records=records.filter(r=>r.folder===view.slice(7));
  records=records.filter(r=>String(r.title).toLowerCase().includes(query));
  records.sort((a,b)=>$('sort').value==='longest'?(b.duration||0)-(a.duration||0):$('sort').value==='oldest'?new Date(a.createdAt).getTime()-new Date(b.createdAt).getTime():new Date(b.createdAt).getTime()-new Date(a.createdAt).getTime());return records;
}
function activeLabel(a){
  switch(a.status){
    case 'recording':return isNum(a.startedAt)?`Recording ${duration((Date.now()-a.startedAt)/1000)}`:'Recording';
    case 'saving':return 'Saving…';
    case 'countdown':return isNum(a.countdownRemaining)?`Starting in ${Math.ceil(a.countdownRemaining)}`:'Countdown';
    case 'starting':return 'Starting…';
    case 'needs-access':return 'Waiting for access in OpenRecorder';
    case 'ready-to-record':return 'Ready in OpenRecorder';
    default:return 'In use in OpenRecorder';
  }
}
function render(){
  if(!state)return;
  if(!settingsSaving&&(state.settings.theme||'system')!==appliedTheme)applyTheme(state.settings.theme||'system');
  if(view.startsWith('folder:')&&!folderName(view.slice(7)))view='all';
  if(selectedId&&!state.recordings.some(r=>r.id===selectedId))closeRecording();
  renderNav();renderActive();renderCapture();
  const inRecording=!!selectedId;
  $('libraryView').hidden=inRecording;$('recordingView').hidden=!inRecording;$('topbar').dataset.page=inRecording?'recording':'library';
  if(inRecording)renderRecording();else renderGrid();
  renderShare();renderSettingsLive();
  if(state.message&&state.message!==lastMessage)toast(state.message);lastMessage=state.message||'';
}
function renderNav(){
  const live=state.recordings.filter(r=>!r.archived&&!['cancelled','error'].includes(r.status));
  $('allCount').textContent=live.length;$('favoriteCount').textContent=live.filter(r=>r.favorite).length||'';
  $('sharedCount').textContent=new Set(liveShares().map(s=>s.id)).size||'';
  const markup=state.folders.map(f=>`<button class="nav-item" data-view="folder:${escapeHTML(f.id)}" title="${escapeHTML(f.name)}">${svg('folder')}<span class="nav-label">${escapeHTML(f.name)}</span><span class="count">${live.filter(r=>r.folder===f.id).length}</span></button>`).join('');
  if(folderMarkup!==markup){folderMarkup=markup;$('folders').innerHTML=markup;}
  document.querySelectorAll('[data-view]').forEach(b=>{const active=b.dataset.view===view;b.classList.toggle('active',active);if(active)b.setAttribute('aria-current','page');else b.removeAttribute('aria-current');});
  $('pageTitle').textContent=selectedId?(state.recordings.find(r=>r.id===selectedId)?.title||''):sectionTitle();
}
function renderActive(){
  const a=state.active,grouped=captureShown()&&captureKnown();
  // With the capture group, Record/Stop and the timer live there; the old status pill and New recording button return only without it.
  $('activeStatus').hidden=!a||grouped;
  if(a){
    setText($('activeLabel'),activeLabel(a));
    const stoppable=a.status==='recording',closable=['needs-access','ready-to-record'].includes(a.status);
    $('stopRecording').hidden=!(stoppable||closable);$('stopRecording').textContent=stoppable?'Stop':'Close';$('stopRecording').title=stoppable?'Stop and save':'Close the recorder preview';
  }
}
// What a card's picture area says. Never a made-up picture or duration: a take's own poster, or one honest word.
function cardMediaState(r,outcome){
  if(!isReady(r))return {kind:'unavailable',note:''};
  if(r.status==='interrupted'&&r.finalized===false)return {kind:'unfinished',note:'Can’t play'};
  if(!r.thumbnailReady)return {kind:'no-poster',note:'No preview image'};
  if(outcome==='blocked')return {kind:'blocked',note:'Preview can’t load here'};
  if(outcome==='loading')return {kind:'loading',note:'Loading preview…'};
  return {kind:'poster',note:''};
}
const playable=r=>r.status==='ready'||(r.status==='interrupted'&&r.finalized===true);
function cardBadge(r){
  if(!isReady(r))return `<span class="duration">${escapeHTML(r.status)}</span>`;
  if(r.status==='interrupted'&&r.finalized===false)return '<span class="duration">Not finalized</span>';
  if(isNum(r.duration)&&r.duration>0)return `<span class="duration">${duration(r.duration)}</span>`;
  return r.status==='interrupted'?'<span class="duration">Stopped early</span>':'';
}
function cardMarkup(r){
  const ready=isReady(r),link=bestShare(r.id),st=link&&shareState(link),title=escapeHTML(r.title),cached=CONNECTION.embedded?media.posters.get(r.id):'',settled=media.cards.get(r.id)?.state,
    // In an agent window a card shows the cached picture, or the address once that has been seen to work, and otherwise
    // says it is loading until the loader (loadPosters) has an answer; it never sits as a silent placeholder.
    outcome=!CONNECTION.embedded||cached||media.direct==='ok'?'poster':settled==='blocked'?'blocked':'loading',stateNow=cardMediaState(r,outcome);
  const src=stateNow.kind==='poster'?(cached||fileURL(r.id,'thumbnail.jpg')):'';
  const badge=cardBadge(r);
  const tag=st==='ready'?'<span class="tag shared"><i aria-hidden="true"></i>Shared</span>':st==='starting'?'<span class="tag">Starting link…</span>':'';
  const meta=[escapeHTML(when(r.createdAt)),r.status==='interrupted'?(r.finalized===false?'Available files kept':'Stopped early'):''].filter(Boolean).map(x=>`<span>${x}</span>`).join('');
  const note=stateNow.note&&(stateNow.kind!=='loading'||CONNECTION.embedded)?`<span class="media-note">${stateNow.note}</span>`:'';
  return `<article class="card" data-id="${escapeHTML(r.id)}" data-media="${stateNow.kind}"><button class="thumbnail" data-open tabindex="-1" aria-hidden="true"><span class="poster-fallback">${poster}</span>${src?`<img ${CONNECTION.embedded?'':'loading="lazy" '}data-poster="${escapeHTML(r.id)}" src="${escapeHTML(src)}" alt="">`:''}${badge}${note}</button><div class="card-body"><button class="card-title" data-open title="${title}">${title}</button><p class="card-meta">${meta}${r.favorite?'<span class="fav" role="img" aria-label="Favorite">★</span>':''}${tag}</p></div><button class="more" data-more aria-label="More actions for ${title}" aria-haspopup="menu" aria-expanded="false">${svg('more')}</button></article>`;
}
function emptyCopy(query){
  if(query)return [`No recordings match “${clip(query,40)}”`,'Try another word or clear the search.'];
  if(view==='shared')return ['Nothing is shared right now','Open a completed recording and choose Share to create a protected link.'];
  if(view==='favorites')return ['No favorites yet','Use ••• › Add to Favorites on any recording.'];
  if(view==='archived')return ['Nothing archived','Archived recordings appear here. Their files stay on disk.'];
  if(view.startsWith('folder:'))return ['This folder is empty','Use ••• › Move to Folder on a recording.'];
  return ['No recordings yet','Press Record to make your first one.'];
}
function renderGrid(){
  const records=filtered(),signature=JSON.stringify([records,state.shares.map(s=>[s.id,s.shareId,shareState(s)]),state.folders.length]);
  if(gridSignature!==signature){
    gridSignature=signature;$('recordings').innerHTML=records.map(cardMarkup).join('');
    if(menuState&&!menuState.detail){const trigger=document.querySelector(`.card[data-id="${CSS.escape(menuState.id)}"] [data-more]`);if(trigger){menuState.trigger=trigger;setExpanded(trigger,true);}else closeMenu();}
    if(CONNECTION.embedded)void loadPosters();
  }
  $('empty').hidden=records.length>0;
  if(!records.length){const [title,text]=emptyCopy($('search').value.trim());$('emptyTitle').textContent=title;$('emptyText').textContent=text;}
}
const folderNote='Filing a recording in a folder never moves its files.';
const fileKinds=[[/^recording\./,'Combined'],[/^screen\./,'Screen'],[/^camera\./,'Camera']];
const fileKind=name=>fileKinds.find(([re])=>re.test(name))?.[1];
function fileLabel(r,f){const kind=fileKind(f.name);return kind==='Combined'&&r.files.length===1&&r.settings?.camera===false?'Screen recording':kind==='Screen'||kind==='Camera'?`${kind} original`:kind||f.name;}
function detailRows(r){
  const rows=[['Recorded',when(r.createdAt)]];
  // Only a duration the take's own metadata gave; otherwise it is said to be unavailable, never estimated.
  if(isNum(r.duration)&&r.duration>0)rows.push(['Duration',duration(r.duration)]);else if(r.status==='interrupted')rows.push(['Duration','Unavailable']);
  const source=typeof r.source==='string'?r.source:typeof r.source?.name==='string'?r.source.name:'';
  if(source)rows.push(['Source',source]);
  if(typeof r.settings?.camera==='boolean')rows.push(['Camera',r.settings.camera?`On${r.settings.cameraShape?` · ${shapeNames[r.settings.cameraShape]||r.settings.cameraShape}`:''}`:'Off']);
  if(typeof r.settings?.microphone==='boolean')rows.push(['Microphone',r.settings.microphone?'On':'Off']);
  if(isNum(r.settings?.fps))rows.push(['Frame rate',`${r.settings.fps} fps set`]);
  return rows.filter(([,v])=>v);
}
function renderRecording(){
  const r=state.recordings.find(x=>x.id===selectedId);if(!r)return;
  const canMove=isReady(r),signature=JSON.stringify([r,state.folders,view]);
  $('backLabel').textContent=sectionTitle();
  $('favoriteButton').setAttribute('aria-pressed',!!r.favorite);$('favoriteButton').setAttribute('aria-label',r.favorite?'Remove from Favorites':'Add to Favorites');$('favoriteButton').title=r.favorite?'Remove from Favorites':'Add to Favorites';
  $('shareButton').disabled=!canShare(r);$('shareButton').title=canShare(r)?'':r.archived?'Unarchive to share':'Only completed recordings can be shared';
  if(recSignature===signature)return;recSignature=signature;
  const refocus=document.activeElement?.id==='moveFolder';
  $('detailTitle').textContent=r.title;$('detailMeta').textContent=[when(r.createdAt),isNum(r.duration)&&r.duration>0?duration(r.duration):''].filter(Boolean).join(' · ');
  const folderOptions=`<option value="">No folder</option>`+state.folders.map(f=>`<option value="${escapeHTML(f.id)}" ${f.id===r.folder?'selected':''}>${escapeHTML(f.name)}</option>`).join('');
  const row=([k,v])=>`<div><dt>${k}</dt><dd>${escapeHTML(v)}</dd></div>`,rows=detailRows(r),early=['Recorded','Duration'];
  $('detailRows').innerHTML=rows.filter(x=>early.includes(x[0])).map(row).join('')+`<div><dt><label for="moveFolder">Folder</label></dt><dd><select id="moveFolder" ${canMove?'':'disabled'}>${folderOptions}</select></dd></div>`+rows.filter(x=>!early.includes(x[0])).map(row).join('');
  if(refocus)$('moveFolder').focus();
  $('moveNote').textContent=folderNote;
  $('detailFiles').innerHTML=r.files.map(f=>`<div class="file-row"><div class="file-info"><span>${escapeHTML(fileLabel(r,f))}</span><small>${[isNum(f.bytes)?bytes(f.bytes):'',f.width&&f.height?`${f.width} × ${f.height}`:'',isNum(f.duration)&&f.duration>0?duration(f.duration):''].filter(Boolean).join(' · ')}</small></div><button data-open-file="${escapeHTML(f.name)}">Open file ↗</button></div>`).join('');
  $('originalNote').textContent=r.status==='interrupted'?(r.error||'This take is incomplete. Available files have been preserved.'):r.files.some(f=>/^camera\./.test(f.name))?'The camera original keeps the full, unmirrored frame.':r.files.length===1&&r.settings?.camera===false?'This take has one screen recording file; no separate screen original was saved.':r.files.length?'':'No playable files are available for this take.';
  $('detailPath').textContent=r.directory||'';$('revealButton').textContent=`Show in ${state.platform==='darwin'?'Finder':'folder'}`;$('revealButton').hidden=!r.directory;
  const tabs=r.files.filter(f=>fileKind(f.name));
  $('playerTabs').hidden=tabs.length<2;
  $('playerTabs').innerHTML=tabs.map(f=>`<button role="tab" data-file="${escapeHTML(f.name)}" aria-selected="${f.name===playerFile}" tabindex="${f.name===playerFile?0:-1}">${fileKind(f.name)}</button>`).join('');
  renderStopped(r);
}
// A stopped take, said briefly and truthfully: what was kept, nothing about whose fault it was, and the raw reason only
// behind Technical details. A player appears only for a take whose files were actually finalized.
function stoppedCopy(r){
  if(r.status!=='interrupted')return null;
  if(r.finalized===false)return {title:'This recording couldn’t be finished.',text:'Its original files are kept.',player:false};
  return {title:'Recording stopped early.',text:'Available files were kept.',player:r.finalized===true};
}
function techFacts(r){
  const c=isObj(r.capture)?r.capture:{},rows=[];
  if(isNum(c.width)&&isNum(c.height))rows.push(['Capture size',`${c.width} × ${c.height}`]);
  if(isNum(r.settings?.fps))rows.push(['Frame rate set',`${r.settings.fps} fps`]);
  if(isNum(c.outputFps))rows.push(['Frame rate written',`${c.outputFps.toFixed(1)} fps`]);
  if(isNum(c.sourceFps))rows.push(['Screen delivered',`${c.sourceFps.toFixed(1)} fps`]);
  if(isStr(c.finalization))rows.push(['Finalization',c.finalization]);
  if(isObj(c.encoders))rows.push(['Encoders',Object.entries(c.encoders).map(([k,v])=>`${k}: ${v}`).join(' · ')]);
  rows.push(['Files',r.files.length?r.files.map(f=>f.name).join(', '):'none']);
  return rows;
}
function renderStopped(r){
  const note=stoppedCopy(r);
  $('stopNote').hidden=!note;$('techBox').hidden=!note;
  if(!note){$('playerNote').hidden=true;$('playerBox').hidden=false;return;}
  setText($('stopNoteTitle'),note.title);setText($('stopNoteText'),note.text);
  $('playerBox').hidden=!note.player;
  $('playerNote').hidden=note.player;if(!note.player)setText($('playerNote'),'There is no player for this recording. Show it in the folder to open its files.');
  const facts=techFacts(r);
  $('techBody').innerHTML=(r.error?`<pre>${escapeHTML(r.error)}</pre>`:'')+`<dl>${facts.map(([k,v])=>`<div><dt>${escapeHTML(k)}</dt><dd>${escapeHTML(String(v))}</dd></div>`).join('')}</dl>`;
  $('techCopy').dataset.copy=[r.error||'',...facts.map(([k,v])=>`${k}: ${v}`)].filter(Boolean).join('\n');
  const fps=state.settings.fps;
  $('techFps').innerHTML=[24,30,60].map(v=>`<button role="radio" aria-checked="${v===fps}" data-fps="${v}">${v} fps</button>`).join('');
}
$('techCopy').onclick=()=>void copyText($('techCopy').dataset.copy||'','Details copied',()=>{});
$('techFps').onclick=async e=>{
  const b=e.target.closest('[data-fps]');if(!b||!state||state.active)return;
  // The person's choice for the next recordings only; saved like any other frame-rate choice.
  await savePreference('fps',Number(b.dataset.fps));
};
function setPlayerFile(name,keepTime){
  const player=$('player'),time=player.currentTime,wasPlaying=!player.paused;playerFile=name;const tok=++media.playTok;
  document.querySelectorAll('#playerTabs [data-file]').forEach(b=>{const on=b.dataset.file===name;b.setAttribute('aria-selected',on);b.tabIndex=on?0:-1;});
  player.dataset.via='';showPlayerNote('');
  const src=name?fileURL(selectedId,name):'';if(!src){player.removeAttribute('src');player.load();if(name)void playerFailed();return;}
  player.src=src;player.playbackRate=Number($('speed').value);
  // An agent window can leave a request neither loaded nor failed: if nothing has arrived in time, take the bridge route.
  clearTimeout(media.playTimer);
  if(CONNECTION.embedded)media.playTimer=setTimeout(()=>{if(tok===media.playTok&&media.played.tok!==tok&&!(player.readyState>0)&&!player.dataset.via)void playerFailed();},POSTER_WAIT+2000);
  if(keepTime)player.addEventListener('loadedmetadata',()=>{player.currentTime=time;if(wasPlaying)void player.play().catch(()=>{});},{once:true});
}
function openRecording(id){
  const r=state.recordings.find(r=>r.id===id);if(!r)return;
  if(!isReady(r)){toast(r.error||'This recording is still being prepared.');return;}
  closeMenu();selectedId=id;recSignature='';
  playerFile=playable(r)?(r.files.find(f=>/^recording\./.test(f.name))||r.files[0])?.name||'':'';
  if(!CONNECTION.embedded&&!pushed){try{history.pushState({recording:id},'');pushed=true;}catch{}}
  render();setPlayerFile(playerFile);$('speed').value='1';scrollTo(0,0);$('detailTitle').focus();
}
function closeRecording(){
  if(!selectedId)return;
  const id=selectedId;selectedId=null;recSignature='';playerFile='';media.playTok++;clearTimeout(media.playTimer);$('player').pause();$('player').removeAttribute('src');$('player').load();
  if($('shareDialog').open&&sharing?.id===id)$('shareDialog').close();
  if(pushed){pushed=false;try{history.back();}catch{}}
  if(state)render();
  const card=document.querySelector(`.card[data-id="${CSS.escape(id)}"] .card-title`);(card||$('search')).focus();
}
addEventListener('popstate',()=>{if(selectedId){pushed=false;closeRecording();}});
let menuState=null;
function menuItems(r,stage,detail){
  if(stage==='move')return [`<button role="menuitem" data-command="back">${svg('back')}<span>Move to Folder</span></button>`,`<button role="menuitemradio" aria-checked="${!r.folder}" data-command="moveTo" data-folder="">${r.folder?'<i></i>':svg('check')}<span>No folder</span></button>`,...state.folders.map(f=>`<button role="menuitemradio" aria-checked="${f.id===r.folder}" data-command="moveTo" data-folder="${escapeHTML(f.id)}">${f.id===r.folder?svg('check'):'<i></i>'}<span>${escapeHTML(f.name)}</span></button>`),`<small>${folderNote}</small>`];
  const ready=isReady(r);
  return [...(detail?[]:['<button role="menuitem" data-command="open">Open</button>']),'<button role="menuitem" data-command="rename">Rename</button>',`<button role="menuitem" aria-haspopup="menu" data-command="move" ${ready?'':'disabled'}><span>Move to Folder</span>${svg('next')}</button>`,`<button role="menuitem" data-command="favorite">${r.favorite?'Remove from':'Add to'} Favorites</button>`,`<button role="menuitem" data-command="share" ${canShare(r)?'':'disabled'}>Share…</button>`,`<button role="menuitem" data-command="reveal">Show in ${state.platform==='darwin'?'Finder':'folder'}</button>`,`<button role="menuitem" data-command="archive">${r.archived?'Unarchive':'Archive'}</button>`,`<small>Files stay on disk.${r.archived?'':' Archiving stops shared links.'}</small>`];
}
function openMenu(id,trigger,stage='main',detail=false){
  const r=state.recordings.find(r=>r.id===id);if(!r)return;
  if(menuState&&menuState.trigger!==trigger)setExpanded(menuState.trigger,false);
  menuState={id,trigger,stage,detail};const menu=$('menu');
  menu.innerHTML=menuItems(r,stage,detail).join('');menu.hidden=false;menu.setAttribute('aria-label',`Actions for ${clip(r.title,40)}`);setExpanded(trigger,true);
  const box=trigger.getBoundingClientRect(),width=menu.offsetWidth,height=menu.offsetHeight;
  menu.style.left=`${Math.max(8,Math.min(innerWidth-width-8,box.right-width))}px`;
  menu.style.top=`${box.bottom+4+height>innerHeight&&box.top-4-height>8?box.top-4-height:box.bottom+4}px`;
  menu.querySelector('button:not(:disabled)')?.focus();
}
function setExpanded(el,on){if(el?.setAttribute)el.setAttribute('aria-expanded',on);}
function closeMenu(restore){
  if(!menuState)return;const {trigger}=menuState;menuState=null;$('menu').hidden=true;setExpanded(trigger,false);if(restore&&trigger?.isConnected)trigger.focus();
}
async function updateRecording(r,patch,revert,message){
  const value=await act('update',{id:r.id,patch});
  if(value&&message)toast(message,()=>{void act('update',{id:r.id,patch:revert}).then(v=>v&&toast('Change undone.'));});
  return value;
}
async function archiveRecording(r){
  const archived=!r.archived,value=await act('update',{id:r.id,patch:{archived}});if(!value)return;
  if(selectedId===r.id)closeRecording();
  toast(archived?`Archived “${clip(r.title,40)}”. Files stay on disk.`:`Unarchived “${clip(r.title,40)}”.`,()=>{void act('update',{id:r.id,patch:{archived:r.archived}}).then(v=>v&&toast('Archive change undone. Shared links remain stopped.'));});
}
function renameRecording(r){askName('rename',r.title,async title=>{title=title.trim();if(!title)throw new Error('Enter a name.');await api('update',{id:r.id,patch:{title}});await refresh();toast(`Renamed to “${clip(title,40)}”.`,()=>{void act('update',{id:r.id,patch:{title:r.title}});});});}
function moveRecording(r,folder){
  if((folder||null)===(r.folder||null))return;
  const name=folder?folderName(folder):'';
  void updateRecording(r,{folder:folder||null},{folder:r.folder||null},folder?`Moved to “${clip(name,40)}”.`:'Removed from its folder.');
}
function runCommand(command,r,dataset){
  switch(command){
    case 'open':openRecording(r.id);break;
    case 'rename':renameRecording(r);break;
    case 'favorite':void updateRecording(r,{favorite:!r.favorite},{favorite:!!r.favorite},r.favorite?'Removed from Favorites.':'Added to Favorites.');break;
    case 'share':openShare(r.id);break;
    case 'reveal':void act('reveal',{id:r.id});break;
    case 'archive':void archiveRecording(r);break;
    case 'moveTo':moveRecording(r,dataset.folder);break;
  }
}
$('menu').addEventListener('click',e=>{
  const b=e.target.closest('[data-command]');if(!b||b.disabled||!menuState)return;
  const {id,trigger,detail}=menuState,r=state.recordings.find(r=>r.id===id);if(!r)return closeMenu();
  if(b.dataset.command==='move')return openMenu(id,trigger,'move',detail);
  if(b.dataset.command==='back')return openMenu(id,trigger,'main',detail);
  closeMenu(b.dataset.command!=='open');runCommand(b.dataset.command,r,b.dataset);
});
$('menu').addEventListener('keydown',e=>{
  const items=[...$('menu').querySelectorAll('button:not(:disabled)')],i=items.indexOf(document.activeElement);
  if(e.key==='ArrowDown'||e.key==='ArrowUp'||e.key==='Home'||e.key==='End'){e.preventDefault();items[e.key==='Home'?0:e.key==='End'?items.length-1:(i+(e.key==='ArrowDown'?1:-1)+items.length)%items.length]?.focus();}
  if(e.key==='Tab')closeMenu();
  if((e.key==='ArrowLeft'||e.key==='Escape')&&menuState?.stage==='move'){e.preventDefault();e.stopPropagation();openMenu(menuState.id,menuState.trigger,'main',menuState.detail);}
});
document.addEventListener('pointerdown',e=>{if(menuState&&!$('menu').contains(e.target)&&!menuState.trigger.contains(e.target))closeMenu();});
addEventListener('resize',()=>{closeMenu();if(innerWidth>720)closeSidebar();});
let sharing=null;
function openShare(id){
  const r=state.recordings.find(r=>r.id===id);if(!r||!canShare(r))return;
  sharing={id,creating:false,error:null,ended:null,sawReady:false,expiresAt:null,phase:''};
  document.querySelector('input[name=shareMinutes][value="60"]').checked=true;
  $('shareDialog').showModal();renderShare();
}
function sharePhase(link){
  if(sharing.ended)return 'ended';
  if(sharing.creating&&(!link||shareState(link)==='failed'))return 'starting';
  if(link){
    const st=shareState(link);
    if(st==='ready'){sharing.sawReady=true;sharing.expiresAt=link.expiresAt;return 'ready';}
    if(st==='starting')return 'starting';
    if(st==='failed')return 'failed';
    if(st==='expired'){sharing.ended='Link expired. The link and code no longer work.';return 'ended';}
  }else if(sharing.sawReady){
    sharing.ended=isNum(sharing.expiresAt)&&sharing.expiresAt<=Date.now()?'Link expired. The link and code no longer work.':'Link stopped or disconnected. The link and code no longer work. Your recording is unchanged.';return 'ended';
  }
  return sharing.error?'failed':'new';
}
function renderShare(){
  if(!sharing||!$('shareDialog').open||!state)return;
  const r=state.recordings.find(r=>r.id===sharing.id);if(!r){$('shareDialog').close();return;}
  const link=bestShare(r.id),phase=sharePhase(link);
  setText($('shareTarget'),`${clip(r.title,60)} · combined video`);
  ['New','Starting','Ready','Failed','Ended'].forEach(name=>$(`share${name}`).hidden=name.toLowerCase()!==phase);
  // The link and code exist in the DOM only while the backend reports the share ready.
  if(phase!=='ready'){$('shareUrl').value='';$('shareCode').textContent='';$('shareInvite').value='';$('shareInvite').hidden=true;}
  if(phase==='starting'){$('cancelShare').disabled=!link?.shareId;}
  if(sharing.phase!==phase){
    const before=sharing.phase,active=document.activeElement;sharing.phase=phase;
    // Keep keyboard focus inside the dialog when the control that had it just went away.
    if(before&&(!active||active===document.body||active.offsetParent===null))(phase==='ready'?$('copyShare'):$(`share${phase[0].toUpperCase()+phase.slice(1)}`).querySelector('button:not(:disabled)'))?.focus();
  }
  if(phase==='starting')setText($('shareStartingText'),'Starting…');
  if(phase==='ready'){
    if($('shareUrl').value!==shareLink(link))$('shareUrl').value=shareLink(link);setText($('shareCode'),formatCode(shareCode(link)));
    setText($('shareExpiry'),isNum(link.expiresAt)?`Active until ${clock(link.expiresAt)}. Keep this computer awake and OpenRecorder open.`:link.never===true?'No end time. It works while OpenRecorder stays open and this computer is awake and online. Revoke it, archive the recording or quit to end it.':'Keep this computer awake and OpenRecorder open.');
    const sessions=isNum(link.unlockedSessions)?link.unlockedSessions:null;
    $('shareSessions').hidden=sessions===null;if(sessions!==null)setText($('shareSessions'),`${sessions} unlocked session${sessions===1?'':'s'}`);
    $('shareLimited').hidden=link.rateLimited!==true;if(link.rateLimited===true)setText($('shareLimited'),'5 wrong codes in the last minute — unlocking resumes within a minute.');
  }
  if(phase==='failed'){
    const message=String((link&&link.error)||sharing.error||''),missing=/cloudflared/i.test(message);
    setText($('shareFailedTitle'),missing?'Sharing needs cloudflared':sharing.unsure?'Couldn’t confirm sharing':'Couldn’t start sharing');
    setText($('shareFailedText'),missing?'Install cloudflared, then try again. No account is needed. Nothing was shared.':sharing.unsure?clip(message,240):`${clip(message,200)||'The connection didn’t answer.'} Nothing was shared.`);
    $('shareInstall').hidden=!missing;
  }
  if(phase==='ended')setText($('shareEndedText'),sharing.ended);
}
async function startShare(){
  const current=sharing,id=current.id;current.creating=true;current.error=null;current.unsure=false;current.ended=null;renderShare();
  try{await api('share',{id,minutes:Number(document.querySelector('input[name=shareMinutes]:checked').value)},true);}
  catch(e){if(sharing===current)current.error=e.message||'The connection didn’t answer.';current.uncertain=!!e.uncertain;}
  finally{if(sharing===current)current.creating=false;}
  await refresh();
  // An unanswered request may still have created the link: the host's list decides, and nothing is sent again.
  if(sharing===current&&current.uncertain){current.uncertain=false;if(state&&liveShares().some(s=>s.id===id)){current.error=null;current.unsure=false;}else{current.unsure=true;current.error='No answer from OpenRecorder, so it’s unknown whether sharing started. Nothing is shared unless a link appears. Try again if it doesn’t.';}}
  renderShare();
}
async function stopShare(link,message){
  const current=sharing;
  try{await api('unshare',{id:link.id,...(link.shareId?{shareId:link.shareId}:{})});if(current&&sharing===current)current.ended=message;await refresh();renderShare();}
  catch(e){showError(e);}
}
$('createShare').onclick=startShare;
$('retryShare').onclick=async()=>{const link=bestShare(sharing.id);if(link?.shareId){try{await api('unshare',{id:link.id,shareId:link.shareId});}catch{}}sharing.error=null;void startShare();};
$('cancelShare').onclick=()=>{const link=bestShare(sharing.id);if(link)void stopShare(link,'Sharing cancelled. Nothing was shared.');};
$('revokeShare').onclick=()=>{const link=bestShare(sharing.id);if(link)void stopShare(link,'Link revoked. The link and code no longer work. Your recording is unchanged.');};
$('newShare').onclick=()=>{sharing={id:sharing.id,creating:false,error:null,ended:null,sawReady:false,expiresAt:null,phase:''};renderShare();};
function selectContents(el){const range=document.createRange();range.selectNodeContents(el);const selection=getSelection();selection.removeAllRanges();selection.addRange(range);}
async function copyText(text,message,fallback){try{await navigator.clipboard.writeText(text);toast(message);}catch{if(fallback)fallback();toast('Select it and copy manually.');}}
$('copyShare').onclick=()=>{const link=bestShare(sharing.id);if(!link||shareState(link)!=='ready'){renderShare();return;}void copyText(shareLink(link),'Link copied',()=>{$('shareUrl').focus();$('shareUrl').select();});};
$('copyCode').onclick=()=>{const link=bestShare(sharing.id);if(!link||shareState(link)!=='ready'){renderShare();return;}void copyText(formatCode(shareCode(link)),'Code copied',()=>selectContents($('shareCode')));};
// The invitation: a short message with the link and the code together, built from what the host reports for this share. It
// is only ever copied (or shown to be selected); nothing is sent, and no part of it is logged.
function inviteText(link){
  const end=isNum(link.expiresAt)?`The link works until ${clock(link.expiresAt)}.`:link.never===true?'The link has no end time; it works while I keep OpenRecorder open.':'';
  return ['I’m sharing a recording with you.','',`Link: ${shareLink(link)}`,`Access code: ${formatCode(shareCode(link))}`,...(end?['',end]:[])].join('\n');
}
$('copyInvite').onclick=async()=>{
  const dialog=sharing,link=bestShare(dialog.id);if(!link||shareState(link)!=='ready'){renderShare();return;}
  const text=inviteText(link);
  // The clipboard answers later. Whatever it says, nothing is shown or restored unless this very dialog (not one closed, or
  // reopened, meanwhile) is still open, has not ended and is still displaying its ready phase for this very ready share: the
  // invitation holds the link and the code. (A cached copy of the share that outlives a revoke whose refresh failed is not enough.)
  const current=()=>{const now=sharing===dialog&&!dialog.ended&&dialog.phase==='ready'&&$('shareDialog').open&&bestShare(dialog.id);return !!now&&shareState(now)==='ready'&&now.shareId===link.shareId&&shareLink(now)===shareLink(link)&&shareCode(now)===shareCode(link);};
  let copied=false;
  try{await navigator.clipboard.writeText(text);copied=true;}catch{}
  if(!current())return;
  if(copied){$('shareInvite').hidden=true;toast('Invitation copied');return;}
  const box=$('shareInvite');box.value=text;box.hidden=false;box.focus();box.select();toast('Select it and copy manually.');
};
$('shareDialog').addEventListener('close',()=>{sharing=null;$('shareUrl').value='';$('shareCode').textContent='';$('shareInvite').value='';$('shareInvite').hidden=true;});
const resolutions=[[1280,'720p'],[1920,'1080p'],[2560,'1440p']],sizeNames=new Map([[140,'Small'],[200,'Medium'],[280,'Large'],[120,'Small'],[168,'Medium'],[224,'Large']]);
// The host validates the whole set, so every option is always offered. A stored value outside the set (a legacy camera
// size) stays selectable as "Current" and is never replaced unless the person picks another size.
// state.capabilities.settings[key], when the host sends it, replaces the built-in list.
function choices(key,fallback,label){
  const confirmed=state.capabilities?.settings?.[key],current=state.settings[key],list=Array.isArray(confirmed)?[...confirmed]:[...fallback];
  const options=list.map(v=>[v,label(v)]);
  if(current!==undefined&&current!==null&&!list.includes(current))options.push([current,`Current (${current})`]);
  return options;
}
const settingDefs=[
  {key:'theme',pane:'general',label:'Appearance',options:()=>[['system','System'],['light','Light'],['dark','Dark']]},
  {key:'quality',pane:'recording',label:'Resolution',help:'Maximum long edge. The screen’s aspect ratio is preserved.',options:()=>choices('quality',resolutions.map(r=>r[0]),v=>resolutions.find(r=>r[0]===v)?.[1]||`${v}`)},
  {key:'fps',pane:'recording',label:'Frame rate',options:()=>choices('fps',[24,30,60],v=>`${v} fps`)},
  {key:'keepOriginals',pane:'recording',label:'Keep originals',toggle:true,help:'Saves separate screen and camera files next to the combined MP4. Uses more disk. Screen-only recordings never duplicate the screen.'},
  {key:'countdownSeconds',pane:'recording',label:'Countdown',optional:true,options:()=>choices('countdownSeconds',[0,3],v=>v===0?'Off':`${v} seconds`)},
  {key:'rememberInputs',pane:'camera',label:'Remember camera and mic on or off',toggle:true,optional:true,help:'Inputs you left on show as ready the next time. They only start when you press Camera, Mic or Record.'},
  {key:'showCamera',pane:'camera',label:'Show camera',toggle:true,optional:true,help:'Shows the camera bubble on screen and in the combined recording. Turning it off never turns the camera off, and the camera original is still recorded.'},
  {key:'mirror',pane:'camera',label:'Mirror camera preview',toggle:true},
  {key:'cameraShape',pane:'camera',label:'Camera shape',options:()=>choices('cameraShape',['circle','rounded','wide','portrait'],v=>({circle:'Circle',rounded:'Rounded square',wide:'Wide 16:9',portrait:'Portrait 9:16'}[v]||String(v))).map(([v,l])=>[v,v==='square'?'Square (kept)':l])},
  {key:'cameraSize',pane:'camera',label:'Camera size',options:()=>choices('cameraSize',[120,168,224],v=>sizeNames.get(v)||String(v))}
];
const settingPanes=[['general','General'],['recording','Recording'],['camera','Camera & Microphone'],['permissions','Permissions'],['sharing','Sharing'],['about','About']];
let captureSeen=0,captureBusy=false,capturePoll;
let settingsPane='general',settingsSaving=false,appliedTheme=null,settingsChain=Promise.resolve(),settingsStatusTimer,paneSignature='';
const settingShown=def=>state.settings[def.key]!==undefined&&state.settings[def.key]!==null;
function settingRow(def){
  const id=`s-${def.key}`,value=state.settings[def.key],help=def.help?`<small id="${id}-help">${escapeHTML(def.help)}</small>`:'',described=def.help?` aria-describedby="${id}-help"`:'';
  const control=def.toggle?`<input type="checkbox" role="switch" id="${id}" name="${def.key}"${described} ${value?'checked':''}>`:`<select id="${id}" name="${def.key}"${described}>${def.options().map(([v,l])=>`<option value="${escapeHTML(v)}" ${v===value?'selected':''}>${escapeHTML(l)}</option>`).join('')}</select>`;
  return `<div class="row"><div class="row-text"><label for="${id}">${escapeHTML(def.label)}</label>${help}</div>${control}</div>`;
}
function sharingPane(){
  const items=liveShares();
  return `<div class="rows">${items.length?items.map(s=>{const r=state.recordings.find(r=>r.id===s.id),st=shareState(s);return `<div class="row"><div class="row-text"><span class="row-label">${escapeHTML(clip(r?.title||'Recording',60))}</span><small>${st==='ready'?(isNum(s.expiresAt)?`Active until ${clock(s.expiresAt)}`:s.never===true?'No end time':'Active'):'Starting…'}</small></div><button data-revoke="${escapeHTML(s.id)}" data-share="${escapeHTML(s.shareId||'')}">Revoke</button></div>`;}).join(''):'<p class="note">Nothing is shared right now.</p>'}</div><p class="note">Codes and viewer sessions are kept in memory only. Links end when they expire, when you revoke them, or when OpenRecorder quits.</p>`;
}
// Permissions: what the app reports, in words, and a human button per grant. Nothing here asks for or resets access.
function permissionRows(){
  const cap=state.capture,known=captureKnown();
  const screen=!known?'Not reported yet':{allowed:'Allowed','not-asked':'Not asked yet. macOS asks when you first record.',denied:'Off for OpenRecorder'}[cap.screenState]||'Not reported yet';
  const input=kind=>{const m=markOf(cap||{},kind);return !known?'Not reported yet':m==='access-off'?'Off in System Settings':m==='live'?'Allowed. Live now.':'No problem reported';};
  return [['screen','Screen Recording',screen,known&&cap.screenState==='denied'],['camera','Camera',input('camera'),known&&markOf(cap,'camera')==='access-off'],['mic','Microphone',input('microphone'),known&&markOf(cap,'microphone')==='access-off']];
}
function permissionsPane(){
  const open=state.platform==='linux'?'':'<button data-privacy="KIND">Open System Settings</button>';
  const live=captureKnown()?['camera','microphone'].filter(k=>markOf(state.capture,k)==='live').map(k=>k==='camera'?'camera':'microphone'):[];
  return `<div class="rows">${permissionRows().map(([kind,label,status,off])=>`<div class="row"><div class="row-text"><span class="row-label">${label}</span><small>${escapeHTML(status)}</small></div>${open?open.replace('KIND',kind):''}</div>`).join('')}</div><p class="note">${live.length?`Live now: ${live.join(' and ')}. `:''}OpenRecorder never resets or re-requests system access. Agents can’t change permissions.</p>`;
}
function aboutPane(){
  const row=(k,v)=>`<div class="row"><div class="row-text"><span class="row-label">${k}</span></div><span class="value">${escapeHTML(v)}</span></div>`;
  const controls=!captureKnown()?'OpenRecorder isn’t running':state.capture.trayAvailable===false?'Home':state.platform==='darwin'?'Menu bar':'Tray';
  return `<div class="rows">${row('Version',state.version||'Unknown')}${row('Platform',state.platform||'Unknown')}${row('Controls',controls)}${row('Window',CONNECTION.embedded?'Agent app panel':'Browser')}</div>`;
}
function paneMarkup(pane){
  if(pane==='sharing')return sharingPane();
  if(pane==='permissions')return permissionsPane();
  if(pane==='about')return aboutPane();
  const lock=state.active?'<p class="note lock">Settings are locked while a recording is in progress.</p>':'';
  const rows=settingDefs.filter(d=>d.pane===pane&&settingShown(d)).map(settingRow);
  if(pane==='recording')rows.unshift(`<div class="row stack"><div class="row-text"><span class="row-label">Save recordings to</span><small id="diskFree">${isNumber(state.freeBytes)?`${bytes(state.freeBytes)} available on this drive`:'Available disk space could not be measured.'}</small></div><button class="path-button" id="chooseDefaultRoot" aria-describedby="diskFree">${escapeHTML(state.root||'')}</button><small>Change the absolute folder path. Existing recordings stay in their current location.</small></div>`);
  if(pane==='recording')rows.push(fpsCapabilityMarkup());
  const note=pane==='camera'?'<p class="note">Choose and turn on the camera or microphone in the recorder. These settings never turn devices on. Microphone only — system audio isn’t available.</p>':'';
  return `${lock}<fieldset class="rows" ${state.active?'disabled':''}>${rows.join('')}</fieldset>${note}`;
}
function renderSettingsNav(){
  $('settingsNav').innerHTML=settingPanes.map(([id,label])=>`<button role="tab" id="tab-${id}" data-pane="${id}" aria-selected="${id===settingsPane}" aria-controls="settingsPane" tabindex="${id===settingsPane?0:-1}">${escapeHTML(label)}</button>`).join('');
  $('settingsTitle').textContent=settingPanes.find(p=>p[0]===settingsPane)[1];$('settingsPane').setAttribute('aria-labelledby',`tab-${settingsPane}`);
}
const paneKey=()=>JSON.stringify([settingsPane,state.settings,state.root,state.freeBytes,!!state.active,settingsPane==='recording'?[state.capture?.capabilities,state.capture?.phase]:['permissions','about','camera'].includes(settingsPane)?[state.capture?.screenState,state.capture?.inputs,state.capture?.trayAvailable]:0,settingsPane==='sharing'?state.shares.map(s=>[s.id,s.shareId,shareState(s),s.expiresAt]):0]);
function renderSettings(){if(!state)return;paneSignature=paneKey();$('settingsPane').innerHTML=paneMarkup(settingsPane);}
function renderSettingsLive(){if($('settingsDialog').open&&!settingsSaving&&paneKey()!==paneSignature&&!$('settingsPane').contains(document.activeElement))renderSettings();}
function setSettingsStatus(text,kind){clearTimeout(settingsStatusTimer);$('settingsStatus').textContent=text;$('settingsStatus').classList.toggle('error',kind==='error');if(text)settingsStatusTimer=setTimeout(()=>{$('settingsStatus').textContent='';$('settingsStatus').classList.remove('error');},kind==='error'?8000:3000);}
function openSettings(pane){
  if(!state)return;closeMenu();settingsPane=pane||settingsPane;setSettingsStatus('');renderSettingsNav();renderSettings();
  if(!$('settingsDialog').open)$('settingsDialog').showModal();
}
function selectPane(id){settingsPane=id;renderSettingsNav();renderSettings();$('tab-'+id).focus();}
$('settingsNav').onclick=e=>{const b=e.target.closest('[data-pane]');if(b)selectPane(b.dataset.pane);};
$('settingsNav').onkeydown=e=>{
  const keys={ArrowDown:1,ArrowRight:1,ArrowUp:-1,ArrowLeft:-1};if(!(e.key in keys)&&e.key!=='Home'&&e.key!=='End')return;
  e.preventDefault();const ids=settingPanes.map(p=>p[0]),i=ids.indexOf(settingsPane);selectPane(ids[e.key==='Home'?0:e.key==='End'?ids.length-1:(i+keys[e.key]+ids.length)%ids.length]);
};
let savesPending=0;
function saveSetting(el){
  const def=settingDefs.find(d=>d.key===el.name);if(!def||!state)return;
  const previous=state.settings[def.key],next=def.toggle?el.checked:def.options().find(([v])=>String(v)===el.value)?.[0];
  if(next===undefined||next===previous)return;
  const refocus=document.activeElement===el,restore=()=>{if(def.toggle)el.checked=!!previous;else el.value=String(previous);};
  el.disabled=true;settingsSaving=true;savesPending++;setSettingsStatus('Saving…');
  settingsChain=settingsChain.then(async()=>{
    try{
      const result=await api('settings',{preferences:{[def.key]:next}},true);
      // A complete settings object that disagrees is a definite "not saved"; an incomplete or malformed one proves nothing
      // either way, so it is reconciled against the host below instead of being committed.
      if(!settingsShape(result?.settings))throw incompleteSettings();
      if(result.settings[def.key]!==next)throw new Error('The app did not confirm the change.');
      commitSettings(result.settings);if(def.key==='theme')applyTheme(next);setSettingsStatus(`${def.label} saved.`);
    }catch(e){
      if(e.uncertain){
        // No answer: the host may have stored it. Read it back instead of assuming either outcome.
        try{const fresh=await api('state');commitState(fresh);
          if(fresh.settings[def.key]===next){if(def.key==='theme')applyTheme(next);setSettingsStatus(`${def.label} saved.`);}
          else{restore();setSettingsStatus(`${def.label} not saved.`,'error');}}
        catch{restore();setSettingsStatus(`${def.label}: no answer, so it’s unknown whether it was saved. Showing the last saved value.`,'error');}
      }else{restore();setSettingsStatus(`${def.label} not saved. ${clip(e.message,160)}`,'error');}
    }
    finally{
      settingsSaving=--savesPending>0;el.disabled=false;
      if(!settingsSaving){renderSettings();if(refocus)document.getElementById(el.id)?.focus();}
    }
  });
}
// One preference, saved like any Settings change: the answer must confirm it, an unanswered write is read back.
async function savePreference(key,value){
  try{
    const result=await api('settings',{preferences:{[key]:value}},true);
    if(!settingsShape(result?.settings))throw incompleteSettings();
    if(result.settings[key]!==value)throw new Error('The app did not confirm the change.');
    commitSettings(result.settings);render();return true;
  }catch(e){
    if(e.uncertain){try{commitState(await api('state'));render();return state.settings[key]===value;}catch{}}
    showError(e);return false;
  }
}
function applyTheme(theme){appliedTheme=theme;storageSet('local','or-theme',theme);if(theme==='system')delete document.documentElement.dataset.theme;else document.documentElement.dataset.theme=theme;}
$('settingsPane').onchange=e=>{if(e.target.name)saveSetting(e.target);};
$('settingsPane').onclick=e=>{
  if(e.target.closest('#chooseDefaultRoot'))return chooseRoot();
  const privacy=e.target.closest('[data-privacy]');
  if(privacy)return void captureCommand('open-privacy',{kind:privacy.dataset.privacy});
  const revoke=e.target.closest('[data-revoke]');
  if(revoke)void api('unshare',{id:revoke.dataset.revoke,...(revoke.dataset.share?{shareId:revoke.dataset.share}:{})}).then(()=>refresh()).then(()=>toast('Link revoked.')).catch(showError);
};
function chooseRoot(){
  if(state.active)return;
  askName('root',state.root,async root=>{
    if(root===state.root)return;
    // A timeout or cut-off answer is uncertain and gets the generic single read-back (api marks it and refreshes once);
    // an answer that parses but lacks the folder is reconciled explicitly below. Neither replays the write.
    const result=await api('settings',{root});
    // An answer without the folder (or with a malformed disk figure) proves nothing: read the host, assume nothing.
    if(!isStr(result?.root)||('freeBytes' in result&&!nullable(isNumber)(result.freeBytes))){void refresh();throw Object.assign(new Error('OpenRecorder’s answer was incomplete, so it’s unknown whether the folder changed. Showing what is saved.'),{uncertain:true});}
    state.root=result.root;if('freeBytes' in result)state.freeBytes=result.freeBytes;renderSettings();setSettingsStatus('Recording folder saved. Applies to new recordings.');await refresh();
  });
}
const nameKinds={
  rename:{title:'Rename recording',label:'Name',max:120,submit:'Save',text:''},
  folder:{title:'New folder',label:'Folder name',max:60,submit:'Create',text:`Folders organize recordings in the Library. ${folderNote}`},
  root:{title:'Recording folder',label:'Absolute folder path',max:4096,submit:'Save',text:'Enter an absolute folder path. Existing recordings stay in their current location.'}
};
function askName(kind,current,callback){
  const k=nameKinds[kind];$('nameTitle').textContent=k.title;$('nameLabel').textContent=k.label;$('nameInput').value=current||'';$('nameInput').maxLength=k.max;$('nameDescription').textContent=k.text;$('nameSubmit').textContent=k.submit;$('nameError').textContent='';nameCallback=callback;
  $('nameDialog').showModal();$('nameInput').focus();$('nameInput').select();
}
$('nameForm').onsubmit=async e=>{e.preventDefault();const submit=$('nameSubmit');submit.disabled=true;$('nameError').textContent='';try{if(nameCallback)await nameCallback($('nameInput').value);$('nameDialog').close();}catch(error){$('nameError').textContent=clip(error.message,200);}finally{submit.disabled=false;}};
const sheetOpen=()=>$('app').classList.contains('sheet-open');
// The sheet is not a native dialog, so the page behind it is made inert while it is open and Tab wraps inside it.
// Focus moves only when the person opens or closes the sheet.
function openSidebar(){$('app').classList.add('sheet-open');$('content').inert=true;$('scrim').hidden=false;$('menuButton').setAttribute('aria-expanded','true');$('sidebar').querySelector('.nav-item')?.focus();}
function closeSidebar(restore){if(!sheetOpen())return;$('app').classList.remove('sheet-open');$('content').inert=false;$('scrim').hidden=true;$('menuButton').setAttribute('aria-expanded','false');if(restore)$('menuButton').focus();}
$('sidebar').addEventListener('keydown',e=>{
  if(e.key!=='Tab'||!sheetOpen())return;
  const items=[...$('sidebar').querySelectorAll('button:not(:disabled)')];if(!items.length)return;
  const first=items[0],last=items[items.length-1],here=document.activeElement;
  if(e.shiftKey&&here===first){e.preventDefault();last.focus();}else if(!e.shiftKey&&(here===last||!items.includes(here))){e.preventDefault();first.focus();}
});
$('sheetClose').onclick=()=>closeSidebar(true);
$('menuButton').onclick=()=>$('app').classList.contains('sheet-open')?closeSidebar(true):openSidebar();
$('scrim').onclick=()=>closeSidebar(true);
$('searchToggle').onclick=()=>{const open=$('topbar').dataset.search!=='open';$('topbar').dataset.search=open?'open':'';$('searchToggle').setAttribute('aria-expanded',open);if(open)$('search').focus();};
function go(next){if(selectedId)closeRecording();view=next;gridSignature='';closeSidebar();render();}
$('sidebar').onclick=e=>{const b=e.target.closest('[data-view]');if(b)go(b.dataset.view);};
$('newFolder').onclick=()=>{closeSidebar();askName('folder','',async name=>{name=name.trim();if(!name)throw new Error('Enter a folder name.');if(state.folders.some(f=>f.name.toLowerCase()===name.toLowerCase()))throw new Error('A folder with this name already exists.');const f=await api('folder',{name});await refresh();go(`folder:${f.id}`);});};
$('settingsButton').onclick=()=>{closeSidebar();openSettings();};
$('search').oninput=()=>{gridSignature='';if(state)render();};$('sort').onchange=()=>{gridSignature='';if(state)render();};
$('recordings').addEventListener('error',e=>{if(e.target.tagName==='IMG')e.target.remove();},true);
$('recordings').onclick=e=>{
  const card=e.target.closest('.card');if(!card)return;
  const r=state.recordings.find(r=>r.id===card.dataset.id);if(!r)return;
  if(e.target.closest('[data-more]')){const b=e.target.closest('[data-more]');return menuState?.trigger===b?closeMenu(true):openMenu(r.id,b);}
  if(e.target.closest('[data-open]'))openRecording(r.id);
};
$('backButton').onclick=closeRecording;
$('detailTitle').onclick=()=>{const r=state.recordings.find(r=>r.id===selectedId);if(r)renameRecording(r);};
$('favoriteButton').onclick=()=>{const r=state.recordings.find(r=>r.id===selectedId);if(r)runCommand('favorite',r);};
$('shareButton').onclick=()=>openShare(selectedId);
$('detailMore').onclick=()=>menuState?.trigger===$('detailMore')?closeMenu(true):openMenu(selectedId,$('detailMore'),'main',true);
$('revealButton').onclick=()=>act('reveal',{id:selectedId});
$('detailFiles').onclick=e=>{const b=e.target.closest('[data-open-file]');if(b)void act('open-file',{id:selectedId,name:b.dataset.openFile});};
$('detailRows').onchange=e=>{if(e.target.id!=='moveFolder')return;const r=state.recordings.find(r=>r.id===selectedId),select=e.target;select.disabled=true;moveRecording(r,select.value);setTimeout(()=>{select.disabled=false;},0);};
$('playerTabs').onclick=e=>{const b=e.target.closest('[data-file]');if(b)setPlayerFile(b.dataset.file,true);};
$('playerTabs').onkeydown=e=>{
  if(!['ArrowLeft','ArrowRight','Home','End'].includes(e.key))return;e.preventDefault();
  const tabs=[...$('playerTabs').querySelectorAll('[data-file]')],i=tabs.findIndex(t=>t.dataset.file===playerFile),next=tabs[e.key==='Home'?0:e.key==='End'?tabs.length-1:(i+(e.key==='ArrowRight'?1:-1)+tabs.length)%tabs.length];
  setPlayerFile(next.dataset.file,true);next.focus();
};
$('speed').onchange=()=>$('player').playbackRate=Number($('speed').value);
$('stopRecording').onclick=()=>act('stop');
$('reachRetry').onclick=()=>{reach.manual=true;void refresh();};
$('copyDetails').onclick=()=>void copyText($('reachRaw').textContent,'Details copied',()=>selectContents($('reachRaw')));
document.querySelectorAll('.close-dialog').forEach(b=>b.onclick=()=>b.closest('dialog').close());
document.addEventListener('click',e=>{
  const t=e.target,inside=sel=>!!(t&&t.closest&&t.closest(sel));
  if(pop&&!inside('#capPop')&&!inside('#capSource')&&!inside('#capCamera')&&!inside('#capMic'))closePop();
  if(hostMenuOpen&&!inside('#hostMenu')&&!inside('#hostMenuButton'))closeHostMenu();
  if(t&&t.id==='checkFps')void captureCommand('capacity-check');
});
document.querySelectorAll('dialog').forEach(d=>{d.addEventListener('click',e=>{if(e.target===d){const r=d.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)d.close();}});});
addEventListener('keydown',e=>{
  if(e.key==='Escape'){
    if(pop){e.preventDefault();const a=pop.anchor;closePop();return a?.focus?.();}
    if(hostMenuOpen){e.preventDefault();closeHostMenu();return $('hostMenuButton').focus();}
    if(menuState){e.preventDefault();return closeMenu(true);}
    if(sheetOpen()){e.preventDefault();return closeSidebar(true);}
    if(document.querySelector('dialog[open]'))return;
    if(selectedId){e.preventDefault();return closeRecording();}
  }
  const typing=/INPUT|TEXTAREA|SELECT/.test(document.activeElement.tagName)||document.activeElement.isContentEditable,plain=!e.metaKey&&!e.ctrlKey&&!e.altKey&&!document.querySelector('dialog[open]');
  if(!typing&&plain&&e.key==='/'&&!selectedId){e.preventDefault();if(innerWidth<=720&&$('topbar').dataset.search!=='open')$('searchToggle').click();else $('search').focus();}
  if((e.metaKey||e.ctrlKey)&&e.key===','){e.preventDefault();openSettings();}
});
document.addEventListener('visibilitychange',()=>{if(!document.hidden&&!reach.stopped&&state)void refresh();});
const storedTheme=storageGet('local','or-theme');if(storedTheme&&storedTheme!=='system')document.documentElement.dataset.theme=storedTheme;
async function init(){
  if(denied){lockedView('open');return;}
  try{if(CONNECTION.embedded){const handshake=await rpc('ui/initialize',{appInfo:{name:'OpenRecorder',version:'0.6.0-preview.11'},appCapabilities:{availableDisplayModes:['fullscreen']},protocolVersion:'2026-01-26'});applyHost(handshake.hostContext);parent.postMessage({jsonrpc:'2.0',method:'ui/notifications/initialized'},hostOrigin||'*');}}
  catch(e){reachFail(e);}
  await refresh();
  // The native menu's Settings… opens Home at /settings: the same Settings the sidebar opens.
  if(hashRoute==='settings'&&state)openSettings();
  // The native menu's Open Last Recording opens the newest take that stopped early; nothing starts or changes.
  if(hashRoute==='last'&&state){const r=newestStopped();if(r)openRecording(r.id);}
}

// ---- Capture controls (the person's own) ----------------------------------------------------------------------------
// Everything shown comes from what the app reports; nothing here keeps its own timer or guesses a phase. Commands are
// sent only from a person's click, never on load, refresh, reconnect or by an agent. The page names the launch and take
// it saw and gives every command an expiry and a single-use number, so a request that is delayed, repeated or made for an
// earlier launch is refused by the library and again by the app. In an agent host the page uses an app-only tool the host
// hides from the model; a host without it gets an honest "can't do that here" and the browser.
const MARKS=['off','armed','starting','live','attention','access-off','not-connected','no-video'];
const capturePhase=()=>state?.capture?.phase||'unavailable';
const captureKnown=()=>!!state?.capture&&state.capture.known!==false&&state.capture.phase!=='unavailable';
const controlsAvailable=()=>!denied&&!(CONNECTION.embedded&&uiControl.supported===false);
const captureShown=()=>!!state&&controlsAvailable();
const markOf=(cap,name)=>{const m=cap?.inputs?.[name]?.mark;return MARKS.includes(m)?m:'off';};
function captureElapsed(){
  const cap=state?.capture;if(!cap||cap.phase!=='recording'||cap.stale||!isNumber(cap.elapsedMs))return null;
  return cap.elapsedMs+Math.max(0,Date.now()-captureSeen);
}
const captureClock=ms=>{const s=Math.floor(ms/1000),h=Math.floor(s/3600),m=Math.floor(s/60)%60,r=s%60;return h?`${h}:${String(m).padStart(2,'0')}:${String(r).padStart(2,'0')}`:`${m}:${String(r).padStart(2,'0')}`;};
function captureCopy(cap){
  return {idle:'Record',countdown:isNumber(cap.countdown)?`Cancel ${cap.countdown}`:'Cancel',starting:'Starting…',recording:'Stop',stopping:'Saving…',saving:'Saving…'}[cap.phase]||'Record';
}
// One status vocabulary, the same words as the menu: Off, On, Starting…, Live, Live · bubble hidden, Not connected,
// Access off, No video. A word never appears without being true, and "On" is never drawn as Live.
function deviceView(kind){
  const noun=kind==='camera'?'Camera':'Microphone',known=captureKnown(),cap=state.capture;
  // Before the app reports, only the person's own saved choice is known, and no device name is.
  const mark=known?markOf(cap,kind):state.settings[kind]===true?'armed':'off';
  const name=known?String(cap.inputs?.[kind]?.name||''):'';
  const hidden=kind==='camera'&&appearance().showCamera===false;
  const word={off:'Off',armed:'On',starting:'Starting…',live:hidden?'Live · bubble hidden':'Live','not-connected':'Not connected','access-off':'Access off','no-video':'No video',attention:'Needs attention'}[mark];
  const css=mark==='off'?'':['armed','starting','live'].includes(mark)?mark:'attention';
  return {noun,mark,css,word,name,on:['armed','starting','live'].includes(mark),hint:mark==='armed'?'starts when you record':''};
}
function deviceTitle(v){return `${v.noun}${v.name?`, ${v.name}`:''}, ${v.word}${v.hint?`, ${v.hint}`:''}`;}
function sourceName(){return clip(state.capture?.source?.name||'Display',40);}
function renderCapture(){
  const group=$('capture'),show=captureShown(),known=captureKnown();
  group.hidden=!show;$('capLine').hidden=true;$('capNotice').hidden=true;$('hostMenuBox').hidden=!CONNECTION.embedded;
  const lines=[];
  if(state&&CONNECTION.embedded&&uiControl.supported===false)lines.push('This window can’t start or stop recordings. Open in browser from the ••• menu to record.');
  if(!show){if(lines.length){$('capLine').hidden=false;setText($('capLine'),lines.join(' '));}closePop();return;}
  const cap=known?state.capture:{phase:'idle',generation:0},phase=cap.phase,idle=phase==='idle',denied=known&&cap.screenState==='denied',inTake=known&&!idle;
  const record=$('capRecord');
  record.dataset.phase=phase;record.dataset.blocked=String(idle&&denied);
  setText($('capRecordLabel'),captureCopy(cap));
  record.disabled=['stopping','saving'].includes(phase)||(idle&&denied)||(known&&cap.stale===true&&!idle);
  record.setAttribute('aria-label',idle?'Record':phase==='recording'?'Stop recording':phase==='countdown'?'Cancel the countdown':captureCopy(cap));
  const elapsed=captureElapsed();$('capTime').hidden=elapsed===null;if(elapsed!==null)setText($('capTime'),captureClock(elapsed));
  setText($('capSourceName'),sourceName());$('capSource').disabled=inTake;$('capSource').title=inTake?'Can’t change during a recording':`Source, ${sourceName()}`;
  for(const [id,nameId,kind] of [['capCamera','capCameraName','camera'],['capMic','capMicName','microphone']]){
    const button=$(id),v=deviceView(kind);
    setText($(nameId),clip(v.name||v.noun,28));button.dataset.mark=v.css;button.title=deviceTitle(v);button.setAttribute('aria-label',deviceTitle(v));
    // The camera's look stays adjustable during a take; its device, and the microphone altogether, do not.
    button.disabled=kind==='camera'?['stopping','saving'].includes(phase):inTake;
  }
  if(idle&&denied)lines.push('Screen Recording is off for OpenRecorder. Open System Settings › Privacy & Security.');
  if(known&&cap.trayAvailable===false)lines.push('No tray on this desktop — capture controls are here in Home.');
  if(lines.length){$('capLine').hidden=false;setText($('capLine'),lines.join(' '));}
  renderNotice(idle&&known?cap.notice:null);
  if(pop)renderPop();
}
// After a stopped take: one calm line and one next step. The raw reason lives on the recording's Technical details.
function renderNotice(n){
  const show=!!n&&n.kind!=='info'&&!!n.text;
  $('capNotice').hidden=!show;if(!show)return;
  const kept=n.kept!==false;
  setText($('capNoticeText'),kept?'Recording stopped early':clip(String(n.text),160));
  setText($('capNoticeSub'),kept?'Available files were kept.':'');
  $('capOpenTake').hidden=!kept||!newestStopped();
}
const newestStopped=()=>[...(state?.recordings||[])].filter(r=>r.status==='interrupted').sort((a,b)=>new Date(b.createdAt).getTime()-new Date(a.createdAt).getTime())[0];
// A fresh single-use request number (128 random bits).
function newRequestNumber(){
  const bytes=new Uint8Array(16),c=globalThis.crypto;
  if(c&&c.getRandomValues)c.getRandomValues(bytes);else for(let i=0;i<16;i++)bytes[i]=Math.floor(Math.random()*256);
  return [...bytes].map(b=>b.toString(16).padStart(2,'0')).join('');
}
// A press is ignored only while the very same command is still being sent (a double press), never another command.
const sending=new Map();
function guard(key){if(sending.has(key))return false;sending.set(key,setTimeout(()=>sending.delete(key),700));return true;}
function afterCommand(){clearTimeout(capturePoll);capturePoll=setTimeout(()=>void refresh(),450);setTimeout(()=>void refresh(),1500);}
async function captureCommand(command,extra){
  if(!captureKnown()||!controlsAvailable())return;
  const cap=state.capture;if(!isStr(cap.launch)||!cap.launch){void refresh();return;}
  if(!guard(`${command}:${extra?.kind||''}`))return;
  const needsTake=['record','stop','cancel'].includes(command);
  try{await uiApi('capture-command',{command,launch:cap.launch,nonce:newRequestNumber(),expires:Date.now()+4000,...(needsTake?{generation:cap.generation}:{}),...extra});}catch(e){showError(e);return;}
  afterCommand();
}
// The person's one press on Record. If the app is up it starts the countdown; if it is not, the press starts it and
// the library starts the countdown once when it reports. Either way it is one press and one recording.
async function startRecording(){
  if(!controlsAvailable()||!guard('record'))return;
  const cap=state?.capture,session=captureKnown()&&isStr(cap.launch)&&cap.launch?{launch:cap.launch,generation:cap.generation}:{};
  try{await uiApi('start-recording',{nonce:newRequestNumber(),expires:Date.now()+4000,...session});}catch(e){showError(e);void refresh();return;}
  if(!session.launch)toast('Starting OpenRecorder…');
  afterCommand();
}
function pressRecord(){
  if($('capRecord').disabled||!controlsAvailable())return;
  const known=captureKnown(),phase=capturePhase();
  if(known&&phase==='recording')return void captureCommand('stop');
  if(known&&(phase==='countdown'||phase==='starting'))return void captureCommand('cancel');
  if(!known||phase==='idle')return void startRecording();
}
$('capRecord').onclick=pressRecord;
$('capOpenTake').onclick=()=>{const r=newestStopped();if(r)openRecording(r.id);void captureCommand('dismiss-notice');};
// ---- Popovers: Source, Camera, Microphone. They stay open while the person chooses, and close on Esc or an outside click.
let pop=null;
const SIZES=[[120,'S'],[168,'M'],[224,'L']],SHAPES=[['circle','Circle'],['rounded','Rounded'],['wide','Wide'],['portrait','Portrait']];
const sizeClassOf=n=>n===120||n===140?120:n===224||n===280?224:168;
function sourceChoices(){
  const list=Array.isArray(state?.capture?.sources)?state.capture.sources:[];
  return list.filter(s=>isNumber(s.id)&&['display','window'].includes(s.kind)).map(s=>({id:s.id,kind:s.kind,name:clip(String(s.name||`Display ${s.id}`),40),selected:s.selected===true}));
}
function deviceChoices(kind){
  const list=state?.capture?.devices?.[kind];
  return Array.isArray(list)?list.filter(d=>isStr(d.name)).map(d=>({uid:isStr(d.uid)?d.uid:'',name:clip(d.name,60),selected:d.selected===true})):[];
}
const switchRow=(label,on,attrs,off)=>`<div class="pop-row"><span>${label}</span><button role="switch" aria-checked="${on}" ${attrs} ${off?'disabled':''}>${on?'On':'Off'}</button></div>`;
const segRow=(items,current,attr,disabled)=>`<div class="seg ${items.length===3?'three':''}" role="radiogroup">${items.map(([v,l])=>`<button role="radio" aria-checked="${v===current}" data-pop="${attr}" data-value="${v}" ${disabled?'disabled':''}>${l}</button>`).join('')}</div>`;
// The camera's look as it is applied now. During a take a change is applied live by the app and saved only once the take is
// over, so then Home shows what the app reports; otherwise the saved preference is the truth. Either way the next press
// reverses what is shown (a hidden bubble offers Show, mirror off offers On).
function appearance(){
  const s=state.settings,a=state.capture?.appearance,live=captureKnown()&&capturePhase()==='recording'&&isObj(a);
  return live?{showCamera:a.showCamera!==false,mirror:a.mirror!==false,cameraShape:a.shape,cameraSize:a.size}:{showCamera:s.showCamera!==false,mirror:s.mirror!==false,cameraShape:s.cameraShape,cameraSize:s.cameraSize};
}
function devicePop(kind){
  const v=deviceView(kind),known=captureKnown(),phase=capturePhase(),idle=known&&phase==='idle',camera=kind==='camera';
  const status=`${v.name?`${v.name} · `:''}${v.word}${v.hint?` — ${v.hint}`:''}`;
  const devices=deviceChoices(kind),lockedText=known&&!idle?'<p class="pop-lock">Unavailable while recording</p>':'';
  let html=`<h3>${v.noun}</h3><p class="pop-status">${escapeHTML(status)}</p>`;
  html+=switchRow(v.noun,v.on,`data-pop="toggle" data-kind="${kind}"`,!idle||v.mark==='access-off');
  html+=lockedText;
  if(known){
    html+=devices.length?`<div class="pop-list" role="menu">${devices.map(d=>`<button class="pop-item" role="menuitemradio" aria-checked="${d.selected}" data-pop="device" data-kind="${kind}" data-uid="${escapeHTML(d.uid)}" ${idle?'':'disabled'}>${escapeHTML(d.name)}</button>`).join('')}</div>`:`<p class="pop-foot">${v.mark==='access-off'?'Access is off in System Settings.':'No devices listed yet.'}</p>`;
  }else html+='<p class="pop-foot">Devices appear when OpenRecorder is running.</p>';
  if(camera){
    const live=!known||idle||phase==='recording',look=appearance();
    html+=switchRow('Show camera',look.showCamera,'data-pop="show"',!live);
    html+=segRow(SHAPES,SHAPES.some(([k])=>k===look.cameraShape)?look.cameraShape:'',  'shape',!live);
    html+=segRow(SIZES,sizeClassOf(look.cameraSize),'size',!live);
    html+=switchRow('Mirror preview',look.mirror,'data-pop="mirror"',!live);
  }
  return html+'<p class="pop-foot">Stays open while you choose. Esc closes.</p>';
}
function sourcePop(){
  const known=captureKnown(),idle=known&&capturePhase()==='idle',list=sourceChoices();
  let html=`<h3>Source</h3><p class="pop-status">${escapeHTML(sourceName())}</p>`;
  if(known&&!idle)return html+'<p class="pop-lock">Can’t change during a recording</p>';
  if(!known)return html+'<p class="pop-foot">Sources appear when OpenRecorder is running.</p>';
  html+=`<div class="pop-list" role="menu">${list.map(s=>`<button class="pop-item" role="menuitemradio" aria-checked="${s.selected}" data-pop="source" data-id="${s.id}" data-kind="${s.kind}">${escapeHTML(s.name)}</button>`).join('')}<button class="pop-item" role="menuitem" data-pop="window" aria-checked="false">Choose Window…</button></div>`;
  return html+'<p class="pop-foot">The camera bubble can appear in a display recording. Recording a window keeps it out.</p>';
}
function renderPop(){
  if(!pop||!state){return;}
  $('capPop').innerHTML=pop.kind==='source'?sourcePop():devicePop(pop.kind);
}
function openPop(kind,anchor){
  if(pop&&pop.kind===kind)return closePop();
  closePop();pop={kind,anchor};renderPop();$('capPop').hidden=false;anchor.setAttribute('aria-expanded','true');
  const box=anchor.getBoundingClientRect();$('capPop').style.left=`${Math.max(8,Math.min(innerWidth-308,box.left))}px`;$('capPop').style.top=`${box.bottom+6}px`;
}
function closePop(){
  if(!pop)return;pop.anchor?.setAttribute?.('aria-expanded','false');pop=null;$('capPop').hidden=true;
}
$('capSource').onclick=()=>{if(!$('capSource').disabled)openPop('source',$('capSource'));};
$('capCamera').onclick=()=>{if(!$('capCamera').disabled)openPop('camera',$('capCamera'));};
$('capMic').onclick=()=>{if(!$('capMic').disabled)openPop('microphone',$('capMic'));};
// A camera look the app can apply live goes through it; with no app running it is an ordinary saved preference.
function lookChange(command,key,value){return captureKnown()?captureCommand(command,{value}):savePreference(key,value);}
$('capPop').onclick=e=>{
  const b=e.target.closest('[data-pop]');if(!b||b.disabled)return;
  const kind=b.dataset.kind,value=b.dataset.value;
  switch(b.dataset.pop){
    case 'toggle':return void captureCommand('input',{kind:kind==='camera'?'camera':'mic',on:b.getAttribute('aria-checked')!=='true'});
    case 'device':return void captureCommand('device',{kind:kind==='camera'?'camera':'mic',uid:b.dataset.uid||null});
    case 'show':return void lookChange('show-camera','showCamera',b.getAttribute('aria-checked')!=='true');
    case 'mirror':return void lookChange('mirror','mirror',b.getAttribute('aria-checked')!=='true');
    case 'shape':return void lookChange('shape','cameraShape',value);
    case 'size':return void lookChange('size','cameraSize',Number(value));
    case 'source':return void captureCommand('source',{kind:b.dataset.kind,id:Number(b.dataset.id)});
    case 'window':return void captureCommand('choose-window');
  }
};
// ---- The agent window's own menu: Open in Browser is secondary, here, never a primary button.
let hostMenuOpen=false;
function openHostMenu(){
  const menu=$('hostMenu');hostMenuOpen=true;
  menu.innerHTML=['<button role="menuitem" data-host="browser">Open in Browser</button>','<button role="menuitem" data-host="reload">Reload</button>','<button role="menuitem" data-host="settings">Settings</button>'].join('');
  menu.hidden=false;$('hostMenuButton').setAttribute('aria-expanded','true');
  const box=$('hostMenuButton').getBoundingClientRect();menu.style.left=`${Math.max(8,box.right-menu.offsetWidth)}px`;menu.style.top=`${box.bottom+4}px`;
}
function closeHostMenu(){hostMenuOpen=false;$('hostMenu').hidden=true;$('hostMenuButton').setAttribute('aria-expanded','false');}
$('hostMenuButton').onclick=()=>hostMenuOpen?closeHostMenu():openHostMenu();
$('hostMenu').onclick=async e=>{
  const b=e.target.closest('[data-host]');if(!b)return;closeHostMenu();
  if(b.dataset.host==='browser'){try{await api('open-library');toast('Opening the Library in your browser.');}catch(err){showError(err);}}
  else if(b.dataset.host==='reload'){resetMedia();gridSignature='';void refresh();}
  else if(b.dataset.host==='settings')openSettings();
};
// ---- Media. What an embedded window may load is the host's decision (the domains the page declares, the default policy
// when it ignores them, private-network rules), and none of it can be assumed or seen from here. So the page does not
// bet on one route: a poster is read the way the MCP Apps contract provides, as bytes through the page-only tool turned
// into a data: URL (every default image policy allows data:, not blob:), and the library's scoped address is the other
// route. Every step has a deadline and each card ends as the picture or as a card that says why it has none; nothing
// stays a silent placeholder, and nothing is invented.
const MEDIA_BRIDGE_CAP=48*1024*1024,DATA_PLAYBACK_CAP=24*1024*1024,POSTER_CAP=1024*1024,POSTER_WAIT=8000,POSTER_PARALLEL=3;
const POSTER_BYTES=32*1024*1024;
// `epoch` changes on a real Reload: work started before it may finish but never writes a verdict, a picture or a card.
// `playTok` names the current playback source (every open, file switch or close changes it), `playBusy` the one source whose
// fallback read is in flight.
const media={lastSession:0,posters:new Map(),posterBytes:0,budget:POSTER_BYTES,playerUrl:'',bridge:'unknown',direct:'unknown',reasons:{bridge:'',direct:''},cards:new Map(),inflight:new Set(),violations:[],running:false,epoch:0,playTok:0,played:{tok:-1,blob:'',data:''},playTimer:0};
// Which policy blocked what (directive and origin only: never the query, which carries the media key).
try{document.addEventListener('securitypolicyviolation',e=>{if(media.violations.length<8)media.violations.push({directive:String(e.effectiveDirective||e.violatedDirective||'').slice(0,24),blocked:String(e.blockedURI||'').replace(/[?#].*$/,'').slice(0,60)});});}catch{}
function decodeBase64(text){const bin=atob(text),out=new Uint8Array(bin.length);for(let i=0;i<bin.length;i++)out[i]=bin.charCodeAt(i);return out;}
// The pieces of a file, in order. Nothing the host's answer says is taken on trust: each piece must be the piece asked for
// (its own offset), of the file's one size and one media type, no longer than the chunk bound, inside the file, and its
// text must be valid base64 that decodes to exactly the length it declares, so a small declared file can never carry a
// larger payload and the cap and chunk limits hold for the bytes actually received. A piece that is not the last must be
// a whole number of base64 groups (3 bytes) so pieces can be joined as text.
function stale(){return Object.assign(new Error('stale'),{stale:true});}
const MEDIA_CHUNK=192*1024,MIME_RE=/^[a-z]+\/[a-z0-9.+-]+$/i;
const base64Bytes=text=>{if(text.length%4||!/^[A-Za-z0-9+/]*={0,2}$/.test(text))return -1;const pad=text.endsWith('==')?2:text.endsWith('=')?1:0;return text.length/4*3-pad;};
async function readPieces(id,name,cap,deadline,onPiece,expectMime,alive=()=>true){
  const bad=()=>new Error('The media answer was incomplete.');
  let start=0,total=null,mime=null;
  while(total===null||start<total){
    const want=start,piece=await uiApi('media-read',{id,name,start:want},deadline);
    // Work that was overtaken (a Reload, a close) stops here: it is not read further and asks for no more pieces.
    if(!alive())throw stale();
    if(!isObj(piece)||![piece.total,piece.length,piece.start].every(Number.isInteger)||!isStr(piece.data)||!isStr(piece.mime))throw bad();
    if(piece.start!==want||piece.length<=0||piece.length>MEDIA_CHUNK||piece.total<1||want+piece.length>piece.total)throw bad();
    if(total!==null&&piece.total!==total)throw bad();
    if(piece.total>cap)throw Object.assign(new Error('This file is too large to load in this window.'),{tooLarge:true});
    if(!MIME_RE.test(piece.mime)||(mime!==null&&piece.mime!==mime)||(expectMime&&!piece.mime.toLowerCase().startsWith(expectMime)))throw bad();
    if(piece.data.length>Math.ceil(MEDIA_CHUNK/3)*4||base64Bytes(piece.data)!==piece.length)throw bad();
    if(want+piece.length<piece.total&&piece.length%3!==0)throw bad();
    total=piece.total;mime=piece.mime;start+=piece.length;
    onPiece(piece,start,total);
  }
}
async function fetchMedia(id,name,cap,onProgress,expectMime,alive){
  const parts=[];let mime='';
  await readPieces(id,name,cap,undefined,(piece,done,total)=>{mime=piece.mime;parts.push(decodeBase64(piece.data));if(onProgress)onProgress(done,total);},expectMime,alive);
  return new Blob(parts,{type:mime});
}
async function fetchDataURL(id,name,cap,deadline,expectMime,alive){
  let mime='',text='';
  await readPieces(id,name,cap,deadline,(piece)=>{mime=piece.mime;text+=piece.data;},expectMime,alive);
  return `data:${mime};base64,${text}`;
}
// A fresh media address (the library's, after a restart). Asked at most every 30 s. True if it changed.
async function refreshMediaSession(ep=media.epoch){
  if(Date.now()-media.lastSession<30000)return false;media.lastSession=Date.now();
  try{
    const next=await uiApi('media-session',{},POSTER_WAIT);
    if(ep!==media.epoch)return false;   // an answer from before a Reload commits nothing
    if(!isStr(next.origin)||!isStr(next.mediaKey))return false;
    const changed=next.origin!==CONNECTION.origin||next.mediaKey!==CONNECTION.mediaKey;
    CONNECTION.origin=next.origin;CONNECTION.mediaKey=next.mediaKey;if(changed)media.direct='unknown';return changed;
  }catch{return false;}
}
// Decodes an image for real (not just a request): 'loaded', 'error' or 'timeout'. Never waits for ever.
function probeImage(url,wait=POSTER_WAIT){
  return new Promise(resolve=>{
    let done=false;const finish=r=>{if(!done){done=true;clearTimeout(timer);resolve(r);}};
    const timer=setTimeout(()=>finish('timeout'),wait);
    try{const img=new Image();img.onload=()=>finish('loaded');img.onerror=()=>finish('error');img.src=url;}catch{finish('error');}
  });
}
// Pictures are kept by size, not count. A picture that is pushed out is forgotten completely, its card included, so a later
// redraw shows "Loading preview…" and the loader fetches it again instead of skipping a card marked as done.
// The one way a picture leaves the cache: the map and the byte count move together.
function forgetPoster(id){const old=media.posters.get(id);if(old===undefined)return;media.posters.delete(id);media.posterBytes-=old.length;}
function rememberPoster(id,url){
  forgetPoster(id);media.posters.set(id,url);media.posterBytes+=url.length;
  while(media.posterBytes>media.budget&&media.posters.size>1){
    const first=media.posters.keys().next().value;
    forgetPoster(first);media.cards.delete(first);
  }
}
function resetMedia(){
  media.epoch++;media.cards.clear();media.posters.clear();media.posterBytes=0;media.inflight=new Set();media.running=false;
  media.bridge='unknown';media.direct='unknown';media.reasons={bridge:'',direct:''};media.violations=[];
}

const policyBlocked=()=>media.violations.some(v=>/img-src|default-src/.test(v.directive));
function skipped(){return Object.assign(new Error('skipped'),{skipped:true});}
async function posterViaBridge(id,ep){
  if(uiControl.supported===false){media.reasons.bridge='this host does not run the recorder’s page tool';throw skipped();}
  if(media.bridge==='dead')throw skipped();
  let url;
  try{url=await fetchDataURL(id,'thumbnail.jpg',POSTER_CAP,POSTER_WAIT,'image/',()=>ep===media.epoch);}
  catch(e){
    if(ep!==media.epoch)throw stale();
    if(e.timedOut||e.unreachable){media.bridge='dead';media.reasons.bridge='no answer from the window tool';}
    else if(uiControl.supported===false){media.bridge='dead';media.reasons.bridge='this host does not run the recorder’s page tool';}
    else media.reasons.bridge=`refused (${clip(String(e.message||'error'),60)})`;
    throw e;
  }
  const shown=await probeImage(url);
  if(ep!==media.epoch)throw stale();
  if(shown!=='loaded'){media.reasons.bridge=policyBlocked()?'the window’s security policy blocks the picture (img-src)':'the picture would not display';throw Object.assign(new Error('decode'),{decode:shown});}
  media.bridge='ok';return url;
}
async function posterViaAddress(id,ep){
  if(media.direct==='failed')throw skipped();
  if(!fileURL(id,'thumbnail.jpg')){media.reasons.direct='no address for this window';throw skipped();}
  let shown=await probeImage(fileURL(id,'thumbnail.jpg'));
  if(ep!==media.epoch)throw stale();
  if(shown!=='loaded'){
    const renewed=await refreshMediaSession(ep);
    // Whatever the answer was (it may be "nothing new" precisely because a Reload overtook it), this step is over for a
    // generation that is no longer current: it must not reach the verdict writes below.
    if(ep!==media.epoch)throw stale();
    if(renewed){shown=await probeImage(fileURL(id,'thumbnail.jpg'));if(ep!==media.epoch)throw stale();}
  }
  if(shown==='loaded'){media.direct='ok';return fileURL(id,'thumbnail.jpg');}
  media.direct='failed';
  media.reasons.direct=policyBlocked()?'blocked by the window’s security policy (img-src)':shown==='timeout'?`no answer in ${POSTER_WAIT/1000} s`:'did not load';
  throw Object.assign(new Error('address'),{decode:shown});
}
const cardEl=id=>document.querySelector?.(`.card[data-id="${CSS.escape(id)}"]`)||null;
function renderMediaLine(){
  const blocked=[...media.cards.values()].some(c=>c.state==='blocked'),el=$('mediaLine');if(!el)return;
  el.hidden=!blocked;
  if(blocked){
    const parts=[media.reasons.bridge&&`window tool: ${media.reasons.bridge}`,media.reasons.direct&&`direct address: ${media.reasons.direct}`].filter(Boolean);
    setText(el,`Previews can’t load in this window${parts.length?` (${parts.join('; ')})`:''}. Open Home in your browser from the ••• menu to see them.`);
  }
}
function showPoster(id,url){
  media.cards.set(id,{state:'poster'});
  const card=cardEl(id),thumb=card?.querySelector?.('.thumbnail');if(thumb){
    let img=thumb.querySelector('img[data-poster]');
    if(!img){img=document.createElement('img');img.alt='';img.dataset.poster=id;const base=thumb.querySelector('.poster-fallback');base?base.after(img):thumb.prepend(img);}
    img.src=url;markCard(card,'poster','');
  }
  renderMediaLine();
}
function showBlocked(id){
  media.cards.set(id,{state:'blocked'});
  const card=cardEl(id);if(card){card.querySelector?.('img[data-poster]')?.remove();markCard(card,'blocked','Preview can’t load here');}
  renderMediaLine();
}
// One poster, to its end: the cached picture, then the routes in the order that has worked (the window tool first until the
// address has been seen to work), then a card that says it could not.
async function loadPoster(id){
  const ep=media.epoch;
  if(media.inflight.has(id))return;
  const cached=media.posters.get(id);if(cached){showPoster(id,cached);return;}
  media.inflight.add(id);
  try{
    const order=media.direct==='ok'?[posterViaAddress,posterViaBridge]:[posterViaBridge,posterViaAddress];
    for(const route of order){
      try{const url=await route(id,ep);if(ep!==media.epoch)return;if(url.startsWith('data:'))rememberPoster(id,url);showPoster(id,url);return;}
      catch(e){if(e?.stale||ep!==media.epoch)return;}
    }
    showBlocked(id);
  }finally{if(ep===media.epoch)media.inflight.delete(id);}
}
const wantsPoster=r=>cardMediaState(r,'loading').kind==='loading'&&media.cards.get(r.id)?.state!=='blocked'&&media.cards.get(r.id)?.state!=='poster';
async function loadPosters(){
  if(!CONNECTION.embedded||!state||media.running)return;
  const ep=media.epoch;media.running=true;
  try{
    // Each pass looks again at what still wants a picture (a card whose picture was pushed out, a redraw), at most four times.
    for(let pass=0;pass<4&&ep===media.epoch;pass++){
      const queue=state.recordings.filter(wantsPoster).map(r=>r.id);if(!queue.length)break;
      await Promise.all(Array.from({length:Math.min(POSTER_PARALLEL,queue.length)},async()=>{for(let id;ep===media.epoch&&(id=queue.shift())!==undefined;)await loadPoster(id);}));
    }
  }finally{if(ep===media.epoch)media.running=false;}
}
function markCard(card,kind,note){
  if(!card)return;card.dataset.media=kind;
  const thumb=card.querySelector?.('.thumbnail');if(!thumb)return;
  let el=thumb.querySelector('.media-note');
  if(!note){el?.remove();return;}
  if(!el){el=document.createElement('span');el.className='media-note';thumb.appendChild(el);}
  el.textContent=note;
}
// An image element's own error: in the browser the card says it has no picture; in an agent window the loader decides.
async function posterFailed(img){
  const id=img.dataset.poster,card=img.closest?.('.card');if(!id)return;
  if(CONNECTION.embedded){
    const tries=(media.cards.get(id)?.tries||0)+1;forgetPoster(id);
    if(!String(img.src||'').startsWith('data:'))media.direct='failed';
    if(tries>2){showBlocked(id);return;}
    media.cards.set(id,{state:'loading',tries});void loadPoster(id);return;
  }
  img.remove?.();markCard(card,'blocked','Preview can’t load here');
}
$('recordings').addEventListener('error',e=>{if(e.target.tagName==='IMG'&&e.target.dataset?.poster)void posterFailed(e.target);else if(e.target.tagName==='IMG')e.target.remove();},true);
$('recordings').addEventListener('load',e=>{if(e.target.tagName==='IMG'&&e.target.dataset?.poster&&!CONNECTION.embedded)markCard(e.target.closest?.('.card'),'poster','');},true);
function showPlayerNote(text){$('playerNote').hidden=!text;if(text)setText($('playerNote'),text);}
async function playerFailed(){
  const name=playerFile,id=selectedId,player=$('player');if(!name||!id)return;
  // Each source (an open, a file switch) gets at most one attempt per stage: the blob, then the data: retry after it. A stage
  // that is under way is not started again (an error and the stall timer are one fallback) and a stage that has ended, in
  // success or failure, is not repeated. An attempt that belongs to an earlier source writes nothing.
  const tok=media.playTok,mine=()=>tok===media.playTok;
  if(media.played.tok!==tok)media.played={tok,blob:'',data:''};
  const stage=player.dataset.via==='bridge'||player.dataset.via==='data'?'data':'blob',tried=media.played[stage];
  if(tried==='busy')return;
  const final='This file can’t be played here.';
  if(tried==='done'||player.dataset.via==='data'){showPlayerNote(final);return;}
  if(stage==='data'){
    // A blob URL may be refused where data: is not (blob: is not in every default policy). Modest files only.
    const file=state.recordings.find(r=>r.id===id)?.files.find(f=>f.name===name);
    if(CONNECTION.embedded&&file&&isNumber(file.bytes)&&file.bytes<=DATA_PLAYBACK_CAP){
      showPlayerNote('Loading playback…');media.played.data='busy';
      try{const url=await fetchDataURL(id,name,DATA_PLAYBACK_CAP,undefined,'video/',mine);if(!mine())return;player.dataset.via='data';player.src=url;showPlayerNote('');return;}
      catch(e){if(!mine())return;}
      finally{if(mine())media.played.data='done';}
    }
    showPlayerNote(final);return;
  }
  if(!CONNECTION.embedded||uiControl.supported===false){showPlayerNote('This file can’t be played here. Show it in the folder to open it.');return;}
  const file=state.recordings.find(r=>r.id===id)?.files.find(f=>f.name===name);
  if(file&&isNumber(file.bytes)&&file.bytes>MEDIA_BRIDGE_CAP){showPlayerNote('This recording is too large to play in this window. Open it in your browser from the ••• menu.');return;}
  showPlayerNote('Loading playback…');media.played.blob='busy';
  try{
    const blob=await fetchMedia(id,name,MEDIA_BRIDGE_CAP,(done,total)=>{if(mine())showPlayerNote(`Loading playback… ${Math.round(done/total*100)}%`);},'video/',mine);
    if(!mine())return;
    if(media.playerUrl&&typeof URL.revokeObjectURL==='function')URL.revokeObjectURL(media.playerUrl);
    media.playerUrl=URL.createObjectURL(blob);player.dataset.via='bridge';player.src=media.playerUrl;showPlayerNote('');
  }catch(e){if(mine())showPlayerNote(e.tooLarge?'This recording is too large to play in this window. Open it in your browser from the ••• menu.':'Playback can’t load in this window. Open it in your browser from the ••• menu.');}
  finally{if(mine())media.played.blob='done';}
}
$('player').addEventListener('error',()=>void playerFailed());
// 60 fps (Settings › Recording): what is known, said plainly. Unknown is never "unavailable", choosing 60 is never blocked,
// and the encoder-only check is shown beside the answer for this setup, never as that answer.
function fpsCapabilityMarkup(){
  const f=state.capture?.capabilities?.fps60;if(!f||!captureKnown())return '';
  const status=['unknown','qualified','failed-last'].includes(f.status)?f.status:'unknown';
  const words={unknown:'60 fps hasn’t been measured on this computer for this setup (screen, camera, microphone and originals as they are now). You can still choose it; a recording stops with the reason if it can’t keep up.',qualified:'A recording of this setup on this computer kept up at 60 fps. Each recording still measures its delivered rate.',['failed-last']:'Your last 60 fps recording of this setup couldn’t keep up. You can still choose it, or use 30 fps.'}[status];
  const measured=isNumber(f.measuredFps)?` Measured ${f.measuredFps.toFixed(1)} fps.`:'';
  const e=isObj(f.encoder)?f.encoder:null;
  const encoder=e?({qualified:'The encoder check passed on test frames. That is encoder capacity only; it doesn’t show this setup will keep up.',['failed-last']:'The encoder check couldn’t keep up with test frames.',unavailable:'The video encoder didn’t accept 60 fps in the last check.'}[e.status]||''):'';
  return `<div class="fps-check" id="fpsCheck"><small>${escapeHTML(words)}${escapeHTML(measured)}</small>${encoder?`<small id="fpsEncoder">${escapeHTML(encoder)}</small>`:''}<small>The check encodes test frames only — no camera, mic or screen — so it shows encoder capacity, not your devices’ real frame rate and not this setup.</small><button id="checkFps" ${capturePhase()==='idle'?'':'disabled'}>${e?'Check encoder again':'Check encoder at 60 fps'}</button></div>`;
}
// One light ticker updates the recording timer and share countdown; it never calls the app.
const tickTimer=setInterval(()=>{
  if(!state||document.hidden)return;
  if(state.active?.status==='recording')setText($('activeLabel'),activeLabel(state.active));
  if(captureShown()){const elapsed=captureElapsed();$('capTime').hidden=elapsed===null;if(elapsed!==null)setText($('capTime'),captureClock(elapsed));}
  if($('shareDialog').open)renderShare();
},1000);
if(typeof __OPENRECORDER_TEST__!=='undefined')globalThis.__openrecorder={cardMarkup,shareState,shareLink,shareCode,formatCode,activeLabel,fileURL,filtered,emptyCopy,detailRows,choices,settingDefs,paneMarkup,when,duration,get state(){return state;},set state(v){state=v;},get view(){return view;},set view(v){view=v;},reach,retryDelays,openRecording,openShare,openSettings,openMenu,closeMenu,render,refresh,renderCapture,captureElapsed,fpsCapabilityMarkup,captureShown,cardMediaState,fetchMedia,fetchDataURL,refreshMediaSession,posterFailed,playerFailed,loadPoster,loadPosters,probeImage,resetMedia,rememberPoster,setPlayerFile,closeRecording,media};
void init();
