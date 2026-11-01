
import json, os, subprocess, sys, time, uuid, signal, hashlib, tarfile, io, platform
BIN=os.environ["BIN"]; AD=os.environ["AD"]; CORES=os.environ.get("CORES","124-125"); SLOW=float(os.environ.get("SLOW","5"))
VERSION="9.9.9"
log=open(f"{AD}/gflow-calls.log","w")
def publish(releases):
    build=os.path.join(releases,"build"); os.makedirs(build,exist_ok=True)
    report=json.dumps({"binary":"ovis-terminal","version":VERSION,"generation":"1"},separators=(",",":"))
    script=os.path.join(build,"ovis-terminal"); open(script,"w").write(f"#!/bin/sh\nprintf '%s\\n' '{report}'\n"); os.chmod(script,0o755)
    target=f"linux-{platform.machine()}"
    json.dump({"version":VERSION,"target":target,"protocol_generations":["1"],"holder_generations":["2"]},open(os.path.join(build,"build.json"),"w"))
    d=os.path.join(releases,VERSION); os.makedirs(d,exist_ok=True)
    archive=os.path.join(d,f"ovis-terminal-{VERSION}-{target}.tar")
    subprocess.run(["tar","--format=ustar","-cf",archive,"-C",build,"build.json","ovis-terminal"],check=True)
    return hashlib.sha256(open(archive,"rb").read()).hexdigest()
def one(k):
    root=f"{AD}/gflow-{k}-{uuid.uuid4().hex[:6]}"; os.makedirs(root)
    gdb=subprocess.Popen(["taskset","-c",CORES,"gdb","-q","-batch","-ex","set pagination off","-ex","handle SIGINT stop print nopass","-ex","handle SIGPIPE nostop noprint pass","-ex","run","-ex","thread apply all bt 40","-ex","kill","--args",BIN,"serve","--root",root,"--stdin-lifetime"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
    def nextjson(key):
        while True:
            line=gdb.stdout.readline()
            if not line: raise RuntimeError("server stdout ended")
            if line.startswith('{"'+key): return json.loads(line)
    ready=nextjson("ready")["ready"]
    inferior=int(open(f"/proc/{gdb.pid}/task/{gdb.pid}/children").read().split()[0]); assert inferior>1
    env={"PATH":"/usr/bin:/bin","OVIS_TERMINAL_SOCKET":ready["socket"],"OVIS_TERMINAL_CREDENTIAL":ready["bearer"]}
    def call(m,p,timed=None):
        t=time.monotonic()
        c=subprocess.Popen(["taskset","-c",CORES,BIN,"--json","call",m,"--params",json.dumps(p)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try:
            out,err=c.communicate(timeout=SLOW if timed else 60)
        except subprocess.TimeoutExpired:
            os.kill(inferior, signal.SIGINT); c.wait(); log.write(f"{k} {timed} SLOW>{SLOW} caught\n"); log.flush(); return None
        if timed: log.write(f"{k} {timed} {time.monotonic()-t:.3f} {c.returncode}\n"); log.flush()
        if c.returncode: raise RuntimeError(f"{m}: {err}")
        return json.loads(out)
    caught=False
    try:
        p={"op_id":str(uuid.uuid7()),"program":{"argv":["/bin/sh"]},"geometry":{"cols":80,"rows":24},"cwd":"/tmp","env":{"PATH":"/usr/bin:/bin","PS1":"$ ","TERM":"xterm"},"name":"shell","labels":None,"parent":None,"child_policy":None,"keep":None,"first_message":None,"machine":None}
        call("terminal.run",p)
        if call("update.query",{},"first") is None: caught=True; return True
        sha=publish(os.path.join(root,"state","updates","releases"))
        staged=call("update.stage",{"op_id":str(uuid.uuid7()),"version":VERSION,"target":None,"sha256":sha})
        call("update.verify",{"op_id":str(uuid.uuid7()),"update_id":staged["update_id"]})
        call("update.activate",{"op_id":str(uuid.uuid7()),"update_id":staged["update_id"]})
        nextjson("restarted")
        if call("update.query",{},"second") is None: caught=True; return True
        return False
    finally:
        if not caught: gdb.stdin.close()
        out=gdb.communicate(timeout=900)[0]
        if caught: open(f"{AD}/gflow-bt-{k}.log","w").write(out)
spin=[subprocess.Popen(["taskset","-c",CORES,"bash","-c","while :; do :; done"]) for _ in range(int(os.environ.get("SPIN","8")))]
try:
    for k in range(int(os.environ.get("SERVERS","25"))):
        try:
            if one(k): print("caught", k); break
        except Exception as e:
            log.write(f"{k} error {e}\n"); log.flush(); print("error", k, str(e)[:200]); break
finally:
    for s in spin: s.kill()
print(open(f"{AD}/gflow-calls.log").read()[-600:])
