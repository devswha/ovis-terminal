
import json, os, subprocess, sys, time, uuid
BIN=os.environ["BIN"]; AD=os.environ["AD"]; CORES=os.environ.get("CORES","124-125"); N=int(os.environ.get("N","300"))
root=f"{AD}/lat-{uuid.uuid4().hex[:6]}"; os.makedirs(root)
spin=[subprocess.Popen(["taskset","-c",CORES,"bash","-c","while :; do :; done"]) for _ in range(8)]
srv=subprocess.Popen(["taskset","-c",CORES,BIN,"serve","--root",root,"--stdin-lifetime"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
try:
    ready=json.loads(srv.stdout.readline())["ready"]
    env={"PATH":"/usr/bin:/bin","OVIS_TERMINAL_SOCKET":ready["socket"],"OVIS_TERMINAL_CREDENTIAL":ready["bearer"]}
    def call(m,p):
        t=time.monotonic(); r=subprocess.run(["taskset","-c",CORES,BIN,"--json","call",m,"--params",json.dumps(p)],env=env,capture_output=True,text=True); return time.monotonic()-t, r.returncode
    p={"op_id":str(uuid.uuid7()),"program":{"argv":["/bin/sh"]},"geometry":{"cols":80,"rows":24},"cwd":"/tmp","env":{"PATH":"/usr/bin:/bin","PS1":"$ ","TERM":"xterm"},"name":"shell","labels":None,"parent":None,"child_policy":None,"keep":None,"first_message":None,"machine":None}
    print("run", call("terminal.run",p))
    lat=[]; codes=[]
    for _ in range(N):
        d,c=call(os.environ.get("METHOD","update.query"),{}); lat.append(d); codes.append(c)
    lat.sort()
    print(json.dumps({"method":os.environ.get("METHOD","update.query"),"n":N,"p50":round(lat[N//2],3),"p99":round(lat[int(N*0.99)],3),"max":round(lat[-1],3),"over2s":sum(1 for x in lat if x>2),"nonzero_exit":sum(1 for c in codes if c)}))
finally:
    for s in spin: s.kill()
    srv.stdin.close(); srv.wait(timeout=120)
