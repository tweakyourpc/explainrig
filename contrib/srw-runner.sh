#!/usr/bin/env bash
# Explainrig runner for a Showrig worker reached with `srw` (NeoForge → legion). Not part of the
# product: it shows how a machine that renders elsewhere plugs in through the command templates.
#
#   ~/.config/explainrig/config.json
#   {"prepare": ["/path/to/srw-runner.sh", "prepare", "{film}"],
#    "render":  ["/path/to/srw-runner.sh", "render", "{film}"],
#    "board":   ["showrig", "board", "{film}"]}
#
# prepare: copy the film to the worker, run `showrig make --until plan` there (voice + plan), and
#          bring work/plan.json and work/voice back, with the worker's paths rewritten to local ones.
# render:  `srw render` (pack, render, fetch, validate, file, and upload to Drive unless
#          EXPLAINRIG_SRW_ARGS=--no-upload), then print the video's path for Explainrig.
set -euo pipefail

mode="${1:?usage: srw-runner.sh prepare|render FILM}"
film="$(readlink -f "${2:?usage: srw-runner.sh prepare|render FILM}")"
base="$(basename "$film")"
stamp="$(date +%Y%m%d-%H%M%S)"

case "$mode" in
  prepare)
    id="explainrig-prep-$stamp"
    srw put "$id" --exclude=work --exclude=out "$film" >/dev/null
    # the job runs in inputs/$id; it records where it ran so the paths in plan.json can be rewritten
    srw start "$id" "showrig make '$base' --until plan && mkdir -p \"\$SRJOB_OUT/work\" && cp -r '$base/work/plan.json' '$base/work/voice' \"\$SRJOB_OUT/work/\" && pwd > \"\$SRJOB_OUT/remote-root\""
    srw log "$id" -f </dev/null || true
    srw wait "$id" >/dev/null
    dest="$HOME/showrig-results/$id"
    srw get "$id" "$dest" >/dev/null
    remote="$(cat "$dest/output/remote-root")/$base"
    mkdir -p "$film/work"
    rm -rf "$film/work/voice"
    cp -r "$dest/output/work/voice" "$film/work/voice"
    python3 -I - "$dest/output/work/plan.json" "$film/work/plan.json" "$remote" "$film" <<'PY'
import sys
src, dst, remote, local = sys.argv[1:5]
text = open(src, encoding="utf-8").read().replace(remote, local)
open(dst, "w", encoding="utf-8").write(text)
PY
    echo "prepared on the worker as $id"
    ;;
  render)
    id="explainrig-$stamp"
    # shellcheck disable=SC2086
    srw render "$film" --id "$id" ${EXPLAINRIG_SRW_ARGS:-}
    video="$(find "$HOME/showrig-results/$id/output" -maxdepth 1 -name '*.mp4' | head -n 1)"
    [[ -n "$video" ]] && echo "explainrig-output: $video"
    ;;
  *)
    echo "srw-runner.sh: unknown mode $mode (prepare or render)" >&2
    exit 2
    ;;
esac
