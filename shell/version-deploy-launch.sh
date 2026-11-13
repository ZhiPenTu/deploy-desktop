set -euo pipefail
umask 077
TXN=$1
shift
chmod 700 "$TXN/worker.sh"
cd "$TXN"
if command -v setsid >/dev/null 2>&1; then
  setsid bash "$TXN/worker.sh" "$@" > "$TXN/worker.log" 2>&1 < /dev/null &
else
  nohup bash "$TXN/worker.sh" "$@" > "$TXN/worker.log" 2>&1 < /dev/null &
fi
printf 'WORKER_STARTED\n'
