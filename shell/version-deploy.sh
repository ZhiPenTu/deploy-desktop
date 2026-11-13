#!/usr/bin/env bash
# Full-version remote deployment worker. All arguments are data.
set -Eeuo pipefail
umask 077
MODE=$1
INSTALL=$2
DEPLOY=$3
TXN=$4
HAD_DEPLOYMENT=$5
OLD_COMPOSE=$6
NEW_COMPOSE=$7
FRONTEND_SERVICE=$8
FRONTEND_PORT=${9:-}
NEW_SERVICES=${10:-}
OLD_SERVICES=${11:-}
NEW_OVERRIDE=${12:-}
OLD_OVERRIDE=${13:-}
OLD_FRONTEND_PORT=${14-$FRONTEND_PORT}

SNAPSHOT="$TXN/snapshot"
INCOMING="$TXN/incoming"
STATUS="$TXN/status.env"
. "$TXN/baseline.sh"
CURRENT_PHASE=preparing
RECOVERING=no
RECOVERY_REASON=
TRANSACTION_STARTED=no
FILES_APPLIED=no
LOCK=

case "$INSTALL" in
  /*) ;;
  *) echo '部署目录必须是绝对路径' >&2; exit 2 ;;
esac
case "$INSTALL" in
  /|*/|*/../*|*/..|*/./*|*/.|*$'\n'*|*$'\r'*) echo '部署目录无效' >&2; exit 2 ;;
esac

write_status() {
  phase=$1
  state=$2
  error_text=${3:-}
  recovery_text=${4:-}
  mkdir -p "$TXN"
  temp="$STATUS.tmp.$$"
  {
    printf 'phase=%s\n' "$phase"
    printf 'state=%s\n' "$state"
    if [ -f "$TXN/deployment.started" ]; then printf 'deploymentStarted=yes\n'; else printf 'deploymentStarted=no\n'; fi
    printf 'updatedAt=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf 'errorBase64='
    printf '%s' "$error_text" | base64 | tr -d '\n'
    printf '\n'
    printf 'recoveryErrorBase64='
    printf '%s' "$recovery_text" | base64 | tr -d '\n'
    printf '\n'
  } > "$temp"
  mv -f "$temp" "$STATUS"
}

compose_bin() {
  if timeout 30 docker compose version >/dev/null 2>&1; then
    COMPOSE=(docker compose)
  elif command -v docker-compose >/dev/null 2>&1; then
    COMPOSE=(docker-compose)
  else
    echo '目标机缺少 Docker Compose' >&2
    return 3
  fi
}

