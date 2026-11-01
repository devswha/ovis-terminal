
import json, os, subprocess, sys, time, uuid, signal
BIN=os.environ["BIN"]; AD=os.environ["AD"]; CORES=os.environ.get("CORES","124-125"); N=int(os.environ.get("N","400")); SLOW=float(os.environ.get("SLOW","3"))
root=f"{AD}/glat-{uuid.uuid4().hex[:6]}"; os.makedirs(root)
log=open(f"{AD}/glat-calls.log","w")
spin=[subprocess.Popen(["taskset","-c",CORES,"bash","-c","while :; do :; done"]) for _ in range(int(os.environ.get("SPIN","8")))]
gdb=subprocess.Popen(["taskset","-c",CORES,"gdb","-q","-batch","-ex","set pagination off","-ex","handle SIGINT stop print nopass","-ex","handle SIGPIPE nostop noprint pass","-ex","run","-ex","thread apply all bt 40","-ex","kill","--args",BIN,"serve","--root",root,"--stdin-lifetime"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
caught=None
try:
    while True:
        line=gdb.stdout.readline()
        if not line: sys.exit("no ready")
        if line.startswith('{"ready"'): ready=json.loads(line)["ready"]; break
    inferior=int(open(f"/proc/{gdb.pid}/task/{gdb.pid}/children").read().split()[0]); assert inferior>1
    env={"PATH":"/usr/bin:/bin","OVIS_TERMINAL_SOCKET":ready["socket"],"OVIS_TERMINAL_CREDENTIAL":ready["bearer"]}
    p={"op_id":str(uuid.uuid7()),"program":{"argv":["/bin/sh"]},"geometry":{"cols":80,"rows":24},"cwd":"/tmp","env":{"PATH":"/usr/bin:/bin","PS1":"$ ","TERM":"xterm"},"name":"shell","labels":None,"parent":None,"child_policy":None,"keep":None,"first_message":None,"machine":None}
    subprocess.run(["taskset","-c",CORES,BIN,"--json","call","terminal.run","--params",json.dumps(p)],env=env,capture_output=True,timeout=120)
    for i in range(N):
        t=time.monotonic()
        c=subprocess.Popen(["taskset","-c",CORES,BIN,"--json","call",os.environ.get("METHOD","update.query"),"--params","{}"],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
        try:
            c.wait(timeout=SLOW); log.write(f"{i} {time.monotonic()-t:.3f} {c.returncode}\n"); log.flush()
        except subprocess.TimeoutExpired:
            caught=i; os.kill(inferior, signal.SIGINT); c.wait(); log.write(f"{i} SLOW>{SLOW} caught\n"); log.flush(); break
finally:
    for s in spin: s.kill()
if caught is None: gdb.stdin.close()
out=gdb.communicate(timeout=900)[0]
open(f"{AD}/glat-bt.log","w").write(out)
print("caught", caught)
