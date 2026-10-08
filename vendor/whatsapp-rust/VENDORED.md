# whatsapp-rust (cópia do fork intellq)

Cópia do crate principal de
[oxidezap/whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) no commit
`f7468ae` (o mesmo que o `Cargo.toml` fixa), ligada pelo `[patch]` do
`Cargo.toml` do ZapFast. O manifesto foi gerado sem o workspace: os campos
herdados viraram os valores desse commit e os crates irmãos (`wacore`,
`wacore-binary`, `wacore-noise`, `waproto` e os de armazenamento, transporte e
HTTP) apontam para a fonte git do mesmo commit. Sem `benches`, `examples` nem
`tests`. `README.md` e `agent_docs/call_test_support.md` vieram junto porque o
código os inclui com `include_str!`.

Mudança de comportamento (recibos retidos, opcional):

- `Client::set_withhold_receipts(on)` (`src/client/accessors.rs`), guardado em
  `withhold_receipts` (`src/client.rs`, iniciado em `src/client/lifecycle.rs`).
- Ligado, `ack_received_message` (`src/message/dispatch.rs`) não manda recibo
  de entrega para mensagem de outra pessoa, nem ao vivo nem no lote do
  offline: manda só o ack de transporte (`<ack class="message">`, o mesmo que a
  biblioteca usa para resposta de bot), que tira a mensagem da fila do
  servidor. O recibo `type="inactive"`, que a biblioteca manda quando a
  presença não é "available", mostra dois tiques ao remetente do mesmo jeito
  (testado em 08/10/2026). Recibos para os aparelhos da própria conta
  (`peer_msg`, `sender`) continuam saindo.
- Ligado, `handle_decrypt_failure` (`src/message/retry.rs`) não envia o pedido
  de reenvio de uma mensagem que não decifrou: manda só o ack de transporte, e
  a mensagem fica ilegível.
- Desligado (o padrão), nada muda.

Quando o `Cargo.toml` mudar o commit do whatsapp-rust, copiar de novo o crate
do novo commit, trocar o `rev` dos crates irmãos no manifesto e reaplicar a
mudança. Se o upstream ganhar uma opção pública equivalente, apagar esta cópia
e o `[patch]`.
