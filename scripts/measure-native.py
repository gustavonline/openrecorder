import subprocess,time,json,sys,os,statistics,math,hashlib,platform,plistlib,shutil,signal,threading,datetime,re

USAGE='usage: measure-native.py [--mode idle|generated] [--scope TEXT] [--binary [LABEL=]PATH]... [--version-label TEXT] [--meta KEY=VALUE]... [--include-browser] [--timeout SECONDS [--test-mode --kill-on-timeout]] [--post-exit-grace SECONDS] [--expect-survivor ARGS_SUBSTRING]... [--expect-output PATH]... OUTPUT.json COMMAND [ARGS...]'
SCHEMA_VERSION=2
WARMUP_SECONDS=5
SAMPLE_INTERVAL=.25
STDOUT_KEEP=6000
STDERR_KEEP=3000
MAX_INFERRED_BINARIES=16
# Collection validity (separate from any performance judgement): the steady window must be long enough and densely
# sampled, with no long gap at its start, inside it or before the run ended.
MIN_STEADY_SECONDS=5.0
MIN_COVERAGE=.8
MAX_GAP_SECONDS=2.0
DEFAULT_POST_EXIT_GRACE=15.0
PS_COLUMNS='pid=,ppid=,rss=,%cpu=,time=,lstart=,comm='
LINUX=sys.platform.startswith('linux')
BROWSER_PREFIXES=('google chrome','chrome','chromium','microsoft edge','msedge','firefox','safari','brave','opera','vivaldi')
LEGACY_SCOPE='Rust library host + native menu-bar/tray app (no toolbar) + generated camera viewport + media helper process family; browser excluded; synthetic screen/camera and test tone, no physical device capture'
MODE_SCOPES={'idle':'Idle app process family: no capture session started, no generated sources, no encoders running','generated':'Generated capture: synthetic screen and camera sources, generated test tone, 3 encoders; Rust library host + native menu-bar/tray app (no toolbar) + media helper process family; no physical device capture'}
VALUE_FLAGS={'--mode','--scope','--binary','--version-label','--meta','--timeout','--post-exit-grace','--expect-survivor','--expect-output'}
BOOL_FLAGS={'--include-browser','--test-mode','--kill-on-timeout'}

def iso(t):return datetime.datetime.fromtimestamp(t,datetime.timezone.utc).isoformat(timespec='milliseconds').replace('+00:00','Z')

def loadavg():
 try:return [round(x,2)for x in os.getloadavg()]
 except OSError:return None

def parse_args(argv):
 opts={'mode':None,'scope':None,'binary':[],'versionLabel':None,'meta':{},'includeBrowser':False,'testMode':False,'killOnTimeout':False,'timeout':None,'postExitGrace':DEFAULT_POST_EXIT_GRACE,'expectSurvivor':[],'expectOutput':[]}
 i=0
 while i<len(argv)and argv[i].startswith('--'):
  flag=argv[i];i+=1;value=None
  if flag=='--':break
  if '='in flag:flag,value=flag.split('=',1)
  if flag in BOOL_FLAGS:
   if value is not None:raise ValueError(flag+' takes no value')
   opts[{'--include-browser':'includeBrowser','--test-mode':'testMode','--kill-on-timeout':'killOnTimeout'}[flag]]=True
  elif flag in VALUE_FLAGS:
   if value is None:
    if i>=len(argv):raise ValueError(flag+' needs a value')
    value=argv[i];i+=1
   if flag=='--mode':
    if value not in MODE_SCOPES:raise ValueError('--mode must be idle or generated')
    opts['mode']=value
   elif flag=='--scope':opts['scope']=value[:500]
   elif flag=='--version-label':opts['versionLabel']=value[:200]
   elif flag=='--binary':
    label,path=(value.split('=',1)if re.match(r'^[A-Za-z0-9_.-]+=',value)else(None,value))
    opts['binary'].append((label or os.path.basename(path),path))
   elif flag=='--meta':
    key,sep,text=value.partition('=')
    if not sep or not re.match(r'^[A-Za-z0-9_.-]{1,64}$',key):raise ValueError('--meta needs KEY=VALUE with KEY of [A-Za-z0-9_.-]')
    opts['meta'][key]=text[:200]
   elif flag=='--expect-survivor':
    if not value:raise ValueError('--expect-survivor needs a non-empty argument substring')
    opts['expectSurvivor'].append(value[:200])
   elif flag=='--expect-output':opts['expectOutput'].append(value)
   elif flag=='--post-exit-grace':
    try:opts['postExitGrace']=float(value)
    except ValueError:raise ValueError('--post-exit-grace needs a number of seconds')
    if not 0<=opts['postExitGrace']<=120:raise ValueError('--post-exit-grace must be between 0 and 120 seconds')
   else:
    try:opts['timeout']=float(value)
    except ValueError:raise ValueError('--timeout needs a number of seconds')
    if not opts['timeout']>0:raise ValueError('--timeout must be positive')
  else:raise ValueError('unknown option '+flag)
 if opts['killOnTimeout']and not(opts['testMode']and opts['timeout']):raise ValueError('--kill-on-timeout needs --test-mode and --timeout')
 if len(argv)-i<2:raise ValueError('need OUTPUT and COMMAND')
 return opts,argv[i],argv[i+1:]

