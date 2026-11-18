INSTALL=$1
while [ "$INSTALL" != "/" ] && [ "${INSTALL%/}" != "$INSTALL" ]; do
  INSTALL=${INSTALL%/}
done
shift
case "$INSTALL" in
  /*) ;;
  *) echo '备份需要有效的绝对部署目录' >&2; exit 2 ;;
esac
case "$INSTALL" in
  ''|*/../*|*/..|*/./*|*/.|*$'\n'*|*$'\r'*) echo '备份目录无效' >&2; exit 2 ;;
esac
# Validate every target before creating a backup, preserving the install-relative tree.
for target in "$@"; do
  case "$target" in
    "$INSTALL"/*) ;;
    *) echo '备份目标超出部署目录' >&2; exit 2 ;;
  esac
  case "$target" in
    */../*|*/..|*/./*|*/.|*/|*$'\n'*|*$'\r'*) echo '备份目标路径无效' >&2; exit 2 ;;
  esac
done
ensure_no_symlink_path() {
  local path=$1
  while [ -n "$path" ] && [ "$path" != "/" ]; do
    [ -L "$path" ] && { echo "路径包含符号链接：$path" >&2; exit 3; }
    path=$(dirname -- "$path")
  done
}
ensure_no_symlink_path "$INSTALL"
mkdir -p -- "$INSTALL/backup"
[ ! -L "$INSTALL/backup" ] && [ -d "$INSTALL/backup" ] || { echo '备份目录不能是符号链接' >&2; exit 3; }
BACKUP=$(mktemp -d "$INSTALL/backup/update-$(date +%Y%m%d-%H%M%S)-XXXXXXXX")
: > "$BACKUP/.updating"
for target in "$@"; do
  REL=${target#"$INSTALL"/}
  if [ -e "$target" ] || [ -L "$target" ]; then
    mkdir -p -- "$BACKUP/$(dirname -- "$REL")"
    cp -a -- "$target" "$BACKUP/$(dirname -- "$REL")/"
  fi
done
# 记录备份生成时已部署的版本，供部署备份列表展示与选择删除。
# 标记最后写入：既不会被同名产物覆盖，也只在备份完成后出现。
VERSION=
if [ -f "$INSTALL/.deploy-desktop/deployed.json" ]; then
  VERSION=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$INSTALL/.deploy-desktop/deployed.json" | head -n1)
  case "$VERSION" in
    *[!A-Za-z0-9._+-]*) VERSION= ;;
  esac
  if [ ${#VERSION} -gt 128 ]; then
    VERSION=
  fi
fi
META_DIR="$INSTALL/backup/.meta"
mkdir -p -- "$META_DIR"
[ ! -L "$META_DIR" ] && [ -d "$META_DIR" ] || { echo '备份版本目录不能是符号链接' >&2; exit 3; }
TMP_META=$(mktemp "$META_DIR/.tmp.XXXXXXXX")
printf '{"version":"%s"}\n' "$VERSION" > "$TMP_META"
TARGET_META="$META_DIR/${BACKUP##*/}.json"
if [ -d "$TARGET_META" ]; then
  rm -f -- "$TMP_META"
  echo '备份版本信息路径无效' >&2
  exit 3
fi
rm -f -- "$TARGET_META"
mv -f -- "$TMP_META" "$TARGET_META"
printf 'BACKUP_DIR=%s\n' "$BACKUP"
