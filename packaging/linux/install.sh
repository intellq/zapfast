#!/usr/bin/env bash
# Installs ZapFast from the Linux release download for the current user, with
# no root: the binary in the user's bin folder, the icon, a launcher entry in
# the application menu and a shortcut on the desktop. ZapFast updates itself
# there (vendor/fastframe-update/VENDORED.md). Running it again installs the
# version it came with over the current one.
#
# Usage: ./install.sh [--no-desktop-shortcut]
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

desktop_shortcut=yes
for argument in "$@"; do
  case "$argument" in
    --no-desktop-shortcut) desktop_shortcut=no ;;
    -h | --help)
      sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Opção desconhecida: $argument" >&2
      exit 2
      ;;
  esac
done

if [[ $EUID -eq 0 ]]; then
  echo "Rode sem sudo: o ZapFast é instalado só para o seu usuário, em ~/.local." >&2
  exit 1
fi
if [[ ! -x "$here/zapfast" ]]; then
  echo "O executável zapfast não está ao lado deste script." >&2
  exit 1
fi

# The same folder the updater treats as the user's own bin folder.
if [[ ${XDG_BIN_HOME:-} == /* ]]; then
  bin_dir=$XDG_BIN_HOME
else
  bin_dir=$HOME/.local/bin
fi
if [[ ${XDG_DATA_HOME:-} == /* ]]; then
  data_dir=$XDG_DATA_HOME
else
  data_dir=$HOME/.local/share
fi
apps_dir=$data_dir/applications
icons_dir=$data_dir/icons/hicolor/scalable/apps
installed=$bin_dir/zapfast

# Replaced in one rename, so a ZapFast that is open keeps running and the
# next start is the new version.
mkdir -p "$bin_dir"
install -m 755 "$here/zapfast" "$bin_dir/.zapfast.new"
mv -f "$bin_dir/.zapfast.new" "$installed"
install -Dm644 "$here/zapfast.svg" "$icons_dir/zapfast.svg"
if command -v gtk-update-icon-cache >/dev/null 2>&1 && [[ -f "$data_dir/icons/hicolor/index.theme" ]]; then
  gtk-update-icon-cache -q -t "$data_dir/icons/hicolor" 2>/dev/null || true
fi

# The launcher names the binary by its full path, so it works before the
# session has the bin folder on PATH. Quoted and escaped as a Desktop Entry's
# Exec key wants (the rules of src/autostart.rs and install-user.sh).
exec_line() {
  EXEC_PATH="$installed" awk '
    function quote(path,   out, i, c) {
      out = "\""
      for (i = 1; i <= length(path); i++) {
        c = substr(path, i, 1)
        if (c == "\"" || c == "`" || c == "$" || c == "\\") out = out "\\"
        if (c == "%") out = out "%"
        out = out c
      }
      return out "\""
    }
    /^Exec=/ { print "Exec=" quote(ENVIRON["EXEC_PATH"]); next }
    { print }
  ' "$here/zapfast.desktop"
}
mkdir -p "$apps_dir"
exec_line > "$apps_dir/zapfast.desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$apps_dir" 2>/dev/null || true
fi

# The desktop folder has a translated name ("Área de Trabalho"), which
# xdg-user-dir knows.
if [[ $desktop_shortcut == yes ]]; then
  desktop_dir=$(xdg-user-dir DESKTOP 2>/dev/null || true)
  if [[ -z $desktop_dir || $desktop_dir == "$HOME" ]]; then
    desktop_dir=$HOME/Desktop
  fi
  if [[ -d $desktop_dir ]]; then
    exec_line > "$desktop_dir/zapfast.desktop"
    # KDE runs a desktop file only when it is executable; GNOME asks for it
    # to be marked trusted.
    chmod 755 "$desktop_dir/zapfast.desktop"
    if command -v gio >/dev/null 2>&1; then
      gio set "$desktop_dir/zapfast.desktop" metadata::trusted true 2>/dev/null || true
    fi
  fi
fi

# Put the bin folder on PATH for shells and new sessions, once. Every line
# added ends in "# zapfast-path", which uninstall.sh removes.
path_line="case \":\$PATH:\" in *\":$bin_dir:\"*) ;; *) export PATH=\"$bin_dir:\$PATH\" ;; esac # zapfast-path"
added_path=no
add_path_to() {
  local file=$1
  if ! grep -qs "# zapfast-path" "$file"; then
    printf '\n%s\n' "$path_line" >> "$file"
    added_path=yes
  fi
}
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *)
    add_path_to "$HOME/.profile"
    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
      [[ -f $rc ]] && add_path_to "$rc"
    done
    if [[ -d $HOME/.config/fish ]]; then
      mkdir -p "$HOME/.config/fish/conf.d"
      printf 'fish_add_path -g %q # zapfast-path\n' "$bin_dir" > "$HOME/.config/fish/conf.d/zapfast.fish"
      added_path=yes
    fi
    ;;
esac

echo "ZapFast instalado em $installed"
echo "Atalho no menu de aplicativos: $apps_dir/zapfast.desktop"
if [[ $desktop_shortcut == yes && -n ${desktop_dir:-} && -d ${desktop_dir:-/nonexistent} ]]; then
  echo "Atalho na área de trabalho: $desktop_dir/zapfast.desktop"
fi
if [[ $added_path == yes ]]; then
  echo "$bin_dir foi adicionado ao PATH; vale nos terminais novos e na próxima sessão."
fi
# Another zapfast earlier on PATH would win in a terminal.
other=$(PATH="$bin_dir:$PATH" type -ap zapfast 2>/dev/null | grep -vxF "$installed" | head -n 1 || true)
if [[ -n $other ]]; then
  echo "Atenção: há outro zapfast em $other. Remova-o para não abrir a cópia errada." >&2
fi
if pgrep -x zapfast >/dev/null 2>&1; then
  echo "O ZapFast está aberto: feche-o (Ctrl+Q) e abra de novo para usar esta versão."
fi
