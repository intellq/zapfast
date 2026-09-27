# fastframe-update (cópia do fork intellq)

Cópia de `crates/fastframe-update` de
[crmne/fastframe](https://github.com/crmne/fastframe) na tag `v0.1.7`
(commit `9cb2830`), usada pelo ZapFast do fork no lugar da dependência git.

Mudanças de comportamento:

- `src/detect.rs`, funções `in_user_bin` e `is_the_app`: no Linux, um executável chamado `zapfast` na pasta de
executáveis do usuário (`$XDG_BIN_HOME` ou `~/.local/bin`), fora de pacotes do
sistema, atualiza-se sozinho sem o arquivo `zapfast-portable.txt`. É onde o
`install.sh` do pacote Linux do fork o instala.
- Limpeza das pastas de preparo `.zapfast-update-<16 hex>`, que o original
  nunca apaga (cada uma guarda `helper` e `previous`, cerca de 160 MB):
  `Receipt::clean_up` (`src/startup.rs`), chamado pelo app depois do
  `acknowledge`, espera o `result.txt` do helper por até dois minutos e apaga
  a pasta se a atualização deu certo; `stage::sweep` (`src/stage.rs`), chamado
  pelo `intercept` a cada abertura, apaga ao lado do executável as pastas de
  atualizações bem-sucedidas e, uma semana depois da última alteração, as
  demais (falhas, downloads não instalados). `stage::UPDATED` passou a ser o
  começo do `result.txt` de sucesso escrito em `src/helper.rs`.

O manifesto troca os campos herdados do workspace pelos valores da v0.1.7.

Quando o upstream mudar a tag do fastframe no `Cargo.toml`, copiar de novo o
crate da nova tag e reaplicar as mudanças acima.
