#!/usr/bin/env bash
# 将 Tauri 生成的 AppImage 重新打包为可在 glibc 2.28 系统（如方德 4.0）运行的版本。
# Tauri 2 依赖 WebKitGTK 4.1，而这类系统同时缺少新版 glibc、HarfBuzz、FreeType
# 和 Wayland/GBM 符号，因此这里内置构建环境中的运行库并用内置 loader 启动。
# 用法：package-linux-legacy.sh <input.AppImage> <output.AppImage>
set -euo pipefail

input_appimage="${1:?input AppImage required}"
output_appimage="${2:?output AppImage required}"

command -v mksquashfs >/dev/null || {
  echo "mksquashfs is required (install squashfs-tools)" >&2
  exit 1
}

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

cp "$input_appimage" "$work_dir/app.AppImage"
chmod +x "$work_dir/app.AppImage"
(
  cd "$work_dir"
  ./app.AppImage --appimage-extract >/dev/null
)

appdir="$work_dir/squashfs-root"
runtime_offset="$(cd "$work_dir" && ./app.AppImage --appimage-offset)"
head -c "$runtime_offset" "$work_dir/app.AppImage" > "$work_dir/runtime"

runtime_dir="$appdir/usr/lib/deploycabin-runtime"
mkdir -p "$runtime_dir"

copy_lib() {
  local name="$1"
  local dest="$2"
  local dir
  for dir in /lib/x86_64-linux-gnu /usr/lib/x86_64-linux-gnu; do
    if [ -e "$dir/$name" ]; then
      cp -L "$dir/$name" "$dest/$name"
      return 0
    fi
  done
  echo "required library not found: $name" >&2
  return 1
}

# glibc 运行时随包携带，避免依赖目标系统的 2.28 版本。
for lib in \
  ld-linux-x86-64.so.2 \
  libc.so.6 \
  libm.so.6 \
  libpthread.so.0 \
  libdl.so.2 \
  librt.so.1 \
  libresolv.so.2 \
  libutil.so.1 \
  libanl.so.1 \
  libnss_files.so.2 \
  libnss_dns.so.2 \
  libnss_compat.so.2 \
  libnss_hesiod.so.2 \
  libgcc_s.so.1 \
  libstdc++.so.6; do
  copy_lib "$lib" "$runtime_dir"
done

# 目标系统的字体与图形库版本过旧，需随包携带构建环境中的版本。
for lib in \
  libharfbuzz.so.0 \
  libfribidi.so.0 \
  libfreetype.so.6 \
  libwayland-client.so.0 \
  libwayland-cursor.so.0 \
  libwayland-server.so.0 \
  libgbm.so.1 \
  libdrm.so.2; do
  copy_lib "$lib" "$appdir/usr/lib"
done

cp "$appdir/AppRun" "$appdir/AppRun.orig"
cat > "$appdir/AppRun" <<'EOF'
#! /usr/bin/env bash
set -e
this_dir="$(readlink -f "$(dirname "$0")")"
source "$this_dir"/apprun-hooks/"linuxdeploy-plugin-gtk.sh"
export APPDIR="$this_dir"
cd "$this_dir/usr"
loader="$this_dir/usr/lib/deploycabin-runtime/ld-linux-x86-64.so.2"
if [ -x "$loader" ]; then
  exec "$loader" --library-path "$this_dir/usr/lib/deploycabin-runtime:$this_dir/usr/lib" \
    "$this_dir/usr/bin/deploy-desktop" "$@"
fi
exec "$this_dir/AppRun.wrapped" "$@"
EOF
chmod +x "$appdir/AppRun"

# WebKit 的辅助进程由 libwebkit 直接 exec，需改为通过内置 loader 启动。
helper_dir="$appdir/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1"
for helper in WebKitWebProcess WebKitNetworkProcess; do
  binary="$helper_dir/$helper"
  [ -f "$binary" ] || continue
  mv "$binary" "$binary.real"
  cat > "$binary" <<'EOF'
#!/bin/sh
self="$(readlink -f "$0")"
dir="$(dirname "$self")"
root="$(readlink -f "$dir/../../../..")"
exec "$root/usr/lib/deploycabin-runtime/ld-linux-x86-64.so.2" \
  --library-path "$root/usr/lib/deploycabin-runtime:$root/usr/lib" \
  "$dir/$(basename "$0").real" "$@"
EOF
  chmod +x "$binary"
done

chmod -R a+rX "$appdir"
chmod a+rx "$appdir/AppRun"

list_output="$("$runtime_dir/ld-linux-x86-64.so.2" \
  --library-path "$runtime_dir:$appdir/usr/lib" \
  --list "$appdir/usr/bin/deploy-desktop" 2>&1 || true)"
if grep -qE "not found|error while loading shared libraries" <<<"$list_output"; then
  echo "$list_output" >&2
  echo "unresolved libraries in the repackaged AppImage" >&2
  exit 1
fi

mksquashfs "$appdir" "$work_dir/filesystem.squashfs" \
  -comp gzip -b 131072 -noappend -no-progress -no-xattrs -all-root >/dev/null

cat "$work_dir/runtime" "$work_dir/filesystem.squashfs" > "$output_appimage"
chmod +x "$output_appimage"
echo "created $output_appimage"
