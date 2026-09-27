# fastframe-update (cópia do fork intellq)

Cópia de `crates/fastframe-update` de
[crmne/fastframe](https://github.com/crmne/fastframe) na tag `v0.1.7`
(commit `9cb2830`), usada pelo ZapFast do fork no lugar da dependência git.

Única mudança de comportamento (`src/detect.rs`, funções `in_user_bin` e
`is_the_app`): no Linux, um executável chamado `zapfast` na pasta de
executáveis do usuário (`$XDG_BIN_HOME` ou `~/.local/bin`), fora de pacotes do
sistema, atualiza-se sozinho sem o arquivo `zapfast-portable.txt`. É onde o
`install.sh` do pacote Linux do fork o instala. O manifesto troca os campos
herdados do workspace pelos valores da v0.1.7.

Quando o upstream mudar a tag do fastframe no `Cargo.toml`, copiar de novo o
crate da nova tag e reaplicar a mudança acima.
