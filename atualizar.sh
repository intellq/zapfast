#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
cd -- "$repo_dir"

if [[ $(git rev-parse --show-toplevel) != "$repo_dir" ]]; then
    printf 'Erro: este script precisa estar na raiz do repositório ZapFast.\n' >&2
    exit 1
fi

branch=$(git symbolic-ref --quiet --short HEAD) || {
    printf 'Erro: selecione uma branch antes de atualizar.\n' >&2
    exit 1
}
upstream=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}') || {
    printf 'Erro: a branch %s não acompanha uma branch remota.\n' "$branch" >&2
    exit 1
}
remote=$(git config --get "branch.$branch.remote")

printf 'Verificando atualizações em %s...\n' "$upstream"
git fetch "$remote"

local_commit=$(git rev-parse HEAD)
remote_commit=$(git rev-parse "$upstream")

if [[ $local_commit == "$remote_commit" ]]; then
    printf 'ZapFast já está atualizado.\n'
    exit 0
fi

if git merge-base --is-ancestor "$remote_commit" "$local_commit"; then
    printf 'A branch local já contém os commits de %s.\n' "$upstream"
    exit 0
fi

if ! git merge-base --is-ancestor "$local_commit" "$remote_commit"; then
    printf 'Erro: a branch local divergiu de %s. Resolva o histórico antes de atualizar.\n' "$upstream" >&2
    exit 1
fi

if [[ -n $(git status --porcelain --untracked-files=no) ]]; then
    printf 'Erro: há alterações locais rastreadas. Salve-as antes de atualizar.\n' >&2
    exit 1
fi

git pull --ff-only
printf 'Repositório atualizado.\n'

# A mesma pasta que o install.sh da release e o atualizador embutido usam.
if [[ ${XDG_BIN_HOME:-} == /* ]]; then
    bin_dir=$XDG_BIN_HOME
else
    bin_dir=$HOME/.local/bin
fi

if ! read -r -p "Compilar e instalar o novo binário em $bin_dir? [s/N] " answer; then
    printf '\nCompilação e instalação ignoradas.\n'
    exit 0
fi

case ${answer,,} in
    s|sim) ;;
    *) printf 'Compilação e instalação ignoradas.\n'; exit 0 ;;
esac

jobs=$(nproc)
printf 'Compilando com %s threads lógicas.\n' "$jobs"
CARGO_PROFILE_RELEASE_CODEGEN_UNITS="$jobs" cargo build --release --locked --bin zapfast -j "$jobs"
# Troca por renomeação: um ZapFast aberto continua rodando e a próxima
# abertura já usa o novo binário.
mkdir -p "$bin_dir"
install -m 755 "$repo_dir/target/release/zapfast" "$bin_dir/.zapfast.new"
mv -f "$bin_dir/.zapfast.new" "$bin_dir/zapfast"
printf 'ZapFast compilado e instalado em %s/zapfast.\n' "$bin_dir"
