#!/bin/sh
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
  printf '%s\n' "This measurement runner requires macOS." >&2
  exit 1
fi
if [ "$#" -ne 2 ]; then
  printf '%s\n' "Usage: $0 <lib-test-binary> <large|small>" >&2
  exit 1
fi

test_binary=$1
scenario=$2
case "$scenario" in
  large | small) ;;
  *)
    printf '%s\n' "Scenario must be large or small." >&2
    exit 1
    ;;
esac

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/codex-tools-provider-repair.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT INT TERM
home="$work_dir/codex-tools-provider-sync-$scenario"
time_file="$work_dir/time.txt"
pid_file="$work_dir/pid.txt"

CODEX_TOOLS_PROVIDER_REPAIR_MODE=setup \
CODEX_TOOLS_PROVIDER_REPAIR_SCENARIO="$scenario" \
CODEX_TOOLS_PROVIDER_REPAIR_HOME="$home" \
  "$test_binary" \
  --exact provider_sync::tests::measure_synthetic_provider_repair \
  --ignored --nocapture --test-threads=1

CODEX_TOOLS_PROVIDER_REPAIR_MODE=run \
CODEX_TOOLS_PROVIDER_REPAIR_SCENARIO="$scenario" \
CODEX_TOOLS_PROVIDER_REPAIR_HOME="$home" \
  /usr/bin/time -l -o "$time_file" sh -c '
    pid_file=$1
    shift
    printf "%s\n" "$$" > "$pid_file"
    exec "$@"
  ' sh "$pid_file" "$test_binary" \
  --exact provider_sync::tests::measure_synthetic_provider_repair \
  --ignored --nocapture --test-threads=1 &
supervisor_pid=$!

startup_checks=0
while [ ! -s "$pid_file" ]; do
  if ! kill -0 "$supervisor_pid" 2>/dev/null; then
    status=0
    wait "$supervisor_pid" || status=$?
    printf '%s\n' "Measurement process exited before publishing its PID." >&2
    if [ "$status" -eq 0 ]; then
      status=1
    fi
    exit "$status"
  fi
  startup_checks=$((startup_checks + 1))
  if [ "$startup_checks" -ge 1000 ]; then
    kill "$supervisor_pid" 2>/dev/null || true
    wait "$supervisor_pid" 2>/dev/null || true
    printf '%s\n' "Timed out waiting for the measurement process PID." >&2
    exit 1
  fi
  sleep 0.01
done
test_pid=$(tr -d '[:space:]' < "$pid_file")
max_threads=0
while kill -0 "$supervisor_pid" 2>/dev/null; do
  threads=$(ps -M -p "$test_pid" -o pid= 2>/dev/null | wc -l | tr -d '[:space:]')
  if [ "$threads" -gt "$max_threads" ]; then
    max_threads=$threads
  fi
  sleep 0.01
done

status=0
wait "$supervisor_pid" || status=$?
printf '%s\n' "maximum sampled threads (10 ms interval): $max_threads"
while IFS= read -r line; do
  case "$line" in
    *real* | *maximum\ resident\ set\ size*) printf '%s\n' "$line" ;;
  esac
done < "$time_file"
exit "$status"