acquire_project_lock() {
  local project_lock directory
  [ -f "$TXN/project-lock" ] || return 0
  project_lock=$(cat "$TXN/project-lock")
  case "$project_lock" in
    /tmp/deploy-desktop-project-locks/*.lock) ;;
    *) echo 'Compose 项目锁路径无效' >&2; return 1 ;;
  esac
  [ ! -L "$project_lock" ] || { echo 'Compose 项目锁不能使用符号链接' >&2; return 1; }
  directory=$(dirname -- "$project_lock")
  [ ! -L "$directory" ] || { echo 'Compose 项目锁目录不能使用符号链接' >&2; return 1; }
  mkdir -p -- "$directory"
  exec 8>>"$project_lock"
  if ! flock -n 8; then
    echo '同一 Compose 项目正在其他部署目录执行任务' >&2
    return 1
  fi
  printf '%s\n' "$$" > "$project_lock"
}

compose() {
  local env_args=()
  [ -f "$DEPLOY/.env" ] && env_args+=(--env-file "$DEPLOY/.env")
  [ -f "$DEPLOY/.env.local" ] && env_args+=(--env-file "$DEPLOY/.env.local")
  if [ "${COMPOSE_TIMEOUT:-0}" -gt 0 ]; then
    timeout "$COMPOSE_TIMEOUT" "${COMPOSE[@]}" --project-directory "$DEPLOY" ${env_args[@]+"${env_args[@]}"} -f "$DEPLOY/$1" "${@:2}"
  else
    "${COMPOSE[@]}" --project-directory "$DEPLOY" ${env_args[@]+"${env_args[@]}"} -f "$DEPLOY/$1" "${@:2}"
  fi
}

compose_down() {
  local file=$1
  [ -n "$file" ] && [ -f "$DEPLOY/$file" ] || return 0
  COMPOSE_TIMEOUT=600 compose "$file" down --remove-orphans
}

compose_up() {
  local file=$1
  local override=${2:-}
  [ -n "$file" ] && [ -f "$DEPLOY/$file" ] || {
    echo "Compose 清单不存在：$file" >&2
    return 4
  }
  if [ -n "$override" ] && [ -f "$override" ]; then
    COMPOSE_TIMEOUT=1800 compose "$file" -f "$override" up -d --no-build --remove-orphans
  else
    COMPOSE_TIMEOUT=1800 compose "$file" up -d --no-build --remove-orphans
  fi
}

http_ready() {
  local port=$1 status
  if ! { exec 3<>"/dev/tcp/127.0.0.1/${port}"; } 2>/dev/null; then return 1; fi
  if ! printf 'GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n' >&3 2>/dev/null; then
    exec 3>&-
    return 1
  fi
  if ! IFS= read -r -t 8 status <&3; then
    exec 3>&-
    return 1
  fi
  exec 3>&-
  case "$status" in
    HTTP/1.0\ [23]*|HTTP/1.1\ [23]*|HTTP/2\ [23]*) return 0 ;;
  esac
  return 1
}

wait_services() {
  local file=$1
  local services=$2
  local port=${3:-}
  [ -n "$file" ] && [ -f "$DEPLOY/$file" ] || return 0
  local attempt unready id state health
  for attempt in $(seq 1 90); do
    unready=
    for service in $services; do
      ids=$(COMPOSE_TIMEOUT=60 compose "$file" ps -a -q "$service" 2>/dev/null || true)
      if [ -z "$ids" ]; then
        unready="$unready $service"
        continue
      fi
      for id in $ids; do
        state=$(timeout 60 docker inspect --format '{{.State.Status}}' "$id" 2>/dev/null || echo missing)
        health=$(timeout 60 docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$id" 2>/dev/null || echo missing)
        if [ "$state" != running ] || { [ "$health" != healthy ] && [ "$health" != none ]; }; then
          unready="$unready $service"
        fi
      done
    done
    if [ -z "$unready" ]; then
      if [ -n "$FRONTEND_SERVICE" ] && [ -n "$port" ] && [[ " $services " = *" $FRONTEND_SERVICE "* ]]; then
        if ! http_ready "$port"; then
          unready="$FRONTEND_SERVICE http"
        fi
      fi
      [ -z "$unready" ] && return 0
    fi
    sleep 2
  done
  echo "服务未在等待时间内就绪：$unready" >&2
  for service in $unready; do
    [ "$service" = http ] && continue
    COMPOSE_TIMEOUT=60 compose "$file" logs --tail 40 "$service" >&2 || true
  done
  return 4
}

services_from_compose() {
  local file=$1
  [ -n "$file" ] && [ -f "$DEPLOY/$file" ] || return 0
  COMPOSE_TIMEOUT=60 compose "$file" config --services 2>/dev/null || true
}

path_is_internal() {
  local first=${1%%/*}
  local alias=${first%%:*}
  alias=$(printf '%s' "$alias" | tr '[:upper:]' '[:lower:]' | sed 's/[. ]*$//')
  internal_state_name "$alias"
}

internal_state_name() {
  local alias=$1 body head tail
  [ "$alias" = ".deploy-desktop" ] && return 0
  body=${alias#[.]}
  case "$body" in
    *~*) ;;
    *) return 1 ;;
  esac
  head=${body%%~*}
  tail=${body#*~}
  case "$tail" in
    ''|*[!0-9]*) return 1 ;;
  esac
  [ -n "$head" ] && [ "${#head}" -le 6 ] || return 1
  case "$head" in
    *[!a-z0-9]*) return 1 ;;
  esac
}

validate_managed_path() {
  local relative=$1
  case "$relative" in
    ""|.|./*|/*|..|../*|*/../*|*/..|*\\*|*$'\t'*|*$'\n'*|*$'\r'*) return 1 ;;
  esac
  if path_is_internal "$relative"; then
    [ "$relative" = ".deploy-desktop/deployed.json" ] || return 1
  fi
}

validate_artifact_path() {
  local relative=$1
  validate_managed_path "$relative" || return 1
  ! path_is_internal "$relative"
}

