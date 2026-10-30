#!/usr/bin/env bash
# usage: stress.sh N LOG ; N runs of the probe, 8 at a time
run(){ out=$(cargo nextest run -p ovis-terminal-holder --test platform_probe --locked 2>&1); rc=$?; echo "run $1 exit=$rc"; [ $rc -ne 0 ] && echo "$out" | grep -E "left a thread|FAIL|panicked" | head -5; return 0; }
export -f run
seq 1 "$1" | xargs -P${P:-8} -I{} bash -c 'run {}' > "$2" 2>&1
echo "total=$(grep -c ' exit=' "$2") ok=$(grep -c 'exit=0' "$2") failed=$(grep -c -v -E 'exit=0|^ ' "$2")" | tee -a "$2"
