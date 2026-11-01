
import json, os, subprocess, sys, time, uuid, signal
BIN="/dev/shm/ovt-target-lane-t31/debug/ovis-terminal"
AD=os.environ["AD"]; root=f"{AD}/gdbroot-{uuid.uuid4().hex[:8]}"; os.makedirs(root)
spin=[subprocess.Popen(["taskset","-c","0-1","bash","-c","while :; do :; done"]) for _ in range(8)]
gdb=subprocess.Popen(["taskset","-c","0-1","gdb","-q","-batch","-ex","set pagination off","-ex","handle SIGINT stop print nopass","-ex","handle SIGPIPE nostop noprint pass","-ex","run","-ex","thread apply all bt 30","-ex","kill","--args",BIN,"serve","--root",root,"--stdin-lifetime"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=open(f"{AD}/gdb-stderr.log","w"),text=True)
ready=None
while True:
    line=gdb.stdout.readline()
    if not line: sys.exit("no ready")
    if line.startswith('{"ready"'): ready=json.loads(line)["ready"]; break
inferior=int(open(f"/proc/{gdb.pid}/task/{gdb.pid}/children").read().split()[0])
assert inferior>1
env={"PATH":"/usr/bin:/bin","OVIS_TERMINAL_SOCKET":ready["socket"],"OVIS_TERMINAL_CREDENTIAL":ready["bearer"]}
def call(m,p):
    t=time.monotonic(); c=subprocess.Popen(["taskset","-c","0-1",BIN,"--json","call",m,"--params",json.dumps(p)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    return c,t
caught=None; times=[]
for i in range(int(os.environ.get("N","30"))):
    p={"op_id":str(uuid.uuid7()) if hasattr(uuid,"uuid7") else str(uuid.uuid1()),"program":{"argv":["/bin/sh"]},"geometry":{"cols":80,"rows":24},"cwd":"/tmp","env":{"PATH":"/usr/bin:/bin"},"name":f"s{i}","labels":None,"parent":None,"child_policy":None,"keep":None,"first_message":None,"machine":None}
    c,t=call("terminal.run",p)
    try:
        c.wait(timeout=6); times.append(round(time.monotonic()-t,2))
    except subprocess.TimeoutExpired:
        caught=i; os.kill(inferior, signal.SIGINT); c.wait(); times.append(round(time.monotonic()-t,2)); break
for s in spin: s.kill()
if caught is None:
    gdb.stdin.close()
out=gdb.communicate(timeout=300)[0]
open(f"{AD}/gdb-bt.log","w").write(out)
print("times", times, "caught", caught)