def resolve_scope(opts):
 if opts['scope']:return opts['scope'],'flag'
 if os.environ.get('OPENRECORDER_MEASURE_SCOPE'):return os.environ['OPENRECORDER_MEASURE_SCOPE'],'env'
 if opts['mode']:return MODE_SCOPES[opts['mode']],'mode'
 scope=LEGACY_SCOPE
 if opts['includeBrowser']:scope=scope.replace('browser excluded','browser included (--include-browser)')
 return scope,'default'

def parse_ps_line(line):
 v=line.split(None,10)
 if len(v)<10:return None
 try:return {'pid':int(v[0]),'ppid':int(v[1]),'rssKiB':int(v[2]),'cpu':float(v[3]),'cpuTime':v[4],'start':' '.join(v[5:10]),'comm':v[10].strip()if len(v)>10 else ''}
 except ValueError:return None

def parse_cputime(text):
 days=0
 try:
  if '-'in text:
   d,text=text.split('-',1);days=int(d)
  parts=[float(x)for x in text.split(':')]
 except ValueError:return None
 seconds=0.0
 for x in parts:seconds=seconds*60+x
 return round(days*86400+seconds,2)

def parse_start(text):
 try:return time.mktime(time.strptime(text,'%a %b %d %H:%M:%S %Y'))
 except (ValueError,OverflowError):return None

def is_browser(comm):return os.path.basename(comm).lower().startswith(BROWSER_PREFIXES)

def snapshot():
 out=subprocess.run(['ps','-axo',PS_COLUMNS],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,errors='replace',timeout=10,env=dict(os.environ,LC_ALL='C'))
 rows=[r for r in map(parse_ps_line,out.stdout.splitlines())if r]
 if not rows:raise OSError('ps returned no parsable rows (exit %s)'%out.returncode)
 return rows

class Family:
 # Root pid plus descendants. A pid stays in the family only while it keeps the same ps start time, so a reused pid is never counted; reparented children stay because they were seen as members first.
 def __init__(self,root_pid,launch_wall,include_browser):
  self.root_pid=root_pid;self.floor=launch_wall-2;self.include_browser=include_browser
  self.known={};self.seen={};self.reused=[];self.excluded={};self.root_seen=False
 def fresh(self,r):
  s=parse_start(r['start'])
  return s is None or s>=self.floor
 def add(self,r):
  old=self.seen.get(r['pid'])
  if old is not None and old!=r['start']and len(self.reused)<20:self.reused.append(r['pid'])
  self.seen[r['pid']]=r['start'];self.known[r['pid']]=r['start']
 def update(self,rows,allow_new=True):
  by_pid={r['pid']:r for r in rows};members={}
  for pid,start in list(self.known.items()):
   r=by_pid.get(pid)
   if r and r['start']==start:members[pid]=r
   else:del self.known[pid]
  if allow_new:
   if not self.root_seen:
    r=by_pid.get(self.root_pid)
    if r and self.fresh(r):
     self.add(r);members[self.root_pid]=r;self.root_seen=True
   children={}
   for r in rows:children.setdefault(r['ppid'],[]).append(r)
   queue=list(members)
   while queue:
    parent=queue.pop()
    for r in children.get(parent,[]):
     pid=r['pid']
     if pid in members or pid==parent:continue
     if not self.include_browser and is_browser(r['comm']):
      if len(self.excluded)<50:self.excluded[(pid,r['start'])]=os.path.basename(r['comm'])
      continue
     if not self.fresh(r):continue
     self.add(r);members[pid]=r;queue.append(pid)
  return members

