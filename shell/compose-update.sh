DEPLOY=$1
COMPOSE_FILE=$2
APPLY_CONFIG=$3
RESTART_COUNT=$4
shift 4
cd -- "$DEPLOY"
if [ ! -f "$COMPOSE_FILE" ]; then
  echo "部署目录缺少 $COMPOSE_FILE，请先完成首次安装" >&2
  exit 4
fi
if docker compose version >/dev/null 2>&1; then COMPOSE=(docker compose); else COMPOSE=(docker-compose); fi
compose() { "${COMPOSE[@]}" -f "$COMPOSE_FILE" "$@"; }
RESTART_SERVICES=()
for ((index=0; index<RESTART_COUNT; index++)); do RESTART_SERVICES+=("$1"); shift; done
WAIT_SERVICES=("$@")
if [ "$APPLY_CONFIG" = yes ]; then
  echo '==> 应用部署配置'
  compose up -d --pull never --no-build --remove-orphans
fi
if [ ${#RESTART_SERVICES[@]} -gt 0 ]; then
  echo '==> 重启所选服务'
  compose restart -- "${RESTART_SERVICES[@]}"
fi
