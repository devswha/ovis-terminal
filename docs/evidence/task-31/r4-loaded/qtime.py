
import json, os, subprocess, time, uuid
BIN=os.environ["BIN"]; AD=os.environ["AD"]; CORES="124-125"
def run(load):
    root=f"{AD}/q-{uuid.uuid4().hex[:6]}"; os.makedirs(root)
    spin=[subprocess.Popen(["taskset","-c",CORES,"bash","-c","while :; do :; done"]) for _ in range(8 if load else 0)]
    srv=subprocess.Popen(["taskset","-c",CORES,BIN,"serve","--root",root,"--stdin-lifetime"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
    try:
        ready=json.loads(srv.stdout.readline())["ready"]
        env={"PATH":"/usr/bin:/bin","OVIS_TERMINAL_SOCKET":ready["socket"],"OVIS_TERMINAL_CREDENTIAL":ready["bearer"]}
        res=[]
        for _ in range(3):
            t=time.monotonic(); r=subprocess.run(["taskset","-c",CORES,BIN,"--json","call","update.query","--params","{}"],env=env,capture_output=True,text=True,timeout=120)
            res.append((round(time.monotonic()-t,2), r.returncode, (r.stderr or r.stdout)[:90].replace("\n"," ")))
        return res
    finally:
        for s in spin: s.kill()
        srv.stdin.close(); srv.wait(timeout=120)
print("unloaded", run(False)); print("loaded", run(True))
