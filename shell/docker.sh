# 远程 Docker 统一入口。桌面端把它注入到每个 SSH 脚本，脚本内仍按普通
# `docker` / `docker-compose` 命令书写，由这里统一处理权限与 sudo 降级。
#
# 未安装 docker 时不定义函数，保留脚本里 `command -v docker` 的原有判断。
if command -v docker >/dev/null 2>&1; then
  DEPLOY_DOCKER_MODE=
  DEPLOY_DOCKER_DENIED=0
  DEPLOY_DOCKER_HINTED=0
  DEPLOY_DOCKER_BIN=$(type -P docker 2>/dev/null || command -v docker)
  DEPLOY_DOCKER_COMPOSE_BIN=
  if command -v docker-compose >/dev/null 2>&1; then
    DEPLOY_DOCKER_COMPOSE_BIN=$(type -P docker-compose 2>/dev/null || command -v docker-compose)
  fi

  # sudo 会重置环境，这里显式传递会改变 Docker 目标与凭据来源的变量。
  deploy_docker_sudo() {
    deploy_env=("DEPLOY_DOCKER_SUDO=1")
    if [ -n "${DOCKER_HOST:-}" ]; then deploy_env+=("DOCKER_HOST=$DOCKER_HOST"); fi
    if [ -n "${DOCKER_CONTEXT:-}" ]; then deploy_env+=("DOCKER_CONTEXT=$DOCKER_CONTEXT"); fi
    if [ -n "${DOCKER_CONFIG:-}" ]; then deploy_env+=("DOCKER_CONFIG=$DOCKER_CONFIG"); fi
    command sudo -n env "${deploy_env[@]}" "$@"
  }

  deploy_docker_pick() {
    if [ -n "${DEPLOY_DOCKER_MODE:-}" ]; then
      return 0
    fi
    if DEPLOY_DOCKER_PROBE=$(command "$DEPLOY_DOCKER_BIN" info 2>&1); then
      DEPLOY_DOCKER_MODE=direct
      return 0
    fi
    # 只有确认是 socket 权限问题时才降级，避免把命令切到另一套 Docker 上执行。
    case "$DEPLOY_DOCKER_PROBE" in
      *[Pp]ermission\ denied*[Dd]ocker*|*[Dd]ocker*[Pp]ermission\ denied*)
        DEPLOY_DOCKER_DENIED=1
        if command -v sudo >/dev/null 2>&1 \
          && deploy_docker_sudo "$DEPLOY_DOCKER_BIN" info >/dev/null 2>&1; then
          DEPLOY_DOCKER_MODE=sudo
          return 0
        fi
        ;;
    esac
    DEPLOY_DOCKER_MODE=direct
    return 1
  }

  deploy_docker_run() {
    deploy_name=$1
    shift
    if [ "$deploy_name" = "docker" ]; then
      deploy_bin=$DEPLOY_DOCKER_BIN
    else
      deploy_bin=$DEPLOY_DOCKER_COMPOSE_BIN
      [ -n "$deploy_bin" ] || deploy_bin=$deploy_name
    fi
    deploy_docker_pick || true
    if [ "${DEPLOY_DOCKER_MODE:-}" = "sudo" ]; then
      deploy_docker_sudo "$deploy_bin" "$@"
      return $?
    fi
    if command "$deploy_bin" "$@"; then
      return 0
    else
      DEPLOY_DOCKER_STATUS=$?
    fi
    if [ "$DEPLOY_DOCKER_DENIED" = 1 ] && [ "$DEPLOY_DOCKER_HINTED" = 0 ]; then
      DEPLOY_DOCKER_HINTED=1
      printf '%s\n' "Docker 权限不足：当前 SSH 用户无法访问 Docker。请执行 sudo usermod -aG docker ${USER:-<用户名>} 后重新登录，或为该用户配置免密 sudo docker。" >&2
    fi
    return "$DEPLOY_DOCKER_STATUS"
  }

  docker() {
    deploy_docker_run docker "$@"
  }

  if command -v docker-compose >/dev/null 2>&1; then
    docker-compose() {
      deploy_docker_run docker-compose "$@"
    }
  fi
fi