validate_project_scope() {
  [ -f "$TXN/project-name" ] || return 0
  local project working
  project=$(cat "$TXN/project-name")
  [ -n "$project" ] || return 0
  local output
  output=$(timeout 60 docker ps -a \
    --filter "label=com.docker.compose.project=$project" \
    --format '{{.Label "com.docker.compose.project.working_dir"}}' 2>/dev/null) || {
    echo '无法检查 Compose 项目容器边界' >&2
    return 1
  }
  [ -n "$output" ] || return 0
  while IFS= read -r working; do
    [ -n "$working" ] || {
      echo "Compose 项目 $project 的容器缺少工作目录标签，无法确认部署边界" >&2
      return 1
    }
    case "$working" in
      "$DEPLOY"|"$DEPLOY"/*) ;;
      *)
        echo "Compose 项目 $project 已被其他部署目录使用（${working}），已阻止部署" >&2
        return 1
        ;;
    esac
  done <<< "$output"
}

snapshot_files() {
  mkdir -p "$SNAPSHOT/files"
  : > "$SNAPSHOT/files.list"
  [ -f "$INCOMING/managed-paths.txt" ] || return 0
  while IFS= read -r relative; do
    [ -n "$relative" ] || continue
    validate_managed_path "$relative" || { echo "非法快照路径：$relative" >&2; return 5; }
    mkdir -p "$SNAPSHOT/files/$(dirname -- "$relative")"
    if [ -e "$INSTALL/$relative" ] || [ -L "$INSTALL/$relative" ]; then
      rm -rf -- "$SNAPSHOT/files/$relative"
      cp -a -- "$INSTALL/$relative" "$SNAPSHOT/files/$relative"
      bytes=$(du -sk -- "$SNAPSHOT/files/$relative" | awk '{print $1 * 1024}')
      digest=$(tar -cf - -C "$SNAPSHOT/files" -- "$relative" | sha256sum | awk '{print $1}')
      printf 'yes\t%s\t%s\t%s\n' "$bytes" "$digest" "$relative" >> "$SNAPSHOT/files.list"
    else
      printf 'no\t0\t-\t%s\n' "$relative" >> "$SNAPSHOT/files.list"
    fi
  done < "$INCOMING/managed-paths.txt"
  touch "$SNAPSHOT/files.complete"
}

restore_files() {
  [ -f "$SNAPSHOT/files.complete" ] && [ -f "$SNAPSHOT/files.list" ] || return 0
  while IFS=$'\t' read -r existed bytes digest relative; do
    [ -n "$relative" ] || continue
    validate_managed_path "$relative" || { echo "非法恢复路径：$relative" >&2; return 11; }
    rm -rf -- "$INSTALL/$relative" || return 11
    if [ "$existed" = yes ] && { [ -e "$SNAPSHOT/files/$relative" ] || [ -L "$SNAPSHOT/files/$relative" ]; }; then
      mkdir -p -- "$(dirname -- "$INSTALL/$relative")" || return 12
      cp -a -- "$SNAPSHOT/files/$relative" "$INSTALL/$relative" || return 12
    fi
  done < "$SNAPSHOT/files.list"
}

check_volume_tools() {
  while IFS=$'\t' read -r logical actual image existed; do
    [ -n "$logical" ] || continue
    [ -n "$image" ] || { echo "数据卷 $logical 缺少备份镜像" >&2; return 6; }
    if ! timeout 300 docker run --rm --network none --entrypoint /bin/sh "$image" -c 'command -v tar >/dev/null && command -v gzip >/dev/null' >/dev/null 2>&1; then
      echo "备份镜像 $image 缺少 tar 或 gzip" >&2
      return 6
    fi
  done < "$INCOMING/volume-specs.tsv"
}

snapshot_volumes() {
  mkdir -p "$SNAPSHOT/volumes"
  : > "$SNAPSHOT/volume-backups.tsv"
  [ -f "$INCOMING/volume-specs.tsv" ] || return 0
  while IFS=$'\t' read -r logical actual image existed; do
    [ -n "$logical" ] || continue
    if [ "$existed" != yes ]; then
      continue
    fi
    backup="$SNAPSHOT/volumes/$logical.tar.gz"
    partial="$backup.part"
    rm -f -- "$partial"
  timeout 21600 docker run --rm --network none -v "$actual:/data:ro" --entrypoint /bin/sh "$image" -c 'tar -czf - -C /data .' > "$partial"
    mv -f -- "$partial" "$backup"
    gzip -t "$backup"
    tar -tzf "$backup" >/dev/null
    digest=$(sha256sum "$backup" | awk '{print $1}')
    bytes=$(wc -c < "$backup" | tr -d '[:space:]')
    printf '%s\t%s\t%s\t%s\t%s\n' "$logical" "$actual" "$image" "$digest" "$bytes" >> "$SNAPSHOT/volume-backups.tsv"
  done < "$INCOMING/volume-specs.tsv"
  touch "$SNAPSHOT/volumes.complete"
}

restore_volumes() {
  [ -f "$SNAPSHOT/volumes.complete" ] && [ -f "$SNAPSHOT/volume-backups.tsv" ] || return 0
  while IFS=$'\t' read -r logical actual image digest bytes; do
    [ -n "$logical" ] || continue
    backup="$SNAPSHOT/volumes/$logical.tar.gz"
    [ -f "$backup" ] || { echo "缺少数据卷备份：$logical" >&2; return 7; }
    [ "$(sha256sum "$backup" | awk '{print $1}')" = "$digest" ] || { echo "数据卷备份摘要不一致：$logical" >&2; return 7; }
    [ "$(wc -c < "$backup" | tr -d '[:space:]')" = "$bytes" ] || { echo "数据卷备份大小不一致：$logical" >&2; return 7; }
    gzip -t "$backup" || return 7
    tar -tzf "$backup" >/dev/null || return 7
    timeout 21600 docker run --rm --network none -i -v "$actual:/data" --entrypoint /bin/sh "$image" \
      -c 'find /data -mindepth 1 -maxdepth 1 -exec rm -rf -- {} + 2>/dev/null || true; [ -z "$(find /data -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)" ] && tar -xzf - -C /data' < "$backup" \
      || return 7
  done < "$SNAPSHOT/volume-backups.tsv"
}

remove_created_volumes() {
  [ -f "$INCOMING/volume-specs.tsv" ] || return 0
  while IFS=$'\t' read -r logical actual image existed; do
    [ -n "$actual" ] || continue
    if [ "$existed" != yes ]; then
      if timeout 60 docker volume inspect "$actual" >/dev/null 2>&1; then
        timeout 120 docker volume rm "$actual" >/dev/null || return 9
      fi
    fi
  done < "$INCOMING/volume-specs.tsv"
}

verify_snapshot() {
  if [ -f "$TXN/starting.started" ] && { [ ! -f "$SNAPSHOT/files.complete" ] || [ ! -f "$SNAPSHOT/volumes.complete" ]; }; then
    echo '启动阶段的文件或数据卷快照不完整，无法安全恢复' >&2
    return 10
  fi
  if [ -f "$SNAPSHOT/files.complete" ]; then
    [ -f "$SNAPSHOT/files.list" ] || { echo '文件快照清单缺失' >&2; return 10; }
    while IFS=$'\t' read -r existed bytes digest relative; do
      [ -n "$relative" ] || continue
      if [ "$existed" = yes ]; then
        if ! { [ -e "$SNAPSHOT/files/$relative" ] || [ -L "$SNAPSHOT/files/$relative" ]; }; then
          echo "文件快照缺失：$relative" >&2
          return 10
        fi
        [ "$(du -sk -- "$SNAPSHOT/files/$relative" | awk '{print $1 * 1024}')" = "$bytes" ] || { echo "文件快照大小不一致：$relative" >&2; return 10; }
        [ "$(tar -cf - -C "$SNAPSHOT/files" -- "$relative" | sha256sum | awk '{print $1}')" = "$digest" ] || { echo "文件快照摘要不一致：$relative" >&2; return 10; }
      fi
    done < "$SNAPSHOT/files.list"
  fi
  if [ -f "$SNAPSHOT/volumes.complete" ]; then
    [ -f "$SNAPSHOT/volume-backups.tsv" ] || { echo '数据卷快照清单缺失' >&2; return 10; }
    while IFS=$'\t' read -r logical actual image digest bytes; do
      [ -n "$logical" ] || continue
      backup="$SNAPSHOT/volumes/$logical.tar.gz"
      [ -f "$backup" ] || { echo "缺少数据卷备份：$logical" >&2; return 10; }
      [ "$(sha256sum "$backup" | awk '{print $1}')" = "$digest" ] || { echo "数据卷备份摘要不一致：$logical" >&2; return 10; }
      [ "$(wc -c < "$backup" | tr -d '[:space:]')" = "$bytes" ] || { echo "数据卷备份大小不一致：$logical" >&2; return 10; }
      gzip -t "$backup" || return 10
      tar -tzf "$backup" >/dev/null || return 10
    done < "$SNAPSHOT/volume-backups.tsv"
  fi
}

verify_previous_resources() {
  local image
  if [ "$HAD_DEPLOYMENT" = yes ] && [ -n "$OLD_COMPOSE" ]; then
    [ -f "$DEPLOY/$OLD_COMPOSE" ] || {
      echo '上一版本 Compose 清单缺失，无法安全恢复' >&2
      return 17
    }
    [ -n "$OLD_OVERRIDE" ] && [ -f "$OLD_OVERRIDE" ] || {
      echo '上一版本镜像清单缺失，无法确认将启动的镜像' >&2
      return 17
    }
    if ! grep -q '"image"[[:space:]]*:[[:space:]]*"' "$OLD_OVERRIDE"; then
      echo '上一版本镜像清单为空，无法安全恢复' >&2
      return 17
    fi
    while IFS= read -r image; do
      timeout 60 docker image inspect -- "$image" >/dev/null 2>&1 || {
        echo "上一版本镜像已不存在：$image" >&2
        return 17
      }
    done < <(
      grep -o '"image"[[:space:]]*:[[:space:]]*"[^"]*"' "$OLD_OVERRIDE" \
        | sed -E 's/.*"image"[[:space:]]*:[[:space:]]*"([^"]*)"/\1/'
    )
  fi
}

verify_recovery_resources() {
  local image
  verify_previous_resources || return 17
  [ -f "$SNAPSHOT/volume-backups.tsv" ] || return 0
  while IFS=$'\t' read -r logical actual image digest bytes; do
    [ -n "$logical" ] || continue
    timeout 60 docker image inspect -- "$image" >/dev/null 2>&1 || {
      echo "数据卷备份镜像已不存在：$image" >&2
      return 17
    }
    timeout 300 docker run --rm --network none --entrypoint /bin/sh "$image" \
      -c 'command -v tar >/dev/null && command -v gzip >/dev/null' >/dev/null 2>&1 || {
      echo "数据卷备份镜像缺少 tar 或 gzip：$image" >&2
      return 17
    }
  done < "$SNAPSHOT/volume-backups.tsv"
}

apply_files() {
  if [ -f "$INCOMING/remove-paths.txt" ]; then
    while IFS= read -r relative; do
      [ -n "$relative" ] || continue
      validate_artifact_path "$relative" || { echo "非法清理路径：$relative" >&2; return 5; }
      rm -rf -- "$INSTALL/$relative"
    done < "$INCOMING/remove-paths.txt"
  fi
  [ -f "$INCOMING/upload-paths.txt" ] || return 0
  while IFS= read -r relative; do
    [ -n "$relative" ] || continue
    validate_artifact_path "$relative" || { echo "非法写入路径：$relative" >&2; return 8; }
    source="$INCOMING/tree/$relative"
    [ -e "$source" ] || { echo "暂存内容缺少 $relative" >&2; return 8; }
    rm -rf -- "$INSTALL/$relative"
    mkdir -p -- "$(dirname -- "$INSTALL/$relative")"
    cp -a -- "$source" "$INSTALL/$relative"
    printf '%s\n' "$relative" >> "$TXN/applied-paths.txt"
  done < "$INCOMING/upload-paths.txt"
  if [ -f "$INCOMING/env-present" ] && [ -f "$INCOMING/env" ]; then
    mkdir -p -- "$DEPLOY"
    install -m 600 "$INCOMING/env" "$DEPLOY/.env"
  fi
}

recover_previous() {
  local reason=$1
  RECOVERING=yes
  RECOVERY_REASON=$reason
  CURRENT_PHASE=recovering
  write_status recovering recovering "$reason"
  verify_snapshot || return 10
  verify_recovery_resources || return 17
  if [ -f "$TXN/starting.started" ]; then
    [ -f "$DEPLOY/$NEW_COMPOSE" ] || { echo '失败版本的 Compose 清单缺失，无法确认服务已停止' >&2; return 11; }
    compose_down "$NEW_COMPOSE" || return 11
  fi
  restore_files || return 12
  if [ -f "$TXN/starting.started" ]; then
    restore_volumes || return 13
    remove_created_volumes || return 14
  fi
  if [ "$HAD_DEPLOYMENT" = yes ] && [ -n "$OLD_COMPOSE" ]; then
    compose_up "$OLD_COMPOSE" "$OLD_OVERRIDE" || return 15
    wait_services "$OLD_COMPOSE" "$OLD_SERVICES" "$OLD_FRONTEND_PORT" || return 16
  fi
  CURRENT_PHASE=recovered
  write_status recovered recovered "$reason"
}

on_error() {
  code=$?
  if [ "$RECOVERING" = yes ]; then
    write_status recovery_failed recovery_failed "$RECOVERY_REASON" "自动恢复失败，现场保留在 $SNAPSHOT"
    exit "$code"
  fi
  message="部署阶段 $CURRENT_PHASE 失败（退出码 ${code}）"
  if [ "$TRANSACTION_STARTED" != yes ]; then
    write_status "$CURRENT_PHASE" failed "$message"
    exit "$code"
  fi
  if [ "$FILES_APPLIED" != yes ]; then
    if [ "$HAD_DEPLOYMENT" = yes ] && [ -n "$OLD_COMPOSE" ] && [ -f "$DEPLOY/$OLD_COMPOSE" ]; then
      RECOVERING=yes
      RECOVERY_REASON=$message
      CURRENT_PHASE=recovering
      write_status recovering recovering "$message"
      if ! verify_previous_resources; then
        write_status recovery_failed recovery_failed "$message" "自动恢复失败，现场保留在 $SNAPSHOT"
        exit "$code"
      fi
      if compose_up "$OLD_COMPOSE" "$OLD_OVERRIDE" && wait_services "$OLD_COMPOSE" "$OLD_SERVICES" "$OLD_FRONTEND_PORT"; then
        CURRENT_PHASE=recovered
        write_status recovered recovered "$message"
        exit 0
      fi
      write_status recovery_failed recovery_failed "$message" "自动恢复失败，现场保留在 $SNAPSHOT"
      exit "$code"
    fi
    write_status "$CURRENT_PHASE" failed "$message"
    exit "$code"
  fi
  write_status "$CURRENT_PHASE" failed "$message"
  if recover_previous "$message"; then
    echo "部署失败，已恢复上一版本：$message" >&2
    if [ "$HAD_DEPLOYMENT" = yes ]; then
      exit 0
    fi
  else
    recovery_message="自动恢复失败，现场保留在 $SNAPSHOT"
    write_status recovery_failed recovery_failed "$message" "$recovery_message"
    echo "$recovery_message" >&2
  fi
  exit "$code"
}
trap on_error ERR
compose_bin
mkdir -p "$TXN"
mkdir -p "$INSTALL/.deploy-desktop"
LOCK="$INSTALL/.deploy-desktop/deploy.lock"
command -v flock >/dev/null 2>&1 || { echo '目标机缺少 flock，无法获取部署锁' >&2; false; }
[ ! -L "$LOCK" ] || { echo '部署锁不能使用符号链接' >&2; false; }
exec 9>>"$LOCK"
if ! flock -n 9; then
  echo '同一部署目录已有任务正在执行' >&2
  if [ ! -f "$STATUS" ]; then write_status preparing failed '同一部署目录已有任务正在执行'; fi
  exit 1
fi
printf '%s\n' "$$" > "$LOCK"
if [ -f "$TXN/old-frontend-port" ]; then OLD_FRONTEND_PORT=$(cat "$TXN/old-frontend-port"); fi

if [ "$MODE" = recover ]; then
  if [ -f "$STATUS" ] && grep -Eq '^state=(recovering|recovered|recovery_failed)$' "$STATUS"; then
    echo '该事务已开始过恢复，禁止重复执行，请检查事务现场' >&2
    exit 9
  fi
  if [ ! -f "$TXN/deployment.started" ]; then
    write_status recovered recovered '本次事务在修改目标前已停止，无需恢复服务、文件或数据卷'
    exit 0
  fi
  if ! acquire_project_lock; then
    write_status recovery_failed recovery_failed '同一 Compose 项目正在其他部署目录执行任务'
    exit 1
  fi
  if ! validate_project_scope; then
    write_status recovery_failed recovery_failed 'Compose 项目工作目录与当前部署目录不一致'
    exit 1
  fi
  [ -n "$OLD_SERVICES" ] || OLD_SERVICES=$(services_from_compose "$OLD_COMPOSE")
  [ -n "$NEW_SERVICES" ] || NEW_SERVICES=$(services_from_compose "$NEW_COMPOSE")
  TRANSACTION_STARTED=yes
  FILES_APPLIED=yes
  CURRENT_PHASE=recovering
  write_status recovering recovering "用户请求恢复上一版本"
  if recover_previous "用户请求恢复上一版本"; then
    exit 0
  fi
  write_status recovery_failed recovery_failed "用户请求恢复上一版本" "自动恢复失败，现场保留在 $SNAPSHOT"
  exit 9
fi

if ! acquire_project_lock; then
  write_status preparing failed '同一 Compose 项目正在其他部署目录执行任务'
  exit 1
fi
if ! validate_project_scope; then
  write_status preparing failed 'Compose 项目工作目录与当前部署目录不一致'
  exit 1
fi
CURRENT_PHASE=preparing
write_status preparing running
timeout 60 docker info >/dev/null 2>&1 || { echo 'Docker 未运行或当前用户无权访问' >&2; false; }
command -v tar >/dev/null 2>&1 || { echo '目标机缺少 tar' >&2; false; }
command -v gzip >/dev/null 2>&1 || { echo '目标机缺少 gzip' >&2; false; }
command -v sha256sum >/dev/null 2>&1 || { echo '目标机缺少 sha256sum' >&2; false; }
command -v timeout >/dev/null 2>&1 || { echo '目标机缺少 timeout，无法限制备份与恢复耗时' >&2; false; }
for other in "$INSTALL/.deploy-desktop/transactions/"*; do
  [ -d "$other" ] && [ "$other" != "$TXN" ] || continue
  if [ -f "$other/status.env" ]; then
    grep -Eq '^state=(committed|recovered)$' "$other/status.env" || {
      echo "存在其他未完成事务：$other" >&2; false;
    }
  elif [ -f "$other/journal.json" ]; then
    grep -Eq '"state"[[:space:]]*:[[:space:]]*"(committed|recovered)"' "$other/journal.json" || {
      echo "存在其他未完成事务或无效记录：$other" >&2; false;
    }
  fi
done
if [ -f "$INCOMING/expected-baseline" ]; then
  baseline=$(deployment_baseline "$INSTALL" "$DEPLOY" "$OLD_COMPOSE")
  [ "$baseline" = "$(cat "$INCOMING/expected-baseline")" ] || {
    echo '目标部署在预检后发生变化，已在停服前中止，请重新检查事务与版本' >&2; false;
  }
fi
managed_paths=()
while IFS= read -r relative; do
  if [ -n "$relative" ]; then managed_paths+=("$relative"); fi
done < "$INCOMING/managed-paths.txt"
deployment_managed_bytes "$INSTALL" "${managed_paths[@]}" >/dev/null
check_volume_tools
if [ "$HAD_DEPLOYMENT" = yes ]; then
  if ! verify_previous_resources; then
    write_status preparing failed '上一版本恢复资源缺失，已在停服前中止'
    exit 1
  fi
fi
touch "$TXN/deployment.started"
TRANSACTION_STARTED=yes

if [ "$HAD_DEPLOYMENT" = yes ]; then
  CURRENT_PHASE=snapshotting_files
  write_status snapshotting_files running
  compose_down "$OLD_COMPOSE" || { echo '停止旧版本失败，已中止部署' >&2; false; }
fi

CURRENT_PHASE=snapshotting_files
write_status snapshotting_files running
snapshot_files

CURRENT_PHASE=snapshotting_data
write_status snapshotting_data running
snapshot_volumes

CURRENT_PHASE=applying
write_status applying running
FILES_APPLIED=yes
apply_files

CURRENT_PHASE=starting
write_status starting running
touch "$TXN/starting.started"
compose_up "$NEW_COMPOSE" "$NEW_OVERRIDE"
wait_services "$NEW_COMPOSE" "$NEW_SERVICES" "$FRONTEND_PORT"

if [ -f "$INCOMING/deployed.json" ]; then
  mkdir -p "$INSTALL/.deploy-desktop"
  install -m 600 "$INCOMING/deployed.json" "$INSTALL/.deploy-desktop/deployed.json"
fi

CURRENT_PHASE=committed
write_status committed committed
rm -rf -- "$SNAPSHOT" 2>/dev/null || true
trap - ERR
