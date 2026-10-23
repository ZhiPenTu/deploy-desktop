if [ ${#WAIT_SERVICES[@]} -gt 0 ]; then
unready=()
for ((attempt=0; attempt<90; attempt++)); do
  unready=()
  for service in "${WAIT_SERVICES[@]}"; do
    ids=$(compose ps -a -q -- "$service")
    if [ -z "$ids" ]; then
      echo "服务 $service 未创建，请检查交付包的服务声明与当前部署清单是否一致" >&2
      compose logs --tail 20 -- "$service" 2>&1 || true
      exit 4
    fi
    for id in $ids; do
      health=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$id" 2>/dev/null || echo missing)
      state=$(docker inspect --format '{{.State.Status}}' "$id" 2>/dev/null || echo missing)
      if [ "$state" = running ] && { [ "$health" = healthy ] || [ "$health" = none ]; }; then continue; fi
      unready+=("$service")
    done
  done
  if [ ${#unready[@]} -eq 0 ]; then break; fi
  sleep 2
done
if [ ${#unready[@]} -gt 0 ]; then
  for service in "${unready[@]}"; do
    echo "服务 $service 未在等待时间内就绪" >&2
    compose logs --tail 40 -- "$service" 2>&1 || true
  done
  exit 4
fi
fi
