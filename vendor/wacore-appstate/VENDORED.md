# wacore-appstate (cópia do fork intellq)

Cópia de `wacore/appstate` de
[oxidezap/whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) no commit
`f7468ae` (o mesmo que o `Cargo.toml` fixa), ligada pelo `[patch]` do
`Cargo.toml` do ZapFast. O manifesto troca os campos herdados do workspace
pelos valores desse commit e os crates irmãos (`wacore-binary`,
`wacore-libsignal`, `waproto`) pela fonte git do mesmo commit. Sem `benches`
nem `tests`.

Mudança de comportamento (`src/processor.rs`, `process_snapshot`):

- Numa conta (02/10/2026, instalação nova no CachyOS) o snapshot da coleção
  `regular_high`, que guarda silêncios de conversa e conversas favoritas, nunca
  validava: `snapshot MAC mismatch`, 2296 registros, todos com índice, a chave
  decifrava o próprio registro. A biblioteca recusava a coleção inteira, então
  nenhum silêncio chegava, embora o celular estivesse certo.
- Agora, só para `regular_high`, o snapshot cujo MAC agregado não bate é aceito
  com um aviso, e o estado fica marcado com `mac_mismatch_fatal`, o mesmo
  recurso que a biblioteca já usa para um patch cujo agregado divergiu (como
  o WA Web): a comparação agregada é pulada dali em diante. Cada registro
  continua autenticado um a um (MAC de conteúdo e de índice), e o `patchMac` de
  cada patch seguinte continua exigido. Perde-se só a prova de que o servidor
  não omitiu registros.
- Qualquer outra coleção continua recusando o snapshot que não bate.

Quando o `Cargo.toml` mudar o commit do whatsapp-rust, copiar de novo o crate
do novo commit, trocar o `rev` dos crates irmãos no manifesto e reaplicar a
mudança. Se o upstream corrigir o cálculo, apagar esta cópia e o `[patch]`.
