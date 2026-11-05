#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-}" in
  check) cargo check --locked --workspace ;;
  test) cargo test --locked --workspace ;;
  run) shift; cargo run --locked -p ikb-failover -- "$@" ;;
  *) printf '%s\n' 'Usage: script/dev.sh check|test|run [arguments]'; exit 2 ;;
esac
