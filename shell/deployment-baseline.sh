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
      ""|/*|..|../*|*/../*|*/..|*\\*) echo '受管路径无效' >&2; return 1 ;;
    esac
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
