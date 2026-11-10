# 远程 Docker 统一入口。桌面端把它注入到每个 SSH 脚本，脚本内仍按普通
# `docker` / `docker-compose` 命令书写，由这里统一处理权限与 sudo 降级。
#
# 未安装 docker 时不定义函数，保留脚本里 `command -v docker` 的原有判断。
if command -v docker >/dev/null 2>&1; then
  DEPLOY_DOCKER_MODE=

  deploy_docker_pick() {
    if [ -n "${DEPLOY_DOCKER_MODE:-}" ]; then
      return 0
    fi
    if command docker info >/dev/null 2>&1; then
      DEPLOY_DOCKER_MODE=direct
      return 0
    fi
    if command -v sudo >/dev/null 2>&1 && command sudo -n docker info >/dev/null 2>&1; then
      DEPLOY_DOCKER_MODE=sudo
      return 0
    fi
    DEPLOY_DOCKER_MODE=direct
    return 1
  }

  deploy_docker_run() {
    if deploy_docker_pick; then
      DEPLOY_DOCKER_PICKED=0
    else
      DEPLOY_DOCKER_PICKED=$?
    fi
    if [ "${DEPLOY_DOCKER_MODE:-}" = "sudo" ]; then
      command sudo -n "$@"
      return $?
    fi
    if command "$@"; then
      return 0
    else
      DEPLOY_DOCKER_STATUS=$?
    fi
    # 只有确认是 socket 权限问题时才补充提示，守护进程未启动等错误保持原样。
    if [ "$DEPLOY_DOCKER_PICKED" -ne 0 ] \
      && command docker info 2>&1 | grep -qi 'permission denied'; then
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
