deployment_baseline() {
  local install=$1 deploy=$2 compose=$3 file
  local files=("$install/.deploy-desktop/deployed.json" "$deploy/.env" "$deploy/.env.local")
  if [ -n "$compose" ]; then files+=("$deploy/$compose"); fi
  {
    for file in "${files[@]}"; do
      if [ -f "$file" ]; then
        sha256sum -- "$file" || return 1
      elif [ -e "$file" ] || [ -L "$file" ]; then
        return 1
      else
        printf 'missing:%s\n' "$file"
      fi
    done
  } | sha256sum | awk '{print $1}'
}

deployment_managed_bytes() {
  local install=$1 relative file ancestor size total=0
  shift
  for relative in "$@"; do
    case "$relative" in
      ""|.|./*|/*|..|../*|*/../*|*/..|*\\*) echo '受管路径无效' >&2; return 1 ;;
    esac
    if deployment_path_is_internal "$relative"; then
      [ "$relative" = ".deploy-desktop/deployed.json" ] || {
        echo '受管路径指向内部状态目录' >&2
        return 1
      }
    fi
    file="$install/$relative"
    ancestor=$file
    while [ "$ancestor" != / ] && [ -n "$ancestor" ]; do
      [ ! -L "$ancestor" ] || { echo '受管路径包含符号链接，无法安全部署' >&2; return 1; }
      ancestor=$(dirname -- "$ancestor")
    done
    if [ -e "$file" ]; then
      if [ -d "$file" ]; then
        [ -z "$(find "$file" -type l -print -quit)" ] || { echo '受管目录包含符号链接，无法安全部署' >&2; return 1; }
      fi
      size=$(du -sk -- "$file" | awk '{print $1}') || return 1
      total=$((total + size * 1024))
    fi
  done
  printf '%s\n' "$total"
}

deployment_path_is_internal() {
  local first=${1%%/*}
  local alias=${first%%:*}
  alias=$(printf '%s' "$alias" | tr '[:upper:]' '[:lower:]' | sed 's/[. ]*$//')
  [ "$alias" = ".deploy-desktop" ] && return 0
  local body=${alias#[.]} head tail
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
