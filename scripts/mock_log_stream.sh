#!/usr/bin/env bash
set -euo pipefail

OUT="${1:-/tmp/logtm_live.log}"
echo "Writing mock log stream to ${OUT} (ctrl-c to stop)" >&2

counter=0
while true; do
  counter=$((counter + 1))
  ts="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  level="INFO"
  if (( counter % 15 == 0 )); then
    level="ERROR"
  elif (( counter % 7 == 0 )); then
    level="WARN"
  fi
  targets=("http" "db" "cache" "worker" "auth" "search")
  tgt=${targets[$((counter % ${#targets[@]}))]}
  msg="msg_id=${counter} action=test path=/resource/${counter} user=${counter%10}"
  echo "${ts} ${level} ${tgt} ${msg}" >> "${OUT}"
  sleep 0.3
done
