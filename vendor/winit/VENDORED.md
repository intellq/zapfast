# winit (cópia do fork crmne/winit)

Cópia de [crmne/winit](https://github.com/crmne/winit) `apps-0.30` no commit
`fb8b24c` (o que o upstream do ZapFast fixa), ligada pelo `[patch.crates-io]`
do `Cargo.toml` do ZapFast. Os manifestos (`Cargo.toml` e `dpi/Cargo.toml`)
trocam os campos herdados do workspace pelos valores do commit; sem
`examples`, `tests` nem `docs`.

Mudança de comportamento (`src/platform_impl/linux/wayland/seat/dnd.rs`):

- Durante o arrasto de arquivos, o protocolo `wl_data_device` entrega a posição
  do ponteiro (`enter` e `motion`), mas o winit a descartava e a janela só sabia
  que havia arquivos sobre ela. Agora cada `enter`/`motion` do arrasto vira um
  `WindowEvent::CursorMoved` (posição lógica convertida pela escala da janela),
  e o fim do arrasto (cancelado ou solto) vira `CursorLeft`.
- O ZapFast usa isso para trocar de conversa ao segurar o arquivo sobre um
  contato da lista.
- Só o Wayland foi mudado. No X11 e no Windows o arrasto continua sem posição
  (o ZapFast então mostra o aviso de envio como antes).

Quando o `Cargo.toml` mudar o commit do fork, copiar de novo e reaplicar. Se
o fork passar a repassar a posição do arrasto, apagar esta cópia e voltar à
fonte git.
