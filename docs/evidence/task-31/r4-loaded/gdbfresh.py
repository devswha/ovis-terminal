
import json, os, subprocess, sys, time, uuid, signal
BIN="/dev/shm/ovt-target-lane-t31/debug/ovis-terminal"
AD=os.environ["AD"]
def one(k):
    root=f"{AD}/fresh-{k}-{uuid.uuid4().hex[:6]}"; os.makedirs(root)
    gdb=subprocess.Popen(["taskset","-c","0-1","gdb","-q","-batch","-ex","set pagination off","-ex","handle SIGINT stop print nopass","-ex","handle SIGPIPE nostop noprint pass","-ex","run","-ex","thread apply all bt 30","-ex","kill","--args",BIN,"serve","--root",root,"--stdin-lifetime"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
    t0=time.monotonic()
    while True:
        line=gdb.stdout.readline()
        if not line: return None, None, "no ready"
        if line.startswith('{"ready"'): ready=json.loads(line)["ready"]; break
    started=round(time.monotonic()-t0,2)
    inferior=int(open(f"/proc/{gdb.pid}/task/{gdb.pid}/children").read().split()[0]); assert inferior>1
    env={"PATH":"/usr/bin:/bin","OVIS_TERMINAL_SOCKET":ready["socket"],"OVIS_TERMINAL_CREDENTIAL":ready["bearer"]}
    p={"op_id":str(uuid.uuid7()),"program":{"argv":["/bin/sh"]},"geometry":{"cols":80,"rows":24},"cwd":"/tmp","env":{"PATH":"/usr/bin:/bin"},"name":"shell","labels":None,"parent":None,"child_policy":None,"keep":None,"first_message":None,"machine":None}
    t=time.monotonic()
    c=subprocess.Popen(["taskset","-c","0-1",BIN,"--json","call","terminal.run","--params",json.dumps(p)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    caught=None
    try:
        c.wait(timeout=6)
    except subprocess.TimeoutExpired:
        caught=k; os.kill(inferior, signal.SIGINT); c.wait()
    took=round(time.monotonic()-t,2)
    if caught is None: gdb.stdin.close()
    out=gdb.communicate(timeout=600)[0]
    if caught is not None: open(f"{AD}/gdb-fresh-{k}.log","w").write(out)
    return started, took, caught
spin=[subprocess.Popen(["taskset","-c","0-1","bash","-c","while :; do :; done"]) for _ in range(8)]
res=[]
try:
    for k in range(int(os.environ.get("SERVERS","20"))):
        res.append(one(k))
finally:
    for s in spin: s.kill()
print("ready-after", [r[0] for r in res]); print("first-call", [r[1] for r in res]); print("caught", [r[2] for r in res if r[2] is not None])
