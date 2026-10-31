#!/usr/bin/env bash
# Samples the connection `happy` test many times in parallel to expose the reply-order race.
set -u
bin="$1"; loops="$2"; iters="$3"; out="$4"
mkdir -p "$out"
for l in $(seq "$loops"); do
  ( for i in $(seq "$iters"); do
      "$bin" happy --exact --test-threads 1 > "$out/run-$l-$i.log" 2>&1 && rm "$out/run-$l-$i.log"
    done ) &
done
wait
echo "samples $((loops*iters)) failures $(ls "$out" | wc -l)"
