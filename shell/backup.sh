INSTALL=${1%/}
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
mkdir -p -- "$INSTALL/backup"
BACKUP=$(mktemp -d "$INSTALL/backup/update-$(date +%Y%m%d-%H%M%S)-XXXXXXXX")
for target in "$@"; do
  REL=${target#"$INSTALL"/}
  if [ -e "$target" ] || [ -L "$target" ]; then
    mkdir -p -- "$BACKUP/$(dirname -- "$REL")"
    cp -a -- "$target" "$BACKUP/$(dirname -- "$REL")/"
  fi
done
printf 'BACKUP_DIR=%s\n' "$BACKUP"