class Tail:
 def __init__(self,keep=16384):
  self.keep=keep;self.buf=bytearray();self.total=0
 def add(self,data):
  self.total+=len(data);self.buf+=data
  if len(self.buf)>self.keep*2:del self.buf[:-self.keep]
 def text(self,chars):return bytes(self.buf[-self.keep:]).decode('utf-8','replace')[-chars:]

def pump(stream,tail):
 try:
  while True:
   data=os.read(stream.fileno(),65536)
   if not data:break
   tail.add(data)
 except (OSError,ValueError):pass

def read_pss(pid):
 try:
  with open('/proc/%d/smaps_rollup'%pid)as f:
   for l in f:
    if l.startswith('Pss:'):return int(l.split()[1])/1024
 except (OSError,ValueError):pass
 return None

def exe_path(pid,comm):
 if LINUX:
  try:return os.readlink('/proc/%d/exe'%pid)
  except OSError:return None
 return comm if comm.startswith('/')else None

def build_sample(members,elapsed,now,phase='run'):
 details=[];rss=0;cpu=0.0;pss=0.0;pss_seen=0
 for pid in sorted(members):
  r=members[pid];value=read_pss(pid)if LINUX else None
  rss+=r['rssKiB'];cpu+=r['cpu']
  if value is not None:pss+=value;pss_seen+=1
  details.append({'pid':pid,'ppid':r['ppid'],'name':os.path.basename(r['comm']),'rssMiB':round(r['rssKiB']/1024,2),'pssMiB':round(value,2)if value is not None else None,'psLifetimeCpuPercent':r['cpu'],'cpuTimeSeconds':parse_cputime(r['cpuTime'])})
 return {'phase':phase,'time':round(elapsed,2),'elapsedMonotonicSeconds':round(elapsed,3),'utc':iso(now),'loadavg':loadavg(),'rssMiB':round(rss/1024,2),'pssMiB':round(pss,2)if pss_seen else None,'pssMissingProcesses':len(members)-pss_seen if LINUX else None,'cpuPercent':round(cpu,2),'processes':len(members),'details':details}

def p95(values):
 s=sorted(values);return s[max(0,math.ceil(len(s)*.95)-1)]

def med(values):return round(statistics.median(values),2)if values else None

def top(values):return round(p95(values),2)if values else None

def summarize_processes(samples,starts):
 acc={}
 for s in samples:
  t=s['elapsedMonotonicSeconds'];steady=t>=WARMUP_SECONDS
  for d in s['details']:
   a=acc.setdefault(d['pid'],{'name':d['name'],'first':t,'last':t,'count':0,'rss':[],'pss':[],'steadyRss':[],'steadyPss':[]})
   a['last']=t;a['count']+=1;a['rss'].append(d['rssMiB'])
   if d['pssMiB']is not None:a['pss'].append(d['pssMiB'])
   if steady:
    a['steadyRss'].append(d['rssMiB'])
    if d['pssMiB']is not None:a['steadyPss'].append(d['pssMiB'])
 return [{'pid':pid,'name':a['name'],'start':starts.get(pid),'firstSeenSeconds':a['first'],'lastSeenSeconds':a['last'],'samples':a['count'],'steadySamples':len(a['steadyRss']),'peakRssMiB':max(a['rss']),'peakPssMiB':max(a['pss'])if a['pss']else None,'steadyRssMedianMiB':med(a['steadyRss']),'steadyRssP95MiB':top(a['steadyRss']),'steadyPssMedianMiB':med(a['steadyPss']),'steadyPssP95MiB':top(a['steadyPss'])}for pid,a in sorted(acc.items())]

def sha256_file(path):
 h=hashlib.sha256()
 with open(path,'rb')as f:
  for chunk in iter(lambda:f.read(1<<20),b''):h.update(chunk)
 return h.hexdigest()

def bundle_version(path):
 marker='.app/Contents/';i=path.rfind(marker)
 if i<0:return None
 try:
  with open(path[:i+len(marker)]+'Info.plist','rb')as f:info=plistlib.load(f)
  return {k:info[k]for k in('CFBundleIdentifier','CFBundleShortVersionString','CFBundleVersion')if isinstance(info.get(k),str)}or None
 except Exception:return None

