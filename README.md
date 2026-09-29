# ZapFast

**WhatsApp, native and fast.** ZapFast is a WhatsApp client written in Rust
with [egui](https://github.com/emilk/egui). It uses
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) for the WhatsApp Web
protocol. It runs on Linux, macOS, and Windows, links to your phone as a
companion device, and has no browser engine. In our Linux test, it opens in
under a second and uses about 200 MB of idle RAM, compared with 1.13 GB for
WhatsApp Web and its Chromium processes. [See the measurements](https://zapfast.rocks/benchmarks/).

**Want Spotify just as fast and native?** [Spotifast](https://spotifast.rocks)
is ZapFast's sibling: the same native interface, for Spotify. Both are built
on [fastframe](https://github.com/crmne/fastframe), the shared foundation for
native Rust apps built with egui.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshot-light.png">
  <img src="docs/screenshot.png" alt="ZapFast showing a conversation with an attachment, voice messages, reactions, a quoted reply, and a link preview">
</picture>

See **[zapfast.rocks](https://zapfast.rocks)** for downloads and guides.

## Alterações deste fork (`intellq`)

Este fork acompanha o [projeto original](https://github.com/crmne/zapfast)
até o commit [`f0a7706`](https://github.com/crmne/zapfast/commit/f0a7706)
(versão 0.17.0 e ajustes seguintes).
Além das novidades do upstream, esta branch inclui as seguintes mudanças:

### Instalar este fork

As versões do fork ficam em
[Releases](https://github.com/intellq/zapfast/releases/latest), para Linux e
Windows x86_64. Os números seguem o upstream: as versões 0.17.1 a 0.17.3 são o
fork sobre o upstream 0.17.0, e depois do upstream 0.17.1 vem a 0.17.101. Os pacotes são
assinados com a chave do fork, e o ZapFast instalado por eles se atualiza
sozinho a partir das Releases do fork (não das do upstream).

- **Linux:** baixe `zapfast-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`, extraia e
  rode `./install.sh`, sem `sudo`. O programa vai para `~/.local/bin/zapfast`,
  com atalho no menu e na área de trabalho, e `~/.local/bin` entra no `PATH`
  se ainda não estiver. `./uninstall.sh` remove (com `--purge`, também a
  sessão, o histórico e as configurações).
- **Windows:** o instalador `…-x86_64-pc-windows-msvc-setup.exe` (sem
  administrador; atalho na área de trabalho marcado por padrão) ou o `.zip`
  portátil, que se atualiza na própria pasta.
- **Máquina virtual Windows sem aceleração 3D:** extraia
  `zapfast-vX.Y.Z-windows-opengl-vm.zip` dentro da pasta do ZapFast (OpenGL
  por software do Mesa).

### Mudanças

- **Interface inteiramente em português brasileiro.** Com o idioma pt-BR, todo
  texto visível ao usuário aparece em português: menus e dicas, diálogos,
  avisos, mensagens de erro vindas do backend (conexão, proxy, microfone,
  chaveiro do sistema, figurinhas, enquetes), seletores de arquivo, nomes de
  temas e cores de fundo, os itens da bandeja do sistema, o menu do macOS e o
  botão **Abrir** das notificações. O atalho do menu de aplicativos e o item
  de inicialização automática também têm nome e descrição em pt-BR. Ao trocar
  o idioma em **Configurações**, os itens da bandeja acompanham na hora, sem
  reiniciar. Termos técnicos (GIF, proxy, TLS, WebSocket, FFmpeg, VAAPI)
  seguem em inglês. Ficam em inglês de propósito apenas o modo de demonstração,
  a ajuda da linha de comando e os logs.
- **Atualizações pelas Releases do fork.** O aviso de nova versão e o
  download automático consultam as
  [Releases deste fork](https://github.com/intellq/zapfast/releases), com
  pacotes assinados pela chave do fork, e não as do projeto original.
  **Verificar atualizações** e **Baixar atualizações automaticamente** vêm
  ligadas em instalações novas; reiniciar para aplicar continua sendo um
  clique seu. Instalações antigas mantêm o que já estava salvo nas
  configurações. No Linux, a atualização sozinha vale para o binário em
  `~/.local/bin` (o do `install.sh`); o código-fonte continua podendo ser
  atualizado pelo [`atualizar.sh`](atualizar.sh). A pasta temporária que a
  atualização cria ao lado do programa (`.zapfast-update-…`, com cerca de
  160 MB) é apagada sozinha logo depois de uma atualização bem-sucedida; a de
  uma atualização que falhou fica uma semana, para consulta do registro.
- **Links clicáveis desde o primeiro clique.** Em conversas longas, a
  virtualização do histórico mudava o identificador dos elementos entre o
  pressionamento e a soltura do mouse. O ponteiro indicava um link, mas o
  clique não era reconhecido; alguns cliques também pareciam estender uma
  seleção de texto. Cada linha agora mantém um identificador estável, mesmo
  quando mensagens antigas entram na tela. Links em mensagens com prévia
  abrem no navegador padrão sem depender de clicar no cartão da prévia.
- **Divisor da lista de conversas livre nos dois sentidos.** A área de
  arrasto da borda entre a lista e a conversa invade alguns pontos do
  histórico, e um clique ali era tratado como início de seleção de texto, que
  prende o ponteiro dentro da conversa. A lista só alargava. Agora o clique na
  borda redimensiona a lista para a esquerda ou para a direita, sem editar o
  `sidebar_width` no `settings.json`.
- **Vídeos com áudio incompatível.** Se a imagem do vídeo pode ser reproduzida
  internamente, mas a faixa sonora não pode ser decodificada (por exemplo,
  HE-AACv2), o vídeo é aberto no reprodutor padrão do sistema. Antes ele podia
  tocar sem som e nem aparecia no mixer de áudio. Vídeos com áudio suportado
  continuam tocando dentro da conversa; vídeos realmente sem faixa de áudio
  continuam no player interno.
- **Vídeos sem miniatura continuam como vídeo.** Mensagens de vídeo que chegam
  sem imagem de prévia (comum no histórico trazido do celular numa conta
  recém-vinculada) apareciam como um arquivo para baixar e abrir fora do
  programa. Agora aparecem no quadro de vídeo, sobre fundo escuro, com o botão
  de reproduzir; o clique baixa e toca na própria conversa.
- **Vídeo expandido sem cópia no histórico.** Ao expandir um vídeo, a
  mensagem no histórico volta a mostrar a miniatura e o botão de reproduzir,
  em vez de exibir a mesma reprodução por trás da tela escurecida. Ao
  recolher pelo botão do canto, o vídeo continua de onde estava, dentro da
  mensagem; ao fechar com Esc, a reprodução para e a mensagem volta ao
  estado inicial.
- **Downloads configuráveis por tipo e tamanho.** Cinco opções independentes
  cobrem todos os arquivos das mensagens: áudios (inclusive mensagens de voz),
  vídeos (inclusive GIFs e vídeos redondos), imagens (inclusive figurinhas
  estáticas e cartões interativos), figurinhas animadas e documentos
  (qualquer arquivo enviado como documento). A antiga opção de baixar todos
  os arquivos foi retirada; quem a tinha ligada fica com as cinco ligadas.
  Numa instalação nova, áudios, imagens e figurinhas animadas vêm ligados, e
  vídeos e documentos, desligados.
  Um slider define o limite de 1 a 64 MiB para downloads automáticos e
  manuais. O limite é verificado antes e durante a transferência.
- **Configurações legíveis em janelas estreitas.** Títulos e descrições podem
  ocupar mais linhas, sem serem cortados com reticências. Cada controle
  reserva apenas a largura necessária; Aparência e Arquivos mantêm as linhas
  alinhadas sem grandes espaços verticais em zoom de 100% ou 130%. Os botões
  **Abrir pasta de temas** e **Como criar um tema** ficam lado a lado, abaixo
  do menu de temas.
- **Mensagens recebidas usam mais largura.** O texto recebido quebra perto da
  borda direita disponível na conversa, respeitando o alinhamento dos balões
  próprios. A estimativa das linhas fora da tela usa a mesma largura, para
  reduzir saltos de rolagem ao navegar pelo histórico.
- **Reações mais limpas.** O emoji de uma reação anexada à mensagem aparece
  sem o círculo ciano ao redor. A borda do menu usado para escolher a reação
  foi preservada. Na grade de emojis da reação não há mais contorno de
  seleção; no seletor de emojis comum, o contorno só aparece depois de clicar
  em um emoji ou de mover a seleção pelas setas, em vez de já vir marcado no
  primeiro item dos recentes. Enquanto você ainda não reagiu a nada, a linha
  **Mais usados** da janela de reação traz os seis emojis padrão do WhatsApp:
  👍 ❤️ 😂 😮 😢 🙏. As reações exibidas embaixo da mensagem ficam mais
  próximas umas das outras.
- **Ctrl+End na caixa de digitação.** Com texto na caixa, Ctrl+End leva o
  cursor ao fim da última linha, como Ctrl+Home leva ao início da primeira
  (e Ctrl+Shift+End seleciona até o fim). Antes o atalho de ir para as
  mensagens mais recentes tomava a tecla e nada acontecia no texto; com a
  caixa vazia ou fora dela, Ctrl+End continua indo para as mensagens mais
  recentes, mas deixou de aparecer na lista de atalhos de **Configurações**.
- **Tons de pele nos emojis de mãos.** O seletor oferece as variantes de tom
  para emojis compatíveis, como joinha, aperto de mãos e mãos juntas. No menu
  de reação, o seletor tem a mesma largura do seletor normal e dispensa a
  barra de pesquisa. A área de recentes mostra até três linhas, só as que
  têm emojis, sem linhas em branco, e, ao escolher outro tom do mesmo emoji,
  substitui a variante anterior em vez de duplicá-la. Nos recentes, o emoji
  já está no tom escolhido: ele aparece sem a marca de variantes (um pequeno
  triângulo sólido no canto inferior direito, como no celular) e um clique o
  insere (ou reage com ele) imediatamente. Ao rolar os emojis, o ícone da
  categoria que chega ao topo fica destacado na linha de categorias abaixo. Enquanto o seletor fica aberto, a
  ordem dos recentes não muda ao escolher um deles; a nova ordem aparece na
  próxima vez que ele for aberto.
  Cada caixa tem a sua lista: reagir atualiza só os mais usados do seletor
  de reações (em ordem de frequência), e inserir no texto só os recentes do
  seletor normal (do mais novo para o mais antigo).
- **Atualização local assistida.** O script [`atualizar.sh`](atualizar.sh), na
  raiz do repositório, consulta a branch remota acompanhada pela branch
  atual. Se não houver atualização, apenas informa isso; se houver avanço
  linear, executa `git pull --ff-only` e oferece compilar em modo release e
  instalar o binário em `~/.local/bin/zapfast` (sem `sudo`; troca atômica,
  como o instalador da release). A compilação usa todas as
  threads lógicas detectadas por `nproc`, tanto nos jobs do Cargo quanto nas
  unidades de geração de código do perfil release.
- **Exclusão confirmada e sincronizada.** O submenu **Apagar para mim** abre uma
  confirmação com **OK** e **Cancelar** em português. A caixa **Apagar também
  no celular** vem marcada: nesse caso o ZapFast envia a exclusão "para mim"
  com a chave e o horário original da mensagem ao WhatsApp, e só apaga sua
  cópia local após a operação ser aceita. O envio com horário foi confirmado
  em teste manual no Android. Se a caixa for desmarcada,
  só o arquivo local é alterado; **Cancelar** não apaga nada. Isso não é
  **Apagar para todos**: outras pessoas conservam suas cópias.
- **Encaminhamento com escolha de destinatários.** A lista de conversas agora
  mostra caixas de seleção e só envia após o clique em **Enviar**; também há
  **Cancelar**. A janela usa cerca de 80% da altura disponível. Antes de
  abri-la, o ZapFast consulta os metadados das mensagens: é possível escolher
  até cinco conversas, no máximo um grupo para mensagem já encaminhada, ou
  somente uma conversa quando alguma mensagem foi encaminhada muitas vezes.
  O backend revalida os limites e envia o lote em ordem.
- **Troca de texto por emoji opcional.** Em **Configurações › Conversas**, a
  opção **Trocar textos por emojis** vem desmarcada: digitar `:` seguido de
  letras (como `:P` ou `:D`) não abre sugestões, e Enter envia o texto como
  foi digitado. Marcada, volta o comportamento anterior: `:nome` sugere
  emojis e Enter ou Tab troca o texto pelo emoji escolhido.
- **Emojis no estilo do WhatsApp (opcional).** Em **Configurações ›
  Aparência**, a opção **Emojis do WhatsApp** troca o Noto Color Emoji pela
  fonte de emojis do WhatsApp, sem reiniciar. A arte é propriedade da
  WhatsApp/Meta e não tem licença de redistribuição, então este fork não
  inclui nem baixa a fonte: você instala o arquivo. Uma versão compatível
  (bitmaps CBDT, com tons de pele, bandeiras e sequências ZWJ) é a do projeto
  [whatsapp-emoji-linux](https://github.com/dmlls/whatsapp-emoji-linux), que
  declara uso apenas não comercial e educacional. Coloque `WhatsAppEmoji.ttf`
  na pasta de fontes de emoji do ZapFast (`~/.config/zapfast/fonts` no Linux;
  o botão **Abrir pasta** aparece enquanto a fonte não é encontrada) para que
  só o ZapFast use esses emojis. O pacote AUR `ttf-whatsapp-emoji` também é
  reconhecido, mas instala uma regra do fontconfig que pode trocar os emojis
  do desktop inteiro. No Windows, vale também uma pasta `fonts` ao lado do
  `zapfast.exe`. Se a fonte já estiver num desses lugares no primeiro uso do
  ZapFast (ainda sem `settings.json`), a opção vem ligada. Sem a fonte, a
  opção fica desabilitada.
- **Prévia de links no envio.** Ao digitar ou colar um link, o ZapFast busca
  na página o título, a descrição e a imagem (as tags Open Graph, ou o título
  e a descrição comuns) e mostra a prévia acima da caixa de digitação, como o
  celular e o cliente oficial. Ela vai junto com a mensagem: a miniatura
  dentro da mensagem e, para imagens largas, uma versão maior enviada aos
  servidores do WhatsApp, que os celulares mostram ocupando a largura do
  balão. O **X** do cartão envia o texto sem a prévia; se a mensagem sair
  antes de a prévia carregar, ela vai sem prévia. Vale para o primeiro link
  do texto, não para legendas de anexos nem para edições. A página é lida
  deste computador, pelo proxy das configurações; por isso o site do link fica
  sabendo do seu endereço IP, como acontece quando o celular gera a prévia.
  A opção **Prévias de links**, em **Configurações › Privacidade**, vem ligada
  e desliga a busca.
- **Áudio "enviando" como no celular.** Ao enviar um áudio gravado, o bloco
  dele aparece na hora na conversa, com a onda e a duração, e um círculo
  girando no lugar do play enquanto o áudio é codificado, enviado ao servidor
  e confirmado; depois vira o play. Se o envio falhar (sem internet, por
  exemplo), o bloco fica como não enviado com um botão de reenviar no lugar
  do play, que manda o mesmo áudio sem gravar de novo. Um áudio interrompido
  por fechar o ZapFast no meio do envio aparece como não enviado na próxima
  abertura.
- **Imagens e arquivos "enviando" como no celular.** Imagens coladas com
  Ctrl+V e arquivos anexados aparecem na conversa assim que você envia, em vez
  de sumir até chegarem ao servidor. Imagens e vídeos mostram por cima um
  anel com o percentual já enviado (vídeos também os megabytes); documentos
  mostram o mesmo no cartão. Vídeos saem com miniatura, duração e proporção,
  lidas do próprio arquivo. Se o envio falhar, um botão no meio da imagem ou do
  vídeo (ou **Enviar de novo** no menu da mensagem, para os demais arquivos)
  reenvia o mesmo arquivo. Um envio interrompido por fechar o ZapFast aparece como não
  enviado na próxima abertura.
- **Prévia grande ao colar ou anexar.** Uma imagem colada ou arquivos
  anexados ocupam o lugar do histórico, como no celular: o arquivo escolhido
  aparece grande, e uma faixa embaixo mostra todos, para escolher, remover ou
  anexar mais. A legenda vai na caixa de digitação logo abaixo; Esc ou o X
  descartam tudo.
- **Ícone por tipo de arquivo.** Documentos aparecem como uma folha com a
  extensão (PDF, DOCX, XLSX, ZIP, RAR, 7Z…) numa faixa na cor do tipo, na
  conversa e na prévia de anexos, em vez de um ícone genérico.
- **Duração das mensagens temporárias nas informações.** Em **⋯ ›
  Informações** de um contato ou grupo com mensagens temporárias ligadas,
  aparece por quanto tempo elas ficam (por exemplo, "Mensagens temporárias:
  7 dias").
- **Miniatura da foto citada.** Ao responder a uma foto ou vídeo, a citação
  mostra uma miniatura dele à direita, na mensagem e na barra de resposta,
  como no celular. Vale quando a mensagem citada está carregada na conversa.
- **Play sólido e velocidade colorida no player de áudio.** O play e o pause
  são formas sólidas num verde um pouco mais escuro que o de destaque (nos seus
  áudios, no tema escuro, um verde-folha mais claro, que se destaca do balão
  verde), quase do tamanho do antigo disco, sem
  o disco em volta (o círculo de carregando e os botões de reenviar e baixar
  continuam com ele). O texto do botão de velocidade muda de cor: a cor normal
  em 1x, verde-claro em 1,25x e 1,5x, passando a amarelo em 2x e a vermelho em
  3x; o fundo fica sempre o de 1x.
- **Leituras feitas no ZapFast chegam ao celular.** Antes, com **Enviar
  confirmações de leitura** desligada no ZapFast ou **Confirmações de
  leitura** desligada na conta, o ZapFast não mandava recibo nenhum, e o
  celular continuava mostrando como não lidas as mensagens lidas (e os áudios
  ouvidos) aqui. Agora, nas conversas individuais, o recibo sempre sai: o
  normal (tique azul, microfone azul) só com as duas opções ligadas; nos
  outros casos, o `read-self`/`played-self`, que vai só para os seus aparelhos,
  como faz o WhatsApp oficial com as confirmações desligadas. Nos grupos o
  recibo vai sempre, como no oficial. Os tiques azuis que os contatos mandam
  continuam aparecendo no ZapFast em qualquer combinação.
- **Leituras feitas no celular com o ZapFast fechado.** Ao reabrir, o
  servidor entrega de uma vez as mensagens acumuladas e os avisos de leitura
  do celular. Os avisos chegavam antes das mensagens, que ainda estavam sendo
  decifradas, e eram descartados, e as mensagens já lidas no celular apareciam
  como não lidas. Agora o aviso fica guardado e é aplicado quando a mensagem
  chega: ela entra como lida e sem notificação.
- **Iniciar minimizado.** Em Configurações › Sistema, **Iniciar ao entrar**
  ganhou a subopção **Iniciar minimizado**, recuada para mostrar que depende
  dela e desligada por padrão: o ZapFast abre com a janela ao entrar no
  computador, e só com o ícone na bandeja quando a subopção está ligada.
- **Aviso quando falta OpenGL 2.0.** Sem OpenGL 2.0 (driver de vídeo ausente
  ou máquina virtual sem aceleração 3D), o ZapFast fechava sem mostrar nada.
  Agora mostra uma mensagem explicando a causa: uma caixa de diálogo no
  Windows e uma notificação nos outros sistemas.
- **Borda discreta na barra de chat.** Ao digitar, o contorno da caixa de texto
  é um tom um pouco mais escuro que o fundo dela, em vez do verde-água; quem
  navega com Tab continua vendo o anel na cor de destaque.
- **Seleção primária do Linux.** Texto selecionado no compositor (com o
  mouse, pelo teclado ou com Selecionar tudo) ou no histórico da conversa
  fica disponível como seleção primária assim que a seleção termina, para ser
  colado com o botão do meio em outros aplicativos. No histórico, o texto é o
  mesmo que o Ctrl+C copiaria, com o cabeçalho de cada mensagem quando a
  seleção passa por várias; a área de transferência comum não é alterada. O clique do meio no
  compositor cola, no ponto clicado, o texto selecionado em qualquer
  aplicativo, com o mesmo tratamento de desfazer do Ctrl+V. Funciona em X11 e
  em Wayland com compositores que oferecem seleção primária via data-control
  (como o KWin); nos outros sistemas não há mudança.
- **Vídeos em qualquer formato com o FFmpeg do sistema (Linux).** Em
  **Configurações › Conversas**, **Reproduzir vídeos com o FFmpeg** usa os
  programas `ffmpeg` e `ffprobe` instalados para tocar dentro da conversa
  vídeos HEVC/H.265, AV1, VP9 e H.264 de 10 bits, com áudio HE-AAC, Opus ou
  AC-3, em vez de abrir o player do sistema. Um processo envia os quadros já
  reduzidos e girados, e outro envia o áudio; nenhuma biblioteca do FFmpeg é
  ligada ao ZapFast, então qualquer versão instalada serve. A opção vem ligada
  quando o FFmpeg é encontrado. Sem ele, fica desligada e bloqueada, explica
  as vantagens e mostra o comando de instalação da distribuição detectada em
  `/etc/os-release`: `pacman` (Arch, EndeavourOS, CachyOS), `dnf` (Fedora,
  pelo RPM Fusion) ou `apt` (Debian, Ubuntu, Mint). A detecção é ao vivo:
  instalado o FFmpeg, a opção libera sem reiniciar. Se o FFmpeg falhar com um
  vídeo, ele volta ao player interno e, em último caso, ao player do sistema.
  No Flatpak a opção fica indisponível, porque o sandbox não enxerga os
  programas do sistema. Logo abaixo, recuada como subopção, **Decodificar na
  GPU** (ligada por padrão) pede ao FFmpeg aceleração por GPU, na
  ordem NVDEC (driver NVIDIA), VAAPI (AMD e Intel) e Vulkan, conforme o que o
  `ffmpeg -hwaccels` e o hardware oferecem; se a GPU não decodificar um
  arquivo, o próprio FFmpeg segue no processador. Ela fica bloqueada quando o
  FFmpeg está desligado ou ausente, ou quando não há aceleração disponível.
- **Vídeo ampliado na janela.** Um botão no canto superior direito do vídeo,
  que aparece junto com os controles (vídeo pausado ou mouse sobre ele),
  mostra o vídeo em quase toda a altura da janela do ZapFast, sobre a conversa
  escurecida, sem usar a tela cheia do sistema. Ali os mesmos controles
  continuam disponíveis; clicar no vídeo pausa ou retoma, e o botão do mesmo
  canto, a tecla Esc ou um clique fora do vídeo o devolvem à mensagem. Espaço
  também pausa e retoma. Ao ampliar, os quadros são decodificados de novo, a
  partir do ponto atual, com até 1440 px em vez de 720 px, para não ficarem
  borrados.
- **Data flutuante ao rolar o histórico.** Enquanto você rola a conversa
  (roda do mouse, barra de rolagem ou teclado), a data das mensagens no topo
  aparece centralizada acima delas, como no WhatsApp, no formato
  `DD/MM/AAAA (sáb)`. A ordem e o separador seguem a região do sistema
  (`LC_ALL`, `LC_TIME` ou `LANG`; por exemplo `MM/DD/AAAA` nos EUA e
  `AAAA-MM-DD` na Suécia), e o dia da semana usa as três primeiras letras do
  nome no idioma da interface. A data some cerca de 1 s depois que a rolagem
  para, esmaecendo, e não aparece no fim da conversa.
- **Ctrl+V de imagens mais confiável.** Com só uma imagem na área de
  transferência (como uma captura do Spectacle), o egui não informa nada
  quando Ctrl+V é pressionado, e a colagem era decidida na soltura do V; se o
  Ctrl fosse solto um instante antes do V, a imagem não era colada. Agora uma
  soltura do V até 0,8 s depois do Ctrl, sem texto digitado nesse meio-tempo,
  também cola a imagem. Um "v" digitado continua sendo só um "v".
- **Áudios em 2,5x e 3x.** O botão de velocidade das mensagens de voz passa
  por 1x, 1,5x, 2x, 2,5x e 3x. O menu do botão direito da mensagem mostra
  todas as velocidades em duas fileiras: 1x, 1,25x, 1,5x, 1,75x e 2x na
  primeira, e 2,5x e 3x na segunda. A voz mantém o tom em todas elas.
- **Imagem ampliada sobre a conversa.** Clicar em uma
  imagem no histórico não abre mais uma janela com cabeçalho: a imagem cresce
  sobre a conversa escurecida, como o vídeo ampliado. Ao passar o mouse sobre
  ela aparecem, no topo, três botões: **Ampliar para 100%** (que alterna com
  ajustar à janela), **Copiar imagem** e **Abrir em outro app**. Um clique
  fora da imagem ou Esc fecha. A roda do mouse, o gesto de pinça, o arrasto e
  o duplo clique continuam funcionando como antes.
- **Vídeos com os decodificadores do Windows.** No
  Windows, a opção **Reproduzir vídeos com os decodificadores do Windows**
  (ligada por padrão, em **Configurações › Conversas**) toca os vídeos na
  conversa pelo Media Foundation: H.264 de qualquer perfil e som HE-AAC ou
  AC-3 sem instalar nada, e HEVC, VP9 e AV1 depois de instalar as extensões
  gratuitas da Microsoft Store. A própria opção lista as extensões que
  faltam. Quadros e som vêm de um Source Reader (RGB32 já redimensionado e
  som em float); se ele falhar com um arquivo, o player interno assume, e se
  só o som falhar, o som vem do decodificador interno. As DLLs do Media
  Foundation são carregadas sob demanda, para o ZapFast abrir também nas
  edições N do Windows sem o Media Feature Pack (nesse caso a opção fica
  indisponível com a explicação). No Windows, uma fonte `WhatsAppEmoji.ttf`
  numa pasta `fonts` ao lado do `zapfast.exe` também é reconhecida. A
  reprodução pelo Media Foundation ainda precisa ser conferida em um Windows
  real; o programa já foi aberto numa máquina virtual.
- **Créditos e código-fonte do fork.** Em **Configurações › Sobre** e no
  diálogo **Sobre**, **Código-fonte** abre este repositório
  (`intellq/zapfast`), e abaixo de "Feito com amor por Carmine Paolino" há
  uma segunda linha, "👍🏽 Com correções e modificações por intell", com
  **intell** levando a [twitter.com/intellq](https://twitter.com/intellq).
- **Menu de contexto na caixa de digitação.** O botão direito no compositor
  abre **Recortar**, **Copiar**, **Colar** e **Selecionar tudo**, em todas as
  plataformas (o `TextEdit` do egui não oferece menu próprio). A seleção feita
  antes do clique direito é preservada, então Recortar e Copiar atuam sobre
  ela e Colar substitui o trecho selecionado. As ações passam pelo tratamento
  normal de texto, área de transferência e desfazer do editor.
- **Ligações de voz 1:1.** Traz o
  [PR #220 do upstream](https://github.com/crmne/zapfast/pull/220), de
  alitura1: receber, atender, recusar e fazer ligações de voz para um contato,
  com mudo e escolha de microfone e alto-falante (lembrada para a próxima
  ligação, sem mexer nos padrões do sistema). Como no cliente da Meta, uma
  ligação chegando aparece num cartão no canto inferior esquerdo da janela,
  com foto, nome, **Recusar** e **Atender**, e também numa notificação; se a
  janela estiver na bandeja ou atrás de outras, ela volta à frente. Atendida,
  ou feita pelo botão de telefone do cabeçalho, a ligação abre numa janela
  própria e compacta, no canto superior direito da tela da janela principal:
  foto, nome, duração, barras que se mexem com a voz do outro lado, mudo,
  dispositivos e desligar. O alfinete no canto dessa janela a mantém acima das
  outras (ligado por padrão e lembrado); no KDE Plasma com Wayland isso é
  feito por um script do KWin, e nos outros desktops Wayland, que não permitem,
  o alfinete não aparece. No KDE Plasma com Wayland a janela já abre no canto,
  sem passar pelo meio da tela. Fechar a janela da ligação desliga; durante a
  ligação, fechar a janela principal só a minimiza. Quando a ligação termina,
  por qualquer motivo, a janela mostra como terminou por 2 segundos e some.
  Vídeo ainda não.
- **Sons de ligação.** Uma ligação chegando toca no alto-falante padrão até
  ser atendida ou desfeita, a menos que a conversa esteja silenciada,
  arquivada ou trancada (a notificação sai então sem som). Uma ligação feita
  daqui chama até o outro lado atender. Ao conectar e ao terminar toca um
  som, e um tom de ocupado quando o outro lado está ocupado ou recusa. Uma
  ligação atendida ou deixada em outro aparelho só para de tocar. São os cinco
  sons do Telegram Desktop, sem alteração, sob a GPL-3.0 ou posterior (aviso
  e licença em `THIRD-PARTY-NOTICES.md`).
- **Transcrição de áudios neste computador.** O WhatsApp não manda a
  transcrição aos aparelhos conectados, então o ZapFast faz a sua, com o
  Whisper (whisper.cpp), sem que o áudio saia do computador. Um botão com
  ícone na linha da duração do áudio, ou **Transcrever áudio** no menu da
  mensagem, põe o áudio na fila (uma transcrição por vez). O cartão do áudio
  mostra o progresso, com botão de cancelar, ou o erro com **Tentar de novo**,
  e depois o texto, selecionável, com o idioma e botões para copiar e
  ocultar. As transcrições ficam no arquivo criptografado enquanto o áudio
  existir. **Configurações › Transcrição de áudio** permite
  transcrever automaticamente os áudios recebidos (desligado por padrão),
  esconder o botão, escolher o modelo (Base, cerca de 148 MB; Small, cerca de
  488 MB, o recomendado e o padrão; Medium, cerca de 1,5 GB), fixar o idioma
  (o padrão é detectar) e apagar os modelos baixados. O modelo é baixado uma
  vez, na primeira transcrição, de `huggingface.co` (pelo proxy configurado)
  e conferido por SHA-256: é o único acesso novo à rede. Fica em
  `~/.local/state/zapfast/whisper` (o `uninstall.sh --purge` apaga). Exige um
  processador x86-64 com AVX2; sem ele o ZapFast avisa em vez de fechar.
- **Mensagens antigas do celular.** Ao abrir um contato antigo aparecia
  sempre "Seu celular não enviou as mensagens antigas. Verifique se ele está
  conectado à internet", com o celular ligado. O pedido vai ao celular (o
  servidor só o entrega), mas o WhatsApp dá aos aparelhos conectados só parte
  do histórico, e o celular ignora pedidos sem uma mensagem de partida. Agora
  o pedido sai pelo id de privacidade (`@lid`) do contato, como o celular
  guarda as conversas depois da migração (foi isso que fez carregar conversas
  de alguns meses), e de novo pelo número quando assim não acha nada. Uma
  conversa que chegou sem nenhuma mensagem não pede nada e diz: "As mensagens
  antigas desta conversa, se houver, ficam no celular: o WhatsApp não as envia
  aos aparelhos conectados." Quando o celular não responde, o aviso aparece no
  topo da conversa, com **Tentar de novo**, e o ZapFast não insiste sozinho.
- **Número no lugar de "Desconhecido", e uma conversa só.** Uma conversa ou
  ligação que chegava por um id de privacidade (`@lid`) antes de o ZapFast
  saber o número do contato aparecia como "Desconhecido", e, quando o número
  era descoberto, uma segunda conversa guardava as mensagens novas. Agora quem
  não tem nome aparece pelo número, como no celular ("Chamador desconhecido"
  só sem número), e as conversas do id de privacidade passam para a do número
  (mensagens, recibos, enquetes, etiquetas, rascunhos e transcrições), que
  fica com as não lidas. Arquivos que já conheciam a associação se unem ao
  iniciar.
- **Links do WhatsApp abrem no ZapFast.** Links `wa.me`, `api.whatsapp.com` e
  `whatsapp://` (inclusive os de mensagens) abrem a conversa com o texto do
  link já no compositor, ou o convite de grupo; um número sem conversa é
  consultado no WhatsApp antes. Com o ZapFast aberto, até na bandeja, o link
  vai para ele. O ZapFast se registra para o `whatsapp://` (Linux: entrada
  oculta e `mimeapps.list`; Windows: `HKCU\Software\Classes\whatsapp`)
  enquanto **Configurações › Sistema › Abrir links do WhatsApp** estiver
  ligada, o padrão; desligar devolve a associação. Links `https://wa.me/...`
  clicados fora do ZapFast passam antes pelo navegador, porque o sistema não
  associa um site a um app; a página do WhatsApp então chama o ZapFast.
- **Janela minimizada volta à frente no Wayland.** Mostrar o ZapFast pela
  bandeja, por uma notificação, por um link ou por uma segunda execução
  restaura também uma janela minimizada no KDE Plasma com Wayland, pelo
  `xdg-activation`.
- **Tamanho da janela lembrado com zoom.** Com zoom diferente de 100%, a
  janela abria menor a cada início e a cada volta da bandeja (cerca de 1/6 a
  menos em 120%). Agora volta com o tamanho em que foi deixada. A posição é
  lembrada no Windows, no macOS e no X11; no Wayland quem posiciona a janela
  é o compositor. No KDE Plasma com Wayland, o ZapFast acrescenta ao iniciar
  uma regra de janela do KWin, só para a janela principal (classe `zapfast`,
  título `ZapFast`), com a posição em "Lembrar"; a regra é criada uma vez e
  depois fica com o KWin, e o `uninstall.sh` a remove.
- **Ícone do atalho na área de trabalho logo após instalar.** No KDE, o
  atalho criado pelo `install.sh` ficava sem ícone até a próxima sessão. O
  atalho agora aponta para o arquivo do ícone, e o instalador avisa o KDE da
  mudança. O instalador do Windows avisa o Explorer das associações novas.
- **Aviso de cliente não oficial mais curto.** No cartão de abertura e no
  diálogo **Sobre**: "Cliente não oficial. Consulte os termos de serviço do
  WhatsApp."
- **Bloqueio de contatos.** Bloqueie ou desbloqueie um contato pelo menu da
  conversa (na lista ou no cabeçalho) ou pelas informações do contato, também
  a partir de um participante de grupo. Bloquear pede confirmação, e o contato
  não é avisado. A mudança vale no celular e nos outros aparelhos conectados e
  só aparece depois que o WhatsApp a aceita; uma recusa é avisada. Numa
  conversa bloqueada, a caixa de digitação dá lugar a "Você bloqueou este
  contato." com **Desbloquear**, o botão de ligação some e o encaminhamento
  mostra o contato como **Bloqueado**, sem deixar escolhê-lo. Em
  **Configurações › Privacidade › Contatos bloqueados** fica a lista, com nome
  e número, e um botão para desbloquear cada um. A biblioteca do protocolo não
  avisa quando o celular muda a lista, então o ZapFast a pede ao conectar, ao
  abrir Configurações e ao abrir uma conversa (no máximo uma vez por minuto);
  o número de quem aparece só pelo id de privacidade (`@lid`) é buscado no
  WhatsApp.
- **Submenus na largura do conteúdo.** Os submenus **Som de notificação** e
  **Etiquetas** do menu da conversa abriam com umas três vezes a largura
  necessária; agora ficam do tamanho do item mais longo.

Esta sincronização também incorpora mudanças recentes do upstream: confirmação
antes de apagar mensagens, aba de figurinhas recebidas, recuperação de conexão
via IPv4 quando IPv6 falha, melhorias de teclado e zoom de imagens, correção
da preservação de mídias já baixadas, indicador de não lidas no Windows e
chinês simplificado, papel de parede com imagem própria ou colorido pelo
tema, transição de tema a partir do centro da janela, nomes de tema sem `.json`,
paletas embutidas copiadas para a pasta de temas e mais contraste entre
conversa, balões e painéis (fastframe v0.1.6), limite de 32 notificações
do Linux aguardando clique (evita esgotar recursos no KDE Plasma), faixas de
resposta, edição e voz mais próximas do compositor,
balões com cauda e sombra suave (com relevo ajustado a temas claros e
escuros, com sombra mais leve nos escuros), etiquetas de data em relevo,
lista de conversas com cartões arredondados, relevo também na conversa
destacada, no compositor e nas faixas acima dele,
edição de nome e foto de grupos, guia para criar temas e rótulos de
acessibilidade nos botões. Da versão 0.17.0 vieram ainda o fastframe v0.1.7
(árabe desenhado em Segoe UI no Windows), o corte de textos de uma linha em
árabe e hebraico no fim lógico e os nomes das fontes de reserva no log. A
confirmação de mensagens veio do upstream; a opção
de sincronizar a exclusão "para mim" com o celular é uma extensão deste fork.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshot-group-light.png">
  <img src="docs/screenshot-group.png" alt="A titled group chat with participant names, reactions, a quoted mention, and a poll">
</picture>

<picture>
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshot-link-light.png">
  <img src="docs/screenshot-link.png" alt="The linking screen with the QR code">
</picture>

## What it does

- **Links to your phone.** Scan a QR code or link with your phone number.
  Recent history is copied to this computer after linking and stored here.
- **Chats.** See pinned, unread, muted, and archived chats, typing indicators,
  and message status. Incoming text uses the available conversation width
  before wrapping, while outgoing bubbles stay compact. Search chats, saved
  messages, and contacts. The
  **Search** icon in a chat's header (or **Ctrl+F**) opens a pane beside the
  chat, as in WhatsApp Desktop, listing its matches newest first with the time
  and the line that matched. The calendar narrows them to one day, or lists
  that day's messages when the field is empty. Clicking a result, or reaching
  it with the arrow keys and pressing Enter, brings it into view with a brief
  flash; Escape closes the calendar, then the pane. The pane can be dragged
  wider, and in a narrow window it lies over the conversation instead of
  squeezing it. The newest 80 matches are listed, and the pane says when there
  are more. Right-click a group or a followed channel and choose **Leave group**
  or **Leave channel** to leave it, with the option to archive it in the same
  step; the local history stays on this computer and the chat keeps its
  messages.
  Filter the list to unread, private (one-to-one), favorites, or group chats
  with the chips under the search bar; a chip with unread chats shows how many
  it has. Right-click a chat and choose **Add to favorites** to mark it.
  Favorites sync with your phone both ways, and the **Favorites** chip lists
  them in the phone's order below any pinned chats. A chat added here goes to
  the end of the list; channels cannot be favorites.
  Followed channels have their own **Channels** chip and stay out of the other
  filters; right-click it to mute or unmute every channel at once. **Archived**
  opens the archived chats. Right-click a chat and choose **Mark as unread**
  to put an empty dot on it, as on the phone; the mark syncs with your phone
  both ways, and opening the chat or a new message clears it. Opening a chat with
  unread messages scrolls to an "unread messages" divider above the first one.
  Pinned chats stay in pin order (most recently pinned first), regardless of
  new messages. Like on the phone, you can pin up to three chats. Chat and contact name searches ignore accents, so `Angel`
  finds `Ángel`.
  The filters stay on one row and scroll horizontally in narrow sidebars.
  Unnamed groups use a shared participant summary for their title and subtitle.
  It names each saved contact by its whole first name as saved on the phone (the
  first word of the name when none is known), repeated names appear as `Andrea ×3`, and your
  own entry is shown as `You`.
  Incomplete group metadata preserves known names and retries with backoff;
  an empty cached subject remains eligible for recovery.
  Typing indicators show other participants, excluding your own linked devices.
  Newsletter channels are read-only; publishing channel posts is not supported.
  Channels show their own pictures, read from the channel's details on WhatsApp.
- **Account privacy.** Settings, Privacy shows who can see your last seen,
  online status, profile photo, and About, who can add you to groups, your
  account read receipts, and whether unknown callers are silenced, and
  changes them on your phone, so a change applies on every linked device. A
  category set to **My contacts except** shows as such; the people it excludes
  are chosen on the phone. The values are read when ZapFast connects and when
  Settings opens; without a connection they cannot be changed.
- **Block contacts.** Block or unblock a contact from its chat menu, in the
  list or the chat header, or from its info, including a group participant's.
  Blocking asks first, and the contact is not told. The change applies on the
  phone and every linked device, and shows once WhatsApp accepts it; a refusal
  says so. A blocked chat replaces the composer with "You blocked this
  contact." and **Unblock**, hides the call button, and the forward list shows
  the contact as **Blocked** and does not let you choose it. Settings, Privacy,
  **Blocked contacts** lists them by name and number, each with **Unblock**.
  The protocol library does not report when the phone changes the list, so
  ZapFast asks for it when it connects, when Settings opens, and when a chat
  opens (at most once a minute); contacts listed only by privacy id (`@lid`)
  have their number looked up on WhatsApp.
- **Read state across devices.** Reading a chat syncs its unread badge with
  your phone and other linked devices, including when read receipts are off.
  Replies from another device clear preceding unread messages. In direct
  chats, messages read and voice messages played here always tell your other
  devices; the contact sees them only when both ZapFast's read-receipt toggle
  and the account's read receipts are on (otherwise ZapFast sends the
  `read-self`/`played-self` receipts only your devices receive). Groups always
  get read and played receipts, as from the phone. A hidden window does not
  read messages.
- **Conversations.** See replies, reactions, edits, deleted messages, read
  receipts, sender names, and group pictures. Older messages load as you
  scroll up, first from the local archive and then from your phone. The phone
  is asked by the contact's privacy id (`@lid`), where it now files chats, and
  once more by the number when that finds nothing. WhatsApp gives linked
  devices only part of the history: a chat whose messages the phone did not
  send when it was linked shows no request at all, since the phone answers
  only from a message it can start at, and a request the phone leaves
  unanswered says so at the top of the chat, with **Try again**.
  Group messages show two gray checks after every recipient has received
  them, and blue checks after every recipient has read them. The recipient
  list and individual receipts are saved locally; later membership changes
  do not change that list. If the original recipients are unknown, ZapFast
  waits for the phone's aggregate status instead of guessing from one reader.
  A message that could not be sent says "Not sent" beside its time. ZapFast
  does not retry it; send it again yourself. Timestamps follow the system's
  12-hour or 24-hour clock: the time format on Windows and macOS, and GNOME's
  clock format or the time locale (`LC_TIME`) on Linux. **Select** in a
  message's menu, or Ctrl-click (Command-click on macOS) on a message, starts
  a selection: click more messages to add or remove them, Shift-click to add
  everything up to the one you click, then **Forward…** sends them together,
  in their original order, or Escape cancels. A batch goes out one message at
  a time, each starting once the one before it reached WhatsApp, so a picture
  cannot overtake the text that came before it.
- **WhatsApp formatting.** Bold, italic, strikethrough, code, lists, quotes,
  mentions, and link previews are supported. Links are clickable. A link you
  type gets a preview above the composer, fetched from its page, and the
  message carries it as the phone's would; Settings, Privacy can turn this
  off, since the linked site sees the request. Hebrew,
  Arabic, and mixed lines follow the Unicode Bidirectional Algorithm, so
  numbers, punctuation, and embedded words stay in reading order and brackets
  face the right way. As in WhatsApp, a message whose first strong character is
  Hebrew or Arabic is aligned to the right, with its time on its own line when
  the text has more than one. Carets and copied text stay on the logical message.
  Emoji use the bundled Noto
  Color Emoji on macOS and Windows. On Linux, ZapFast prefers an installed
  Noto Color Emoji and falls back to the bundled copy. **WhatsApp emoji** in
  Appearance draws them with a WhatsApp emoji font you install yourself
  (`WhatsAppEmoji.ttf` in the emoji fonts folder, or the Linux package
  `ttf-whatsapp-emoji`); WhatsApp's artwork is not ZapFast's to ship, so it is
  neither bundled nor downloaded. Emoji-only messages are larger.
- **Readable text.** Secondary text in the built-in light and dark themes
  reaches WCAG AA contrast. Inside message bubbles, times, ticks, and other
  grey text adjust to the bubble's colour, in custom themes as well.
- **Screen-reader access.** AccessKit exposes the interface to desktop
  accessibility services. Custom buttons, chat rows, settings switches and
  message text include readable labels. Windows NVDA navigation still needs
  platform verification; keyboard and screen-reader support is not complete.
  In the chat view, Tab cycles through the message input, send/voice button,
  the plus menu, emoji, profile, sidebar toggle, New chat, Settings, search,
  and chat filters, then returns to the input. Shift+Tab reverses that order;
  hidden controls are skipped. Messages, reactions and chat rows are not stops
  in this cycle; Alt+Up/Down switches conversations. Menus, dialogs and Settings
  keep their own Tab navigation. Every focus border is a single one-pixel inset
  outline following the control's shape, including circular voice buttons.
  Text fields stay outlined while active; other outlines hide when you use the
  mouse. Focus stays below menus, dialogs, and toasts.
- **Safer desktop opening.** Links open only web pages or email addresses.
  Common documents and media open in their default apps; executable, script,
  and unrecognized attachment formats open their containing folder instead.
- **Use interactive messages.** Business templates and button messages show their
  image above the text and their options in separate rows below the timestamp.
  Reply buttons send the selected option with a quote of the original message.
  Simple lists open a choice dialog, web links open in your browser, and copy-code
  buttons copy to the clipboard. Unavailable actions have a phone icon and an
  explanation. Lists group choices by section, with descriptions and keyboard support.
  Carousels show separate cards in a horizontal strip, with images, web links,
  and copy-code actions. Short carousels keep the timestamp beside their last
  card. When more cards are offscreen, overlaid previous/next arrows move one
  card at a time. **Shift + mouse wheel** and horizontal touchpad scrolling also
  work over the cards, without a bottom scrollbar. Their text can be selected, copied, and searched.
  Images use the same download, retry, and automatic-download setting as photos.
  Previously unsupported messages are recovered from the local archive when their
  original message is available and they have not been edited, without relinking.
  Other embedded attachments and templates containing only a
  reference to server-side text still need the phone.
  Meta AI replies show as text, with code in monospace blocks and tables as
  rows; their images, maps, and other media parts still need the phone, and a
  reply made only of those shows as an unsupported message.
- **Errors stay readable.** Confirmations such as "Copied" fade after a few
  seconds. Error messages stay above the composer until you dismiss them, and
  a button copies their text for a bug report. A repeated error replaces its
  earlier copy, and only the three newest are kept.
- **Send attachments with captions.** Paste a picture, drop files, or choose
  **Send files** from the plus menu. They stay in the composer until you send them or press Escape.
  Pasting a picture uses its image data without adding the source URL or HTML
  to your caption. Files copied in Finder, Explorer, or a Linux file manager
  paste as the files themselves, not their icons. Text-only clipboard contents
  still paste as text.
  MP3, M4A, AAC, and OGG files go as audio messages; other audio, such as
  WAV or FLAC, goes as a document so the recipient gets the original file.
- **Mute chats** for eight hours, one week, or indefinitely. The setting also
  applies on your phone and to desktop notifications. Mute changes from your
  phone survive history arriving later, including during initial linking.
  Existing installations request one settings refresh after upgrading to
  recover previously lost mute settings and pin order, without relinking.
- **Read the last message from the chat list.** When a chat's one-line
  preview is cut short, resting the pointer on it shows the whole message in
  a tooltip, as in WhatsApp Web, without opening the chat or marking it read.
- **Delete chats.** Remove a chat and its messages from the chat list's
  right-click menu. The phone deletes it first, so this needs a connection,
  and the chat only leaves this computer once the phone has confirmed. Chats
  you delete or clear on the phone disappear here as well, and history that
  was already on its way does not bring them back.
- **Clear chats.** Empty a chat's messages from the menu in its header, in the
  same way as WhatsApp Web, and keep the chat itself in the list. The phone
  clears it first, so this needs a connection, and the messages only go from
  this computer once the phone has confirmed. Starred messages and downloaded
  media go with them, and history that was already on its way does not bring
  them back.
- **Voice messages.** Play, seek, record, reply with, and send voice messages
  in the chat. The speed chip cycles between 1x, 1.5x, 2x, 2.5x, and 3x,
  and the message menu offers 1x, 1.25x, 1.5x, 1.75x, and 2x, with 2.5x
  and 3x on a second row, keeping the speaker's
  pitch; the last choice applies to later messages. When one ends, playback
  carries on through the voice messages right after it that you have not
  heard yet, as on the phone; any other message ends the run. A voice message
  you send shows at once with a spinner in place of play until the server has
  it, and a retry button if it fails. The app
  normalizes quiet recordings and handles OGG/Opus without external tools. On Linux and
  Windows, music and other media playing in other apps pause while you record
  or play a voice message, or watch a video with sound, and resume afterwards;
  only players that were
  playing are resumed. **Pause other media while recording or playing** in
  Settings turns this off. Linux uses MPRIS, so any player that implements it
  works; macOS has no public API for this, so the switch is hidden there.
- **Voice calls.** One-to-one voice calls, from upstream pull request #220 by
  alitura1. A ringing call shows as a card in the window's bottom-left corner
  with the caller's picture and name and **Accept** / **Decline**, and as a
  desktop notification. An answered call, or one started from the phone
  button in a chat's header, gets its own small window in the top-right corner
  of the main window's screen, with the call's length, bars that move with the
  other side's voice, mute, the **Microphone** / **Speaker** pickers, and
  hang up. A pin keeps that window above the others where the desktop allows
  it (on KDE Plasma under Wayland through a KWin script, which also opens the
  window in that corner from the start). The devices a call uses are
  remembered for the next one without changing the system defaults. When a
  call ends, for any reason, its window shows how it ended for two seconds and
  goes. Calls make sound: an incoming call rings on the default speaker until
  it is answered or gone (silently for a muted, archived or locked chat), an
  outgoing one rings back, and the call plays a sound when it connects and
  when it ends, and a busy tone when the peer is busy or declines. The five
  sounds are Telegram Desktop's, unmodified, under the GPL-3.0-or-later. A
  person whose privacy id (`@lid`) has no name is shown by their number, and
  the chat begun under the privacy id joins the one under the number once the
  mapping is known.
- **Voice message transcription.** WhatsApp sends linked devices no
  transcript, so ZapFast makes its own with Whisper (whisper.cpp), on this
  computer: the audio never leaves it. An icon button on a voice message's
  duration row, or **Transcribe audio** in its menu, queues it (one at a
  time). The card shows the progress with a cancel button, or the failure with
  **Try again**, then the selectable transcript with its language and buttons
  to copy and fold it. Transcripts live in the encrypted archive for as long
  as their audio message does. **Settings > Voice message transcription**
  turns on transcribing received voice messages automatically (off by
  default), hides the button, picks the model (Base, about 148 MB; Small,
  about 488 MB, recommended and the default; Medium, about 1.5 GB), fixes the
  language (detected by default) and deletes the downloaded models. The model
  is downloaded once, on the first transcription, from `huggingface.co`
  through the configured proxy and checked against its SHA-256: it is the only
  network access this adds. It needs an x86-64 processor with AVX2; without
  it ZapFast says so instead of crashing.
- **Send messages.** Press Enter to send text and Shift+Enter for a new line.
  You can swap these keys in Settings. The composer is focused when you open
  or return to a conversation, and clicking empty conversation space returns
  focus to it; invoking search keeps focus in search, and
  Escape clears search and returns to the composer; another Escape closes the
  chat and saves your text draft. Drafts are kept in the encrypted archive, so
  unsent text survives closing ZapFast and restarting. Open menus, dialogs, and unfinished actions
  are dismissed first. Sending while reading older messages keeps your place; use the
  newest-message button or End to return to the latest message. With **Replace text with emoji** on in Settings,
  type `:name` to autocomplete an emoji without leaving the composer; it is
  off by default, so text such as `:P` is sent as typed, or `@` in a group to mention a member.
  Reply, react with any emoji, edit, forward, delete, and check when a message was sent,
  delivered, or read. Replies can be text, attachments, voice messages,
  stickers, or GIFs. A reply never goes out without its quote: if the
  original is no longer available on this computer, nothing is sent, the text
  or attachments return to the composer, and a voice message waits above it
  to be sent again or discarded. Cancel the reply to send without a quote.
  Quotes carry a bar and name in the quoted person's colour; clicking one
  scrolls back to the original, which flashes briefly, as a search result
  does. The same right-click menu copies a message's ID, which
  helps when looking one up for a bug report.
  Opening a message's context menu outlines that message until the menu closes.
  The full reaction picker stays beside the menu and adds a target preview.
  The conversation stays still while you choose; the emoji
  grid can scroll. Quick reactions learn from usage on this computer, independently
  of inserted emoji. These preferences do not sync from the phone.
  Hovering a message also shows a small smiley control beside it; clicking it
  opens the full reaction picker for that message, so right-click is never required.
  Deleting a message asks first and says which copies go:
  deleting for everyone leaves "This message was deleted" in the chat, while
  deleting for yourself removes the message from this computer and, by default,
  sends the deletion to your phone and linked devices. Uncheck **Also delete
  on phone** to delete only the local copy. Neither can be undone, because
  the archive here is the only local copy.
- **Disappearing-message timers.** Outgoing messages use the chat's known
  timer, including replies, attachments, edits, and forwards. Forwarded copies
  use the destination chat's timer. Received messages remain in the local archive
  after they expire on the phone.
  A clock badge on chat avatars shows enabled timers and follows changes from
  the phone. Changing the default timer for new chats leaves existing chats alone.
- **View attachments.** ZapFast can download audio, videos, images (including
  static stickers), animated stickers, and documents automatically, each with
  its own switch: audio, images, and animated stickers are on by default,
  videos and documents off. Set the download size limit from 1 to 64 MiB; it applies to
  automatic downloads and clicks. Anything left off downloads with a click.
  Photos, stickers, GIFs, voice messages, audio,
  locations, contacts,
  polls, and link previews appear in the chat. Click a downloaded JPEG, PNG,
  WebP, or GIF photo to expand it over the chat, as an expanded video is, or
  choose **Open externally**. Hovering the picture shows buttons along its top
  edge to switch to 100% and back, copy it, and open it in another app; a
  click beside the picture or Esc closes it. In the preview, the mouse wheel and Ctrl+wheel
  (Cmd+wheel on macOS) zoom around the pointer, as does a trackpad pinch on
  macOS and Windows. Drag a zoomed picture to move it; where a trackpad scrolls
  smoothly (macOS, Wayland), two-finger scrolling moves it instead of zooming.
  Double-click to switch between fitting the window and the original size.
  Copy the image to your clipboard via the copy button over the picture, the
  right-click menu (**Copy image**), or
  Ctrl+C (Cmd+C on macOS); a downloaded image's message menu has **Copy image**
  too, without opening the preview. Click a video to play it in its message, with
  sound, a seek bar, and a mute switch; round video messages play inside their
  circle with a progress ring, like on the phone. A video that is not
  downloaded yet downloads first and then plays. On Linux with FFmpeg
  installed, **Play videos with FFmpeg** in Settings (on by default) plays
  HEVC, AV1, VP9, 10-bit H.264, and HE-AAC, Opus, or AC-3 sound in the chat
  through the system's `ffmpeg`; without it, the switch is disabled and names
  the install command for Arch, Fedora, or Debian and Ubuntu. Otherwise,
  videos in codecs other than H.264, such as HEVC, or with an audio track
  ZapFast cannot decode open in your system player. The expand button in a
  playing video's top right corner shows it nearly as tall as the window;
  Escape brings it back. **Open in system player** in a video's right-click menu
  does the same. Unsupported pictures and documents keep opening in their
  default desktop apps. **Save as…** in a downloaded
  attachment's right-click menu keeps a copy wherever you choose, starting in
  your Downloads folder. Profile pictures and downloaded images support
  Windows drive paths and filenames with spaces or non-ASCII characters.
  If an attachment has expired, ZapFast asks your
  phone to upload it again. Downloads stop after two minutes with an inline
  retry error if they cannot finish; the menu disables Download while one is running.
  Cached attachment filenames use extensions of at most 16 ASCII letters, digits,
  or hyphens; invalid or empty extensions are saved as `.bin`. A photo, video,
  or voice message sent to be viewed once shows as a view-once placeholder:
  WhatsApp opens it only on your phone, as it does in WhatsApp Web.
- **Polls.** Choose **Create poll** from the plus menu beside the message
  field to create a poll with 2–12 answers. Turn off **Allow multiple
  answers** for a single-choice poll.
  Click an answer in a poll to vote; click a selected answer again to remove
  it. Each option shows a result bar and a checkmark for your selection.
  **Show votes** lists participants and vote times, updating as votes arrive.
  Results and your selection are retained in the encrypted archive, including
  votes received through phone history. New polls received live start at zero votes
  without asking the phone for earlier results. Polls from history or offline
  delivery automatically request earlier votes when visible. Until a usable
  snapshot arrives, results are labelled incomplete and requests retry with backoff;
  no refresh button or relinking is needed.
  Voting needs the original poll's key;
  if that key is missing, the message explains that voting is available on your
  phone. Creating polls in disappearing-message chats is not yet supported by
  the protocol library's poll API, so ZapFast blocks it instead of ignoring the timer.
- **Emoji, GIF, and sticker picker.** Search emoji and GIFs, use recent emoji
  and stickers, and choose a skin tone by clicking a compatible emoji, then its
  variant. The same choices are available when reacting to a message; the
  default yellow emoji remains an option. Recent emoji occupy up to three rows, only as many as they fill,
  and choosing another skin tone replaces the older tone of that emoji there;
  a recent emoji is used in its tone with one click. Emoji that come in skin
  tones carry a small solid triangle in their corner, and the category tabs
  under the grid highlight the section scrolled into view.
  Add stickers to Favorites with a
  right-click. Favorites
  sync with your phone both ways, and Recent holds only stickers you sent. Emoji autocomplete and
  picker search select their first match; use the arrow keys and Enter to
  choose it. GIF search needs a free GIPHY API key unless the build includes
  one.
- **Sticker packs.** A tab strip like WhatsApp's holds Recent, Favorites, and
  every pack. ZapFast adds a Received tab (the speech bubble) with the
  stickers people sent you that are already downloaded, newest first, each
  once, leaving out locked chats. Search stickers by emoji, by a word that
  names one, or by pack name. Import a pack from a `signal.art` link or
  `.wastickers` file, or make your own packs from any sticker with a
  right-click. Open WhatsApp sticker packs shared in a chat and add them, or
  send any of your packs as one.
  Turn any picture into a sticker: crop it square, keep its transparent
  background, and tag it with emojis.
  Animated packs remain animated. Packs are stored as WebP files on your
  computer.
- **Consistent names.** Names from your address book come first, as on the
  phone, and public WhatsApp profile names (shown with a `~`) fill in, across
  chats, replies, mentions, and notifications.
- **Groups.** See members, sender names, and sender pictures (shown in groups
  only, as on WhatsApp). Announcement
  groups are read-only for non-admins. Rename a group with the pencil beside
  its name in the group's info (Enter saves, Escape cancels), and click its
  photo to change or remove it; the picture is cropped to a centred square
  and sent at up to 640 pixels. These appear when WhatsApp lets you edit the
  group's info (every member, or only admins when the group is set that
  way), apply for everyone in the group, and show here once WhatsApp accepts
  them. Changes made on the phone or by other members arrive as before. Clicking a `chat.whatsapp.com` invite
  link shows the group's name, size, and description, and joins it (or sends a
  join request when admins approve members) without leaving ZapFast.
- **Presence.** See online, last-seen, and typing status, and send your typing
  status. Like WhatsApp Web, ZapFast shows you as online only while its window
  is focused, and goes offline ten seconds after you switch away or hide it to
  the tray, so your phone keeps receiving notifications meanwhile.
- **Idle rendering.** History-sync progress updates when data arrives. Animated
  stickers and GIFs show a still first frame and play while hovered in the
  focused window, keeping idle conversations from continuously repainting.
- **Sync recovery.** A conflicting app-state collection is recovered through
  whatsapp-rust, including requesting a fresh snapshot from the paired phone
  when validation fails. Private read-state updates run one at a time. Failures
  pause the whole queue with backoff from 30 seconds to 15 minutes; pending reads
  remain saved and resume automatically. New messages can still arrive.
- **Connects over either address family.** On a direct connection, ZapFast
  dials every address the WhatsApp host resolves to, IPv6 and IPv4, starting
  the next one a quarter of a second after the last, and keeps the first that
  answers. A network whose IPv6 has a route but no path past the gateway, as on
  some phone hotspots and captive portals, still links over IPv4. Each
  reconnect resolves the names again, so changing networks does not need a
  restart. With a proxy configured, ZapFast dials the proxy instead:
  `socks5h://` and `http://` proxies resolve WhatsApp's host themselves, and
  `socks5://` hands the proxy the first address this computer resolves.
- **Reconnects after sleep.** After the computer wakes from sleep, or when the
  connection has received nothing for two minutes, ZapFast reconnects and
  fetches what arrived meanwhile, instead of waiting on a connection that
  looks open but no longer delivers.
- **Runs in the background.** Closing the window keeps ZapFast linked in the
  system tray. Reopen it from the tray or by launching it again. Quit from the
  tray or with `Ctrl+Q`, or disable this behavior in Settings. The window
  reopens where you left it; on Windows and X11, one that would open on no
  connected monitor (for example on a display that is now unplugged) moves to
  the middle of the primary monitor.
- **Start at login.** Turn on **Start at login** in Settings to start ZapFast
  when you log in. Its sub-option **Start minimized**, off by default, loads
  only the tray icon, without opening a window. It adds
  `~/.config/autostart/zapfast.desktop` on Linux, a LaunchAgent in
  `~/Library/LaunchAgents` on macOS, or a `Run` entry for your user on Windows,
  and removes it when turned off. The entry passes `--start-hidden` only while
  **Start minimized** is on, and ZapFast rewrites it when that changes; a
  minimized start opens the window anyway when no tray is available. The
  Flatpak does not offer this setting yet.
- **Desktop notifications.** Get notifications with the chat picture when you
  are away from the open chat. Muted chats do not notify you, and archived
  chats stay quiet until you unarchive them. Windows notifications
  identify ZapFast as the sender and show chat pictures as small circular icons;
  installed and portable builds register this identity in the current user's registry.
  On Linux and Windows, clicking a notification opens the chat at the message
  it announced. On Linux, reading the chat here or on another device dismisses
  its outstanding notifications. Linux keeps this link for the 32 most recent
  notifications: older ones stay on the desktop, but clicking them or reading
  their chat no longer reaches them. On macOS, notifications use
  the installed ZapFast application's identity without an application chooser;
  unregistered development builds skip notifications if that identity is unavailable.
  Sounds follow Pidgin: **Message sound** plays for every new message, in
  chats and groups alike, and **Mention sound** when someone in a group
  mentions you or replies to one of your messages. Each can be Pidgin's classic
  message sound (the default for messages), its alert (the default for
  mentions), the system's notification sound, no sound, or an audio file
  (WAV, MP3, or OGG Vorbis) that ZapFast plays itself. Turning off **Play
  sounds for group messages** keeps group notifications silent unless they
  mention or answer you. **Notification sound** in a chat's right-click menu
  gives that chat its own sound for every message in it, mentions included,
  stored in the encrypted archive.
- **Unread count on the taskbar.** Linux desktops that implement the Unity
  Launcher API show the number of unread chats on the app icon; KDE Plasma needs
  **Show badges** enabled in Task Manager. On Windows, ZapFast overlays a compact
  count on its taskbar button while the window is open, showing `99+` above 99.
  Windows must be using its regular taskbar icon size for overlays to appear. As
  in WhatsApp, the count is of chats, not of the messages in them or of toasts
  kept in Windows notification history: archived, muted, and locked chats are
  left out, and a chat marked unread counts. Reading a chat lowers the count,
  and zero removes the overlay.
- **Update notices.** ZapFast checks GitHub once a day and shows a download
  link when a newer release is available. You can turn this off in Settings.
- **Themes.** Light, dark, follow the system, or a local JSON palette. Native
  Linux packages can follow Omarchy colors without restarting the app. Zoom with
  Ctrl+plus and Ctrl+minus. On Linux text is hinted and antialiased as the
  desktop asks (its font settings through the desktop portal, else
  fontconfig), and follows changes to them without a restart.
- **Copy text.** Select part of a message or copy across messages in
  WhatsApp's `[time, date] Name:` format. Contact names and numbers are also
  selectable, with Brazilian numbers shown as `(DDD) XXXX-XXXX` or
  `(DDD) XXXXX-XXXX`.
- **Keyboard shortcuts.** `Ctrl+K` or `Ctrl+Shift+F` searches your chats,
  where `↑`/`↓` selects a matching chat and Enter opens it ready for typing;
  `Ctrl+F` searches the open chat as in WhatsApp (`↑`/`↓` walk the results
  and Enter jumps to one; with no chat open it searches your chats, and in
  Settings it searches the settings), `Alt+↑/↓` or WhatsApp's
  `Ctrl+Shift+[`/`Ctrl+Shift+]` switches chats and
  keeps the active chat visible in the list, `↑` in an empty input edits your
  previous message, `PgUp`/`PgDn` scroll the open chat by about a page,
  `Home`/`End` jump to the top or newest message of the open chat (when the
  input is empty), `Esc` cancels the current action, `Ctrl+L` focuses the
  message input, `Ctrl+N` opens New chat, `Ctrl+B` collapses or expands the
  chat list, and `?` (outside text fields) or
  `Ctrl+/` opens Keyboard shortcuts (use Command instead of Ctrl on macOS).
  The × at the left of the shortcut hints
  hides the bar; bring it back with **Show shortcut hints under the message
  box** in the Keyboard shortcuts dialog.
- **Collapsed chat list.** Hiding the chat list (`Ctrl+B`, or the button beside
  **New chat**) leaves a narrow column of avatars.
  It shows the same chats as the full list under the current filter, with
  unread badges (dimmed for muted chats); hovering names a chat, clicking opens
  it, and `Ctrl+B` brings the full list back.
- **Local storage.** Messages, contacts and sticker metadata are stored in a
  SQLCipher-encrypted archive, unlocked automatically through your OS keyring.
  Existing plaintext archives are migrated on first use. Attachments remain
  ordinary files in the cache directory. Unlinking deletes both and removes this device from
  your phone.

## What it does not do yet

- Play videos in codecs other than H.264 in the app on Windows, macOS, or
  Linux without FFmpeg (they open in your system player).
- Video calls, group calls, and screen sharing.
- Status posts, communities, newsletters, and group administration beyond a
  group's name and photo (members, admins, descriptions, settings).
- Submit interactive forms, payments, shopping flows, or carousel selections.
  Use these in WhatsApp Web or on your phone. Embedded videos and documents,
  and templates without readable text also need another client.

## Installing

On macOS with Homebrew: `brew install --cask crmne/tap/zapfast`.

ZapFast was previously called FastsApp. Version 0.13.0 introduces the new
package and executable names. On Arch Linux:

```sh
yay -S zapfast-bin      # the released build, ready made
yay -S zapfast          # the release, built from source
yay -S zapfast-git      # built from the latest commit
```

With [Nix](https://nixos.org), install the package directly from its flake:

```sh
nix profile install github:crmne/zapfast
```

NixOS configurations can add the repository as a flake input and include
`inputs.zapfast.packages.${pkgs.system}.default` in
`environment.systemPackages`.

Builds for every release are on the
[releases page](https://github.com/crmne/zapfast/releases):

| Platform | File |
| --- | --- |
| Linux x86_64 and arm64 | `zapfast-vX.Y.Z-<target>.tar.gz`, with the desktop file and icon in `packaging/` |
| Windows x64 and arm64 | `zapfast-vX.Y.Z-<target>-setup.exe` (no administrator rights needed), or the `.zip` |
| macOS, universal | `zapfast-vX.Y.Z-macos-universal.dmg` |

On macOS, the rounded Dock icon matches the app bundle. Native menus provide
Settings, editing, search, view controls, and window commands. The traffic
lights share the chat header, leaving more room for conversations in a normal
window. Settings is also available with `⌘,`.

The macOS release process signs the app with Developer ID, submits the DMG
to Apple's notarization service, and staples and validates its ticket before
publishing. Open the DMG and drag **ZapFast** to Applications.
When upgrading from FastsApp on macOS, quit the old app and remove its
application bundle after installing ZapFast.

Releases before 0.13.0 keep their original FastsApp filenames.

### Flatpak

Flatpak packaging lives in `packaging/flatpak/`, following Spotifast's source
manifest and release-bundle setup. Future releases will attach an x86_64
`.flatpak` bundle; install a downloaded bundle with `flatpak install --user FILE`
and run `flatpak run rocks.zapfast.ZapFast`. Flathub publication is pending;
ZapFast is not yet listed there. See [PACKAGING.md](PACKAGING.md) for local builds
and preparing a Flathub submission. File selection uses desktop portals;
the sandbox has no general access to your home directory.

### Archive encryption

The archive key is a random 256-bit secret in Secret Service on Linux, Keychain
on macOS, or Windows Credential Manager. Linux needs a working Secret Service
provider (for example GNOME Keyring or KeePassXC with Secret Service enabled).
If the keyring is locked or unavailable, unlock it and click Retry; ZapFast keeps
its archive intact and waits before connecting. It never saves a replacement
plaintext archive. Back up both the archive and its OS keyring key: copying only
`archive.db` to another computer is insufficient.

A missing key is different from a locked keyring. If ZapFast says the key is
missing, restore the original OS credential store or use the original profile
location. Do not delete the archive or create replacement credentials: neither
can decrypt the existing archive. If the original key cannot come back,
**Start over…** on that screen renames the unreadable archive to
`archive-unreadable-<date>.db` beside it, forgets the linked session, and
shows the linking screen: linking again brings recent history back from your
phone. Remove the old ZapFast entry under Linked devices on the phone
afterwards. For help, report the OS, app version, whether
the profile was moved/restored, and the error text with personal paths removed.
Never attach the archive, keys, or full logs from older releases.

Only `archive.db` and its SQLite journal/WAL are encrypted. Device credentials in
`session.db`, downloaded media, profile pictures, favorite sticker files and settings
remain ordinary files. Use full-disk encryption for those files, swap, backups and
remnants of the old plaintext archive. Migration removes the original only after
verifying its encrypted copy; deletion cannot guarantee erasure from SSDs or
snapshots. Keyring unlocking also does not protect against software running as you
while your login is unlocked.

### From source

ZapFast needs Rust, a C/C++ toolchain, CMake and Perl (for bundled OpenSSL). `rust-toolchain.toml` pins the exact version. On Linux,
it also needs GUI development packages:

```sh
# Debian and Ubuntu
sudo apt install libxkbcommon-dev libwayland-dev libgl1-mesa-dev libasound2-dev cmake perl
# Arch
sudo pacman -S libxkbcommon wayland mesa alsa-lib cmake perl
```

Then:

```sh
cargo install --path .
zapfast
```

With Nix, `nix develop` provides the pinned Rust toolchain and all native build
dependencies. From the checkout, use `nix build .#zapfast` to build the package
or `nix run .#zapfast` to run it.

`cargo install` puts the binary on your `PATH`, but it does not add a launcher
entry. On Linux, a source build can have the entry the packages install:

```sh
cargo build --release --locked
packaging/install-user.sh
```

The script installs the binary, the icon, and a desktop entry under
`~/.local` (or the prefix you pass), with `Exec=` set to the installed binary's
full path, since a graphical session often lacks `~/.local/bin` on `PATH`. It
is Linux-only; on macOS and Windows use a packaged release or run the binary
directly.

`whatsapp-rust` is pinned to a Git commit because version 0.7.0 on crates.io
enables a `simd` feature that needs nightly Rust. The pinned commit builds on
stable Rust and includes the upstream fixes for missing app-state snapshots and
conflicts that make no progress. ZapFast does not reset your session to recover
a collection.

## Using it

On first start, scan the QR code from WhatsApp under **Linked devices**,
**Link a device**. To link without the camera, click **Link with phone number
instead**, enter your number with its country code, then enter the shown code
on your phone.

WhatsApp then sends your recent history. This can take a few minutes. A banner
shows the progress. New messages arrive live, and your phone does not need to
stay on the same network.

Right-click a chat or message to open its menu. Double-click beside a message,
or on its edge, to reply to it (a double-click on its text still selects the
word). Open Settings from the gear or with `Ctrl+,`, and close them the same
way. The pencil opens **New chat**, with **Message yourself** and
**+ Add contact** at the top, followed by searchable contacts. Add contact also
lets you message a new number without saving it. **Also save to your phone's
contacts** in that dialog adds the contact to your phone's address book too, as
the phone asks; the next contact starts from your last choice. You
can also open a group member's contact card. Saved names sync through WhatsApp
to your phone and linked devices.

### Locked chats

**Lock chat** in a chat's right-click menu moves the chat into a locked
folder: it disappears from the chat list, search, and the unread badge, and
its messages never raise a desktop notification. The lock state syncs
with your phone and other linked devices.

Choose **Locked** beside the other chat filters, type your local code, and press
Enter or choose **Open locked chats**.
The tab appears when locked chats exist, without a count or names before opening.
If no local code exists, it offers to set one up. The local code is separate
from your phone's code and is a visibility control, not an extra encryption layer.
Search inside the open tab filters its chats. Leaving it, changing or clearing
the code in Settings, or closing the window hides the locked chats and closes
any open locked conversation. Typing the code into ordinary search remains
an alternative way in. Revealed locked chats are currently read-only:
sending messages and forwarding into them remain disabled.

After linking or upgrading, chats wait up to ten seconds for WhatsApp's lock
state before appearing. Chats already known to be locked stay hidden. If the
lock state cannot be confirmed in time, the chats appear with a notice that
chats locked on the phone may show until they sync, and recovery keeps retrying
in the background. The recovered state is saved in the encrypted archive for
offline use.

Protocol logs omit private payloads and raw error details, including verbose
logging. Panic logs record the source location without the panic payload.
Pairing signature failures and rate limits retain a diagnostic category.

Offline previews for these states use `--demo --demo-page channel`,
`--demo --demo-page locked`, `--demo --demo-page locked-open`, and
`--demo --demo-page keyring`. The open locked-folder preview uses `demo-code`.
Use `--demo-page locked-prompt`, `locked-setup`, `new-chat`, `unnamed-group`,
or `react-picker` for the new dialogs, shared group summaries, and reactions.
`group-info`, `group-info-rename`, `group-info-saving`, and `group-info-locked`
show a group's info with its name and photo editable, being renamed, saving,
and locked to admins. `meta-ai` shows a Meta AI reply with code and a table.

The protocol dependency includes the upstream WhatsApp Business pairing fix.
Device-store migration waits until an updated window is acknowledged, preserving
startup rollback; an unused legacy column is retained for 0.14 compatibility.

### Interactive messages

Business messages keep their image, formatted text, timestamp, and options
together in one bubble. Reply buttons immediately send the selected response,
quoting the original message so the business can recognize your choice. Simple
list buttons open a centered dialog with sections, descriptions, and a full-row
selection target; choosing an item sends that response. Hover highlights
the full action row, following the card edges. Link buttons
open your browser, and copy-code buttons copy the offered code locally.

Carousels retain separate cards and images in a horizontally scrollable strip.
Each card can open web links or copy codes; reply, calling, and shopping actions
that require an unsupported carousel envelope stay unavailable.

Reply buttons require a connection and a writable conversation. They pause while
sending, and become available again if the send fails. Actions with a phone icon
are unavailable in ZapFast; use WhatsApp Web or your phone. Hovering explains
which restriction applies. Replies from other devices retain their quotes too.

| Text and reply options | Image and website link |
| --- | --- |
| ![Offline demo of an interactive text message with separate option rows and a quoted reply](docs/screenshot-interactive.png) | ![Offline demo of an interactive image message with working reply options and an active website link](docs/screenshot-interactive-media.png) |

![Offline demo with a reply button, a session list, a copy-code action, and an unavailable form](docs/screenshot-interactive-actions.png)

| Carousel cards | Poll participant details |
| --- | --- |
| ![Synthetic carousel with independent images, copy-code and web actions](docs/screenshot-carousel.png) | ![Synthetic poll results listing voters and vote times](docs/screenshot-poll-results.png) |

These screenshots use synthetic offline chats. See the
[usage guide](https://zapfast.rocks/using-zapfast/#interactive-messages) for
download behavior and the remaining limitations.
### Finding a setting

The search field at the top of **Settings** narrows the page to the settings
whose name or description contains what you type, ignoring case and accents,
and hides sections with nothing left. A match on a section's name keeps the
whole section. Translated settings are found in the interface language and in
English. `Ctrl+F` on the Settings page focuses the field, and `Esc` clears it.

### Interface language

**Settings > Appearance > Language** chooses the interface language. **Auto**
follows the first of the operating system's preferred languages that ZapFast
has a translation for, and falls back to English when it has none. Brazilian
Portuguese, German, Spanish, Italian, French, Russian, and Simplified Chinese
cover the chat list, search, composer, shortcut hints, Settings, and dates.
Translations are compiled from gettext PO
files at build time, with no runtime parsing or network access. Message
contents, contact names, logs, and protocol errors are never translated, and
copied messages keep WhatsApp's `[time, date] Name:` format.

### Proxy

**Settings > System > Proxy** sends the WhatsApp connection, media, profile
pictures, GIF search, Signal sticker imports, and update checks through a proxy. It accepts
`socks5h://host:port` (the proxy resolves names, as Tor expects),
`socks5://host:port`, and `http://host:port`, each with an optional
`user:password@`. A bare `host:port` is an HTTP proxy. Changing it reconnects
at once. When the field is empty, ZapFast uses `ALL_PROXY` or `HTTPS_PROXY`
from the environment and honors `NO_PROXY`.

## Files

| What | Linux | Notes |
| --- | --- | --- |
| Settings | `~/.config/zapfast/settings.json` | JSON, safe to edit |
| Device keys | `~/.local/state/zapfast/session.db` | Owned by whatsapp-rust; deleting it unlinks |
| Messages | `~/.local/state/zapfast/archive.db` | SQLCipher-encrypted SQLite, unlocked by the OS keyring; raw messages retain attachment keys |
| Attachments, avatars | `~/.cache/zapfast/` | Safe to delete; **Settings > Files > Change…** sends new downloads to another folder, leaving earlier ones in place |
| Favorite stickers and packs | `~/.local/state/zapfast/stickers/` | Plain WebP files; each pack is a folder |
| Wallpaper image | `~/.local/state/zapfast/wallpaper.jpg` | Copy of the chosen picture, or `.png`, `.webp`, `.gif`; deleted by **Remove image** |
| Transcription models | `~/.local/state/zapfast/whisper/` | Downloaded on the first transcription; **Delete models** in Settings, or `uninstall.sh --purge`, removes them |
| Log of the last run | `~/.local/state/zapfast/zapfast.log` | `--verbose` for more; **Settings > Files > Log > Open** shows it in its folder when no app opens it |

macOS and Windows use the standard platform directories selected by the
`directories` crate. On first start, ZapFast moves settings, the linked session,
message archive, favorite stickers, caches, and window state from `fastsapp`
(or the earlier `fastwhatsapp`) paths. Existing ZapFast directories take
precedence and are never overwritten. Quit FastsApp before starting ZapFast;
if an older copy is still running, the new launch brings its window forward.
Your phone may keep showing the old linked-device name until you link again.

On Linux and macOS, ZapFast restricts its configuration, state, and cache
directories to the current user (`0700`), including existing installations.
Startup stops if those directories cannot be created or secured, before opening
logs or databases. Windows uses the permissions inherited from your user profile.

### Local themes

**Settings → Appearance → Theme** uses the same picker as Spotifast, with
Follow system, Light, Dark, and its Catppuccin, Catppuccin Latte, Nord, Ristretto,
Tokyo Night, Rose Pine, Rose Pine Moon, and Rose Pine Dawn palettes.
Choose **Open themes folder** below the picker to add
JSON palettes beside `settings.json`; **How to make a theme** opens
[the guide](https://zapfast.rocks/themes/) with every colour name. The
bundled palettes are written into the themes folder once, as ordinary files
to read or change; ZapFast never rewrites them, and a deleted one stays
deleted. For example:

```json
{"base":"dark","colors":{"accent":"#89b4fa","bubble_out":"#293954"}}
```

Unspecified colors inherit the light or dark base. Spotifast palettes also work:
chat backgrounds, bubbles, and links derive from their interface colors when not
specified. Color names match `Palette`
in `src/theme.rs`; use `#RRGGBB` or `#RRGGBBAA`. The last accepted palette is cached
in settings, so a missing or damaged theme file does not reset your appearance.
Linux watches the themes folder for changes without periodic repaints. On other
platforms, use `zapfast reload-themes` after editing. The command also works while
the window is closed and never launches a stopped app.

**Settings → Appearance → Wallpaper** offers **Theme** first, then WhatsApp's
light and dark wallpaper colours, with a live preview that shows exactly what
the chat will. Theme, the default, uses the active palette's chat colour, so a
local or Omarchy theme colours the conversation too and a theme change shows at
once. Settings from earlier versions that still had the old default (Beige, or
Black in dark mode) move to Theme once; a colour you chose stays. **Add
doodles** controls only the SVG layer, so disabling it leaves the selected
background colour in place; the doodles switch between dark and light lines to
stay visible on any colour. Light and dark selections are stored independently,
and the embedded SVG is rendered at its native size and repeated across the
conversation without stretching.

**Choose image…** on the same page uses a picture of your own instead, in light
and dark mode alike, filling the conversation and cropped from the centre
without stretching. ZapFast keeps its own copy as `wallpaper.jpg` (or `.png`,
`.webp`, `.gif`) in its state directory, so the original can move; a picture
larger than 2560 pixels on its long side is scaled down first. The image
replaces the colour and doodles, which return with **Remove image**, which also
deletes the copy. If the copy goes missing or cannot be read, the colour shows
instead.

On Omarchy, **Follow system** and **Omarchy** read the active desktop palette and
follow its changes in native, portable, and source builds, even without installed
hooks. Other desktops keep their normal light/dark system preference. Native
packages additionally register a missing per-user template and theme hook on
first launch; existing user files are preserved. Flatpak uses the desktop's
light/dark preference and does not read host theme files or install desktop hooks.

### Updating ZapFast

ZapFast checks GitHub once a day when **Check for updates** is enabled.
Click **Update** in the banner to download and verify a newer release, then
**Restart to update** when convenient. **Download updates automatically** is
optional and off by default; it downloads in the background and still waits for
you to restart. Downloads contact GitHub's API and release-asset hosts and are
checked against the release's SHA-256 checksums. Before downloading a package,
the updater verifies the checksum manifest's Ed25519 publisher signature using
its embedded public key. Missing or invalid signatures stop the update.
The updater keeps a backup and restores it if the updated app cannot start;
its helper writes what it did to `helper.log` in the update's staging folder
beside the app.
Release builds also carry GitHub provenance attestations, independently
verifiable with `gh attestation verify FILE -R crmne/zapfast`.
See [update signing](packaging/UPDATE_SIGNING.md) for key custody and recovery.

The in-app updater supports marked portable downloads, the Windows installer,
and the macOS app in Applications. Keep `zapfast-portable.txt` beside a portable
executable. AUR, DEB, RPM, Flatpak, Cargo and Homebrew installations use their
package manager. Older portable downloads without the marker need one manual
upgrade. No account or additional service is needed.

## Developing

```sh
cargo run --features demo -- --demo            # sample chats, no connection
cargo run --features demo -- --demo-page login # or settings, pair, info, light, …
cargo run --features demo -- --demo-shot shot.png --demo-page chat,light
cargo run --features demo -- --demo-tour      # Space starts/replays a 41-second tour
cargo run --features demo -- --demo-tour --demo-tour-script whats-new # what 0.16 added
cargo run --features demo -- --demo-hover 900,400 # holds a fake pointer there
cargo test --all-features                      # includes a headless layout of every screen
cargo clippy --all-targets --all-features -- -D warnings
```

To include a default GIPHY key for GIF search, set it at build time. A key in
Settings overrides it:

```sh
ZAPFAST_GIPHY_KEY=your-key cargo build --release
```

The earlier `FASTSAPP_GIPHY_KEY` build variable remains supported as a fallback.

`AGENTS.md` describes the architecture and the rules for changes.
CI checks the complete lockfile against RustSec advisories with `cargo audit`.
Candidate-specific manual checks and results are tracked in the release PR.

### Recording a demo

The `demo` feature uses offline sample chats in a fresh temporary directory.
It does not open your linked account, read your message archive, connect to
WhatsApp, or register a tray icon. You can run it alongside your regular app.

```sh
cargo build --locked --features demo
./target/debug/zapfast --demo-tour --demo-size 1280x800
```

The **ZapFast Demo** window waits for **Space**. The 41-second tour starts with
search, switches chats with keyboard shortcuts, scrolls, right-clicks a message
and selects Reply, types quickly, completes emoji and mentions, searches the GIF
picker and sends a still sticker, opens group information and the shortcut list,
and changes themes through Settings. It uses the normal mouse and keyboard handlers;
a local responder handles outgoing messages with no WhatsApp connection.
The GIF-search thumbnails and still stickers are rendered from the bundled
Noto emoji font; demo GIF search uses these local fixtures. The tour makes no
sound and holds its final frame. Space rebuilds the sample and replays.
For an automatic start, add `--demo-tour-delay 5000` (milliseconds).
Use `--demo` instead of `--demo-tour` to explore the sample chats yourself.

`--demo-tour-script whats-new` plays an 86-second tour of what ZapFast 0.16
added instead: the composer's plus menu and poll dialog, searching a chat and
narrowing it to a day, the photo preview, videos and round video messages
playing in place, sticker shelves and sticker search, message info in a group,
the Favorites and label chips and a chat's menu, recording a voice message and
choosing a playback speed, the chat list folded to avatars, hover controls,
Ctrl-click and Shift-click selection with Forward, and Settings (languages,
search, and the light theme). `--demo-tour-script launch` is the default.
Demo runs never open the microphone: recording plays back a synthetic tone.
Use `--demo-page rtl-self` for a self-chat of mixed Hebrew, Arabic, and
English lines.
Use `--demo-page composer-tools` to preview the WhatsApp-style composer pill
and its attachment and poll menu. `typing`, `mention`, and
`emoji-complete` preview the multiline field and inline suggestions.
Use `--demo-page chat-menu` to preview the compact chat context menu,
`--demo-page chat-header-menu` for the menu at the top of an open chat, and
`--demo-page chat,voice,voice-menu` for a voice message's menu with its speeds.
`--demo-page video` shows a video and round video messages, and
`video-playing` or `note-playing` starts one of them, silently.
For deterministic theme screenshots, `--demo-page settings,omarchy` and
`--demo-page settings,omarchy-light` preview following dark and light Omarchy
palettes without changing the desktop theme.

Use `--demo-page shared-contact` for an offline shared-contact card with synthetic
vCard data, or `--demo-page interactive` for text and button messages, or
`--demo-page interactive-media` for messages with an image, and
`--demo-page interactive-list` for a list message,
`--demo-page interactive-list-dialog` for its grouped choice dialog, `--demo-page carousel`
for a scrolling strip or `--demo-page carousel-pair` for two cards, and `--demo-page poll-empty`, `poll-voted`, or `poll-results`
for voting states. Use `--demo-page interactive-actions` for reply, list, copy-code, and unavailable
form actions. Add `,light` to
preview any of these in the light theme. Capture the app's own frame without desktop
content:

```sh
./target/debug/zapfast --demo --demo-page interactive-media --demo-shot interactive.png
./target/debug/zapfast --demo --demo-page interactive-media,light --demo-shot interactive-light.png
```

On Omarchy, run `omarchy screenrecord`, select the demo window, then press Space
in ZapFast. Recording has no audio unless you explicitly enable desktop or
microphone audio. Stop with `omarchy screenrecord --stop-recording` after the
tour finishes. The default capture records a fixed rectangle, so keep the demo
window visible and stationary until recording stops.

To annotate the video with a visible pointer, click rings, and outlined shortcut
labels, add `--demo-tour-events tour.json` when launching the tour. After
recording, run:

```sh
python3 scripts/render-demo.py recording.mp4 tour.json launch.mp4 --start 0.8
```

Set `--start` to the recording time (in seconds) when you pressed Space. The
export trims the setup footage, adds a caption band below the app, and produces
a silent H.264 MP4. It requires `ffmpeg` with libass support and `ffprobe`.
`--scale 1.5` keeps 1.5 pixels per point, for example 1920 pixels across from a
1280-point window recorded at 2x; the default is one pixel per point.

These annotations are added during video export, not drawn by the app. The
trace contains only pointer coordinates and shortcut labels, not typed text.

Instead of recording the screen, the tour can save its own frames. With
`--demo-tour-frames DIR`, it starts at once, plays on a virtual clock (steady
frame times even when a frame is slow to draw), writes every frame as a PNG
at the window's pixel size, and quits when the tour ends. `--demo-fps` sets the
rate (30 by default). The window still has to be shown somewhere; a virtual
output keeps it off your screens. Then assemble and annotate the frames:

```sh
cargo build --release --locked --features demo
./target/release/zapfast --demo-tour --demo-tour-script whats-new \
  --demo-size 1280x800 --demo-tour-frames frames --demo-tour-events tour.json
ffmpeg -framerate 30 -i frames/frame-%05d.png -c:v libx264 -crf 12 -pix_fmt yuv420p raw.mp4
python3 scripts/render-demo.py raw.mp4 tour.json whats-new.mp4 --scale 1.5
```

## Disclaimer

ZapFast is an unofficial client and is not affiliated with WhatsApp or
Meta. Using an unofficial client may be against WhatsApp's terms of service
and could get an account suspended. Use it at your own risk.

## Packaging maintenance

Release packaging uses the [native-packages](https://rubygems.org/gems/native-packages) gem. macOS release builds automatically sign and notarize when the Apple CI credentials are configured. `native-packages.yaml` declares packages and downstream repositories; native recipes and installation assets live in `packaging/`; see [PACKAGING.md](PACKAGING.md) for local commands and CI behavior.

## License

MIT. Inter and Noto Color Emoji are under the SIL Open Font License; the icons
and the chat wallpaper doodles are from [Lucide](https://lucide.dev) (ISC).
The notification sounds are [Pidgin](https://pidgin.im)'s, under the GPL-2.0
(see `assets/sounds/`).
