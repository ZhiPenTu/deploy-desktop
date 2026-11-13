DEPLOY=$1
COMPOSE_FILE=$2
APPLY_CONFIG=$3
RESTART_COUNT=$4
shift 4
cd -- "$DEPLOY"
if [ ! -f "$COMPOSE_FILE" ]; then
  echo "部署目录缺少 ${COMPOSE_FILE}，请先完成首次安装" >&2
  exit 4
fi
if docker compose version >/dev/null 2>&1; then
  COMPOSE=(docker compose)
  COMPOSE_V2=yes
else
  COMPOSE=(docker-compose)
  COMPOSE_V2=no
fi
compose() {
  if [ -n "${DEPLOY_IMAGE_OVERRIDE:-}" ]; then
    "${COMPOSE[@]}" -f "$COMPOSE_FILE" -f "$DEPLOY_IMAGE_OVERRIDE" "$@"
  else
    "${COMPOSE[@]}" -f "$COMPOSE_FILE" "$@"
  fi
}
compose_with_extra() {
  extra=$1
  shift
  if [ -n "${DEPLOY_IMAGE_OVERRIDE:-}" ]; then
    "${COMPOSE[@]}" -f "$COMPOSE_FILE" -f "$DEPLOY_IMAGE_OVERRIDE" -f "$extra" "$@"
  else
    "${COMPOSE[@]}" -f "$COMPOSE_FILE" -f "$extra" "$@"
  fi
}
RESTART_SERVICES=()
for ((index=0; index<RESTART_COUNT; index++)); do RESTART_SERVICES+=("$1"); shift; done
WAIT_SERVICES=("$@")
if [ "$APPLY_CONFIG" = yes ]; then
  echo '==> 应用部署配置'
  compose_log=$(mktemp "${TMPDIR:-/tmp}/deploy-compose.XXXXXX") || exit 2
  no_pull_override=
  cleanup_compose_log() { rm -f "$compose_log" ${no_pull_override:+"$no_pull_override"}; }
  trap cleanup_compose_log EXIT
  if compose up -d --pull never --no-build --remove-orphans 2>&1 | tee "$compose_log"; then
    :
  else
    compose_status=$?
    if [ "$COMPOSE_V2" = yes ] && { grep -qi 'unknown flag.*pull' "$compose_log" || grep -qi 'flag provided but not defined.*pull' "$compose_log"; }; then
      services=$("${COMPOSE[@]}" -f "$COMPOSE_FILE" config --services 2>/dev/null) || services=
      if [ -n "$services" ]; then
        no_pull_override=$(mktemp "${TMPDIR:-/tmp}/deploy-no-pull.XXXXXX") || exit 2
        {
          printf 'services:\n'
          printf '%s\n' "$services" | while IFS= read -r service; do
            [ -n "$service" ] && printf '  %s:\n    pull_policy: never\n' "$service"
          done
        } > "$no_pull_override"
        echo '==> 当前 Compose 不支持 --pull，改为固定不拉取镜像后重试' >&2
        compose_with_extra "$no_pull_override" up -d --no-build --remove-orphans || exit $?
      else
        exit "$compose_status"
      fi
    else
      exit "$compose_status"
    fi
  fi
fi
if [ ${#RESTART_SERVICES[@]} -gt 0 ]; then
  echo '==> 重启所选服务'
  compose restart -- "${RESTART_SERVICES[@]}"
fi