def file_info(path,label,source):
 info={'label':label,'source':source,'name':os.path.basename(path),'path':path}
 try:
  st=os.stat(path);info['bytes']=st.st_size;info['mtimeUtc']=iso(st.st_mtime);info['sha256']=sha256_file(path)
 except OSError as e:info['error']='%s: %s'%(e.__class__.__name__,e.strerror)
 bundle=bundle_version(path)
 if bundle:info['bundle']=bundle
 return info

def resolve_executable(name):
 path=os.path.abspath(name)if '/'in name else shutil.which(name)
 return os.path.realpath(path)if path else None

def system_info():
 info={'platform':sys.platform,'system':platform.system(),'release':platform.release(),'machine':platform.machine(),'platformString':platform.platform(),'logicalCpus':os.cpu_count(),'python':platform.python_version()}
 if sys.platform=='darwin':info['macOS']=platform.mac_ver()[0]
 return info

def assess_collection(samples,run_seconds,error_count):
 # Judges only whether the collector observed the steady window well enough to say anything about it.
 window=max(0.0,(run_seconds or 0)-WARMUP_SECONDS)
 steady=[x['elapsedMonotonicSeconds']for x in samples if x.get('phase','run')=='run'and x['elapsedMonotonicSeconds']>=WARMUP_SECONDS]
 info={'steadyWindowSeconds':round(window,2),'steadySamples':len(steady),'samplingErrorCount':error_count,'minSteadySeconds':MIN_STEADY_SECONDS,'minCoverage':MIN_COVERAGE,'maxGapSeconds':MAX_GAP_SECONDS,'coverage':None,'longestGapSeconds':None,'headGapSeconds':None,'tailGapSeconds':None,'valid':False,'problems':[]}
 if window<MIN_STEADY_SECONDS:info['problems'].append('steady window %.2fs is shorter than the required %.1fs'%(window,MIN_STEADY_SECONDS))
 if not steady:
  info['problems'].append('no steady samples')
  return info
 edges=[WARMUP_SECONDS]+steady+[run_seconds];gaps=[b-a for a,b in zip(edges,edges[1:])]
 info['coverage']=round((steady[-1]-steady[0])/window,3)if window else None
 info['longestGapSeconds']=round(max(gaps),2);info['headGapSeconds']=round(gaps[0],2);info['tailGapSeconds']=round(gaps[-1],2)
 if info['coverage']is not None and info['coverage']<MIN_COVERAGE:info['problems'].append('samples cover %.0f%% of the steady window (need %.0f%%)'%(info['coverage']*100,MIN_COVERAGE*100))
 if max(gaps)>MAX_GAP_SECONDS:info['problems'].append('longest sampling gap %.2fs exceeds %.1fs (head %.2fs, tail %.2fs, %d sampling errors)'%(max(gaps),MAX_GAP_SECONDS,gaps[0],gaps[-1],error_count))
 info['valid']=not info['problems']
 return info

def args_for(pids):
 # Command lines of specific pids, only to classify survivors against --expect-survivor.
 if not pids:return {}
 try:out=subprocess.run(['ps','-o','pid=,args=','-p',','.join(str(p)for p in sorted(pids))],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,errors='replace',timeout=10,env=dict(os.environ,LC_ALL='C')).stdout
 except (OSError,subprocess.SubprocessError):return {}
 found={}
 for line in out.splitlines():
  head,_,rest=line.strip().partition(' ')
  if head.isdigit():found[int(head)]=rest.strip()
 return found

def classify_survivors(members,root_pid,expected):
 # Everything still alive after the root exited. A declared service (for example the background library host)
 # is expected; anything else, notably a media encoder, means work had not finished.
 live={pid:r for pid,r in members.items()if pid!=root_pid}
 args=args_for(set(live))
 out=[]
 for pid,r in sorted(live.items()):
  text=args.get(pid)
  out.append({'pid':pid,'ppid':r['ppid'],'name':os.path.basename(r['comm']),'expectedService':text is not None and any(e in text for e in expected),'argsKnown':text is not None})
 return out

def check_outputs(paths):
 # Finalization evidence for declared outputs: present, non-empty and not still growing.
 out=[]
 for path in paths:
  item={'path':path,'exists':False,'bytes':None,'stable':None}
  try:
   first=os.stat(path).st_size;time.sleep(.5);second=os.stat(path).st_size
   item.update(exists=True,bytes=second,stable=first==second)
  except OSError as e:item['error']='%s: %s'%(e.__class__.__name__,e.strerror)
  out.append(item)
 return out

