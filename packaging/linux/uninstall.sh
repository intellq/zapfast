#!/usr/bin/env bash
# Removes what install.sh installed for the current user. The linked WhatsApp
# session, the message history and the settings stay unless --purge is given.
#
# Usage: ./uninstall.sh [--purge]
set -euo pipefail

purge=no
for argument in "$@"; do
  case "$argument" in
    --purge) purge=yes ;;
    -h | --help)
      sed -n '2,5p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Opção desconhecida: $argument" >&2
      exit 2
      ;;
  esac
done

if [[ $EUID -eq 0 ]]; then
  echo "Rode sem sudo, com o usuário que instalou o ZapFast." >&2
  exit 1
fi

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
if [[ ${XDG_CONFIG_HOME:-} == /* ]]; then
  config_dir=$XDG_CONFIG_HOME
else
  config_dir=$HOME/.config
fi

if pgrep -x zapfast >/dev/null 2>&1; then
  echo "Feche o ZapFast (Ctrl+Q) antes de desinstalar." >&2
  exit 1
fi

rm -f "$bin_dir/zapfast" "$bin_dir/.zapfast.new"
rm -rf "$bin_dir"/.zapfast-update-*
rm -f "$data_dir/icons/hicolor/scalable/apps/zapfast.svg"
rm -f "$data_dir/applications/zapfast.desktop" "$data_dir/applications/zapfast-links.desktop"
# The WhatsApp link default, only while it still names ZapFast's entry.
mimeapps=$config_dir/mimeapps.list
if grep -qs '^[[:space:]]*x-scheme-handler/whatsapp[[:space:]]*=[[:space:]]*zapfast-links.desktop' "$mimeapps"; then
  sed -i '/^[[:space:]]*x-scheme-handler\/whatsapp[[:space:]]*=[[:space:]]*zapfast-links\.desktop/d' "$mimeapps"
fi
rm -f "$config_dir/autostart/zapfast.desktop"
desktop_dir=$(xdg-user-dir DESKTOP 2>/dev/null || true)
if [[ -z $desktop_dir || $desktop_dir == "$HOME" ]]; then
  desktop_dir=$HOME/Desktop
fi
rm -f "$desktop_dir/zapfast.desktop" "$desktop_dir/.zapfast.desktop.new"
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$data_dir/applications" 2>/dev/null || true
fi

# The KWin rule that remembers the window's position on Plasma under Wayland
# (src/kwin_rule.rs): its group, and its id in [General]'s rules and count.
kwin_rule=5f0c2d9e-8b1a-4c7e-9a43-2e6d1f7b0a58
kwinrules=$config_dir/kwinrulesrc
if grep -qs "^\[$kwin_rule\]" "$kwinrules"; then
  # Two passes over the file: the first finds the rules left, the second
  # writes the file without ZapFast's.
  if RULE=$kwin_rule awk '
    FNR == 1 { general = rule = 0 }
    /^\[/ { general = ($0 == "[General]"); rule = ($0 == "[" ENVIRON["RULE"] "]") }
    NR == FNR {
      if (general && /^rules=/) {
        count = split(substr($0, 7), ids, ",")
        for (i = 1; i <= count; i++) {
          if (ids[i] == "" || ids[i] == ENVIRON["RULE"]) continue
          rules = rules (kept ? "," : "") ids[i]
          kept++
        }
      }
      next
    }
    rule { next }
    general && /^rules=/ { print "rules=" rules; next }
    general && /^count=/ { print "count=" kept + 0; next }
    { print }
  ' "$kwinrules" "$kwinrules" >"$kwinrules.zapfast-uninstall"; then
    mv -f "$kwinrules.zapfast-uninstall" "$kwinrules"
    if command -v dbus-send >/dev/null 2>&1; then
      dbus-send --session --type=method_call --dest=org.kde.KWin /KWin \
        org.kde.KWin.reconfigure >/dev/null 2>&1 || true
    fi
  else
    rm -f "$kwinrules.zapfast-uninstall"
  fi
fi

# The PATH lines install.sh added.
for file in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.zshrc"; do
  if grep -qs "# zapfast-path" "$file"; then
    sed -i '/# zapfast-path$/d' "$file"
  fi
done
rm -f "$config_dir/fish/conf.d/zapfast.fish"

if [[ $purge == yes ]]; then
  if [[ ${XDG_STATE_HOME:-} == /* ]]; then
    state_dir=$XDG_STATE_HOME
  else
    state_dir=$HOME/.local/state
  fi
  if [[ ${XDG_CACHE_HOME:-} == /* ]]; then
    cache_dir=$XDG_CACHE_HOME
  else
    cache_dir=$HOME/.cache
  fi
  rm -rf "$state_dir/zapfast" "$config_dir/zapfast" "$data_dir/zapfast" "$cache_dir/zapfast"
  echo "ZapFast removido, com a sessão, o histórico e as configurações."
else
  echo "ZapFast removido. A sessão, o histórico e as configurações continuam em"
  echo "~/.local/state/zapfast e ~/.config/zapfast (apague com --purge)."
fi
