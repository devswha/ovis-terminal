#!/usr/bin/env bash
# Dumps process and host state once per long-running test process (over 60 s).
seen=" "
while :; do
  for tp in $(pgrep -f 'deps/assembly-.*--exact' || true); do
    age=$(ps -o etimes= -p $tp 2>/dev/null | tr -d ' '); [ -z "$age" ] && continue
    if [ "$age" -gt 60 ] && [[ "$seen" != *" $tp "* ]]; then
      seen="$seen$tp "; out=/dev/shm/ovt-fixtures/t31-scratch/stall-auto-$tp.txt
      { date +%H:%M:%S; echo "test $tp age=$age $(tr '\0' ' ' < /proc/$tp/cmdline | cut -c1-160)"; cat /proc/loadavg
        for f in cpu memory io; do echo "psi $f $(head -1 /proc/pressure/$f)"; done
        echo "--- top on cores 124-125"; ps -eo psr,pcpu,stat,pid,comm --sort=-pcpu | awk '$1==124||$1==125' | head -15
        for p in $tp $(pgrep -P $tp) $(for c in $(pgrep -P $tp); do pgrep -P $c; done); do
          echo "== $p $(ps -o stat=,etimes=,args= -p $p | cut -c1-140)"
          for t in /proc/$p/task/*; do echo "   $(cat $t/comm) $(cat $t/wchan) $(awk '{print $3, "u="$14, "s="$15}' $t/stat)"; done
        done; } > $out 2>&1
    fi
  done
  sleep 5
done