def failure_reasons(r):
 out=[]
 if r.get('launchError'):out.append('launch failed: '+r['launchError'])
 if r.get('interrupted'):out.append('measurement interrupted before child exit; child was not killed')
 code=r.get('exit')
 if code is not None and code!=0:out.append('child exit %d'%code if code>0 else 'child killed by signal %d'%-code)
 if r.get('timeoutExceeded'):out.append('exceeded --timeout of %ss'%r['timeoutSeconds'])
 if r.get('launchError'):return out
 if not r['sampleCount']:out.append('no process-family samples collected')
 elif not r['steadySampleCount']:out.append('no steady samples: last sample at %.2fs, steady window starts at %ss'%(r['lastSampleSeconds'],WARMUP_SECONDS))
 collection=r.get('collection')
 if collection and r['steadySampleCount']and not collection['valid']:out.append('incomplete collection: '+'; '.join(collection['problems']))
 # Without one successful process snapshot after the root exited there is no evidence that nothing (an encoder) was still
 # running, so an empty survivor list means 'unknown', never 'clean'.
 if code is not None and r.get('survivorAccounting')!='complete':out.append('post-exit process accounting was %s: no successful process snapshot after the root exited, so it is unknown whether an encoder had finished'%r.get('survivorAccounting','missing'))
 unexpected=[x for x in r.get('postExitSurvivors',[])if not x['expectedService']]
 if unexpected:out.append('processes still running after the %ss post-exit window (not declared with --expect-survivor): %s'%(r.get('postExitGraceSeconds'),', '.join('%s[%d]'%(x['name'],x['pid'])for x in unexpected)))
 for o in r.get('expectedOutputs',[]):
  if not o['exists']:out.append('declared output missing: '+o['path'])
  elif not o['bytes']:out.append('declared output is empty: '+o['path'])
  elif not o['stable']:out.append('declared output was still growing: '+o['path'])
 return out

def exit_code(report):
 if report.get('launchError'):return report.get('launchExit',127)
 if report['interrupted']:return 130
 code=report['exit']
 if code<0:return 128-code
 if code:return code
 return 0 if report['status']=='PASS'else 3

def write_report(path,report):
 tmp=path+'.tmp'
 with open(tmp,'w')as f:f.write(json.dumps(report,indent=2))
 os.replace(tmp,path)

def on_term(signum,frame):raise KeyboardInterrupt

