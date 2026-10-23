# Shared by the desktop and MCP adapters. Arguments are data, never evaluated.
ACTION=$1
TARGET=$2
STAGE=${3:-}
case "$TARGET" in
  /*) ;;
  *) echo '目标路径必须是绝对路径' >&2; exit 2 ;;
esac
case "$TARGET" in
  /|*/|*/../*|*/..|*/./*|*/.|*$'\n'*|*$'\r'*) echo '目标路径无效，请检查部署目录' >&2; exit 2 ;;
esac
if [ "$ACTION" = prepare ]; then
  mkdir -p -- "$(dirname -- "$TARGET")"
  mktemp -d "$TARGET.deploy-stage.XXXXXXXX"
  exit 0
fi
case "$STAGE" in
  "$TARGET".deploy-stage.*) ;;
  *) echo '暂存目录与目标不匹配，已停止操作' >&2; exit 2 ;;
esac
SUFFIX=${STAGE#"$TARGET".deploy-stage.}
case "$SUFFIX" in
  ''|*[!a-zA-Z0-9]*) echo '暂存目录名称无效' >&2; exit 2 ;;
esac
[ ${#SUFFIX} -eq 8 ] || { echo '暂存目录名称无效' >&2; exit 2; }
[ -d "$STAGE" ] && [ ! -L "$STAGE" ] || { echo '暂存目录不可用' >&2; exit 2; }
case "$ACTION" in
  receive-file) cat > "$STAGE/payload" ;;
  receive-dir) mkdir -- "$STAGE/payload"; tar -xzf - -C "$STAGE/payload" ;;
  cleanup)
    [ ! -d "$TARGET.deploy-lock" ] || { echo '目标仍在替换中，暂存内容已保留' >&2; exit 3; }
    # A failed rollback is recoverable only while the previous copy survives.
    if [ -e "$STAGE/previous" ] || [ -L "$STAGE/previous" ]; then
      echo "旧版本仍保留在 $STAGE/previous，请检查目标目录后恢复" >&2
      exit 5
    fi
    rm -rf -- "$STAGE"
    ;;
  commit)
    [ -e "$STAGE/payload" ] || { echo '暂存内容缺失，原部署未修改' >&2; exit 2; }
    LOCK="$TARGET.deploy-lock"
    mkdir -- "$LOCK" 2>/dev/null || { echo '该目标正在被另一个任务替换，请稍后重试' >&2; exit 3; }
    finish() {
      code=$?
      trap - EXIT HUP INT TERM
      if [ -e "$STAGE/previous" ] || [ -L "$STAGE/previous" ]; then
        if [ ! -e "$TARGET" ] && [ ! -L "$TARGET" ]; then
          if ! mv -- "$STAGE/previous" "$TARGET"; then
            echo "恢复旧版本失败，旧内容保留在 $STAGE/previous" >&2
            code=5
          fi
        fi
      fi
      rmdir -- "$LOCK" || code=5
      exit "$code"
    }
    trap finish EXIT
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
    if [ -e "$TARGET" ] || [ -L "$TARGET" ]; then mv -- "$TARGET" "$STAGE/previous"; fi
    mv -- "$STAGE/payload" "$TARGET"
    rm -rf -- "$STAGE/previous"
    ;;
  *) echo '不支持的文件传输操作' >&2; exit 2 ;;
esac