def measure(opts,command):
 scope,scope_source=resolve_scope(opts);binaries=[];covered=set()
 for label,path in opts['binary']:
  binaries.append(file_info(path,label,'explicit'));covered.add(os.path.realpath(path))
 exe=resolve_executable(command[0])
 if exe and exe not in covered:binaries.append(file_info(exe,'command','inferred-argv0'));covered.add(exe)
 try:script_sha=sha256_file(__file__)
 except OSError:script_sha=None
 report={'schemaVersion':SCHEMA_VERSION,'status':'FAIL','steadyWindowAvailable':False,'failureReasons':[],'platform':sys.platform,'mode':opts['mode'],'scope':scope,'scopeSource':scope_source,'browserPolicy':'included (--include-browser)'if opts['includeBrowser']else'excluded','cpuMeasurement':'ps lifetime CPU percentage, not instantaneous CPU','cpuMeasurementNote':'ps %cpu semantics are OS-defined and may be a decaying average; per-process values are psLifetimeCpuPercent, cpuTimeSeconds is cumulative; neither is an interval measurement','pssAvailable':LINUX,'warmupExcludedSeconds':WARMUP_SECONDS,'sampleIntervalSeconds':SAMPLE_INTERVAL,'system':system_info(),'loadavgStart':loadavg(),'measurementScript':{'name':os.path.basename(__file__),'sha256':script_sha},'command':{'executable':os.path.basename(command[0]),'argCount':len(command)-1},'metadata':{'versionLabel':opts['versionLabel'],'custom':opts['meta']},'timeoutSeconds':opts['timeout'],'timeoutExceeded':False,'testMode':opts['testMode'],'interrupted':False,'sampleCount':0,'steadySampleCount':0,'lastSampleSeconds':None,'exit':None}
 samples=[];errors=[];error_count=0;exe_paths={};out_tail=Tail();err_tail=Tail();family=None;timed_out=False;killed=False;survivors=[];launch_wall=time.time()
 try:p=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 except OSError as e:
  report['launchError']='%s: %s'%(e.__class__.__name__,e.strerror);report['launchExit']=127 if isinstance(e,FileNotFoundError)else 126;report['utcStart']=report['utcEnd']=iso(launch_wall)
  report.update({'binaries':binaries,'samples':[],'processSummary':[],'samplingErrors':[],'postExitSurvivors':[],'expectedOutputs':[],'result':'','stderr':'','runSeconds':0.0,'rootPid':None,'endCondition':'launch-failed'})
  report['failureReasons']=failure_reasons(report);report['failure']={'reasons':report['failureReasons']}
  return report,report['launchExit']
 start=time.monotonic();report['utcStart']=iso(launch_wall);report['rootPid']=p.pid
 threads=[threading.Thread(target=pump,args=(p.stdout,out_tail),daemon=True),threading.Thread(target=pump,args=(p.stderr,err_tail),daemon=True)]
 for t in threads:t.start()
 family=Family(p.pid,launch_wall,opts['includeBrowser'])
 try:
  while p.poll() is None:
   try:rows=snapshot()
   except (OSError,subprocess.SubprocessError)as e:
    rows=None;error_count+=1
    if len(errors)<20:errors.append({'elapsedMonotonicSeconds':round(time.monotonic()-start,3),'error':e.__class__.__name__+': '+str(e)[:200]})
   if rows is not None:
    elapsed=time.monotonic()-start;members=family.update(rows)
    if members:
     samples.append(build_sample(members,elapsed,time.time()))
     for pid,r in members.items():
      if pid not in exe_paths:exe_paths[pid]=exe_path(pid,r['comm'])
   if opts['timeout']and not timed_out and time.monotonic()-start>=opts['timeout']:
    timed_out=True
    if opts['killOnTimeout']:
     killed=True
     try:p.terminate()
     except OSError:pass
   time.sleep(SAMPLE_INTERVAL)
 except KeyboardInterrupt:report['interrupted']=True
 run_seconds=time.monotonic()-start;report['utcEnd']=iso(time.time());report['loadavgEnd']=loadavg();report['runSeconds']=round(run_seconds,3)
 drained=None;end_condition='interrupted'if report['interrupted']else'root-exit';accounting='not-attempted';live_after_exit=False;outputs=[]
 if not report['interrupted']:
  report['exit']=p.returncode
  # The root has exited. Observe what is left for a bounded window instead of ending at root exit or waiting forever:
  # descendants are still followed from the members already seen, identity-checked by pid and ps start time.
  grace_end=time.monotonic()+opts['postExitGrace'];first=True;accounting='unavailable'
  while True:
   try:rows=snapshot()
   except (OSError,subprocess.SubprocessError)as e:
    rows=None;error_count+=1
    if len(errors)<20:errors.append({'elapsedMonotonicSeconds':round(time.monotonic()-start,3),'phase':'post-exit','error':e.__class__.__name__+': '+str(e)[:200]})
   if rows is not None:
    accounting='complete';members=family.update(rows)
    survivors=classify_survivors(members,p.pid,opts['expectSurvivor'])
    if first:live_after_exit=bool(survivors);first=False
    if survivors:samples.append(build_sample({pid:r for pid,r in members.items()if pid!=p.pid},time.monotonic()-start,time.time(),'post-exit'))
    if not[x for x in survivors if not x['expectedService']]:break
   if time.monotonic()>=grace_end:break
   time.sleep(SAMPLE_INTERVAL)
  if accounting=='complete':end_condition='root-exit'if not live_after_exit else('root-exit+drained'if not survivors else('root-exit+expected-services-remain'if all(x['expectedService']for x in survivors)else'root-exit+grace-expired'))
  else:end_condition='root-exit+accounting-unavailable'
  outputs=check_outputs(opts['expectOutput'])
  # Output pipes may be held open by surviving descendants, so they are drained only after the observation window.
  for t in threads:t.join(2)
  drained=not any(t.is_alive()for t in threads)
 run_samples=[x for x in samples if x.get('phase','run')=='run']
 steady=[x for x in run_samples if x['elapsedMonotonicSeconds']>=WARMUP_SECONDS]
 inferred=[]
 for path in sorted({x for x in exe_paths.values()if x}):
  real=os.path.realpath(path)
  if real in covered:continue
  covered.add(real)
  if len(inferred)<MAX_INFERRED_BINARIES:inferred.append(file_info(path,os.path.basename(path),'inferred-process-family'))
 pss_values=[x['pssMiB']for x in samples if x['pssMiB']is not None]
 report.update({'peakRssMiB':max((x['rssMiB']for x in samples),default=None),'peakPssMiB':max(pss_values)if pss_values else None,'sampleCount':len(samples),'steadySampleCount':len(steady),'steadyWindowAvailable':bool(steady),'lastSampleSeconds':run_samples[-1]['elapsedMonotonicSeconds']if run_samples else None,'postExitSampleCount':len(samples)-len(run_samples),'timeoutExceeded':timed_out,'timeoutAction':('SIGTERM sent to root pid only (test mode)'if killed else'none; sampling continued and nothing was killed'),'binaries':binaries+inferred,'samples':samples,'processSummary':summarize_processes(run_samples,family.seen),'excludedBrowserProcesses':[{'pid':k[0],'name':v}for k,v in family.excluded.items()],'pidReuseWithinFamily':family.reused,'postExitSurvivors':survivors,'survivorAccounting':accounting,'rootExitedWithLiveDescendants':live_after_exit,'endCondition':end_condition,'postExitGraceSeconds':opts['postExitGrace'],'expectedServiceSurvivorPatterns':opts['expectSurvivor'],'expectedOutputs':outputs,'collection':assess_collection(samples,run_seconds if not report['interrupted']else(run_samples[-1]['elapsedMonotonicSeconds']if run_samples else 0),error_count),'samplingErrorCount':error_count,'samplingErrors':errors,'lineage':'sampled observed descendants of the root (pid + ps start time identity), polled every %ss; not an exhaustive process lineage, short-lived intermediates can be missed'%SAMPLE_INTERVAL,'statusMeaning':'PASS means the command exited 0, sampling covered the steady window and nothing undeclared outlived it; it does not judge frame pacing, realtime factor, output validity or resource thresholds','peakIncludesPostExit':True,'outputCapture':{'stdoutBytes':out_tail.total,'stderrBytes':err_tail.total,'drainedAtExit':drained},'result':out_tail.text(STDOUT_KEEP),'stderr':err_tail.text(STDERR_KEEP)})
 if report['interrupted']:report['childStillRunningPid']=p.pid
 if steady:
  rss=[x['rssMiB']for x in steady];pss=[x['pssMiB']for x in steady if x['pssMiB']is not None];cpu=[x['cpuPercent']for x in steady]
  report.update({'steadyRssMedianMiB':med(rss),'steadyRssP95MiB':top(rss),'steadyPsLifetimeCpuMedianPercent':med(cpu),'steadyPsLifetimeCpuP95Percent':top(cpu)})
  if pss:report.update({'steadyPssMedianMiB':med(pss),'steadyPssP95MiB':top(pss)})
 reasons=failure_reasons(report);report['failureReasons']=reasons;report['status']='FAIL'if reasons else'PASS'
 if reasons:report['failure']={'reasons':reasons,'childExit':report['exit'],'childSignal':-report['exit']if report['exit']is not None and report['exit']<0 else None,'runSeconds':report['runSeconds'],'sampleCount':len(samples),'steadySampleCount':len(steady),'lastSampleSeconds':report['lastSampleSeconds'],'warmupExcludedSeconds':WARMUP_SECONDS,'lastSamples':samples[-3:],'stdoutTail':out_tail.text(2000),'stderrTail':err_tail.text(2000)}
 return report,exit_code(report)

def main(argv):
 try:opts,output,command=parse_args(argv)
 except ValueError as e:
  print('measure-native.py: '+str(e),file=sys.stderr);print(USAGE,file=sys.stderr);return 2
 try:write_report(output,{'schemaVersion':SCHEMA_VERSION,'status':'RUNNING'})
 except OSError as e:
  print('measure-native.py: cannot write '+output+': '+str(e),file=sys.stderr);return 2
 signal.signal(signal.SIGTERM,on_term)
 report,code=measure(opts,command)
 write_report(output,report)
 print(json.dumps({k:v for k,v in report.items()if k not in['samples','result','stderr','processSummary','binaries','failure','system']}));print(str(report.get('stderr',''))[-1000:])
 return code

if __name__=='__main__':sys.exit(main(sys.argv[1:]))
