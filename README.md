# screen-sharing-rust

Compartilhamento de tela em tempo real dentro de um app Tauri: a captura roda no Rust, o encode é por hardware e o decode é feito pelo WebCodecs no WebView. Hoje o vídeo não sai do processo; o objetivo é que ele saia cifrado, por um relay que não consegue ler a tela.

## O produto

- Um host transmite para **~5 espectadores simultâneos**, todos em apps Tauri.
- **O relay não consegue ler a tela** — cifra ponta a ponta.
- Quem entra no meio do fluxo vê imagem em **menos de ~2 s**.
- A rede ruim de um espectador **não degrada** a experiência dos outros.
- Entrada por link, com modo **público ou privado**, trocável ao vivo.

Não-objetivos da fase 1: áudio (fase 2), expulsar espectador, espectador em navegador, gravação, contas de usuário e transcodificação no relay.

## A ideia

Quatro decisões guiam o projeto:

1. **Um payload, dois destinos.** O formato que vai para a rede é o mesmo que vai para o preview local: AVCC, com fronteiras de NAL explícitas. Isso elimina a classe de bug em que o preview funciona e o remoto não.
2. **O relay é burro por design.** Ele repassa datagramas, cacheia config e o último IDR por camada, e escolhe a camada de cada espectador. Nunca decifra mídia nem controle, e nunca decide quem entra.
3. **Hardware primeiro, CPU como rede de segurança.** O encoder tenta `h264_nvenc` → `h264_amf` → `h264_videotoolbox` e só cai para `openh264` (CPU) se nenhum abrir. O caminho de GPU usa CBR, `zerolatency`, preset `p1` e GOP de 60 frames.
4. **Medir, não adivinhar.** Cada segundo o backend imprime quanto custou cada estágio e *quem* está limitando a taxa. Sem isso, "está lento" é opinião; com isso, é um número.

## Pipeline local

```
┌─ captura (xcap) ──────────────────────────────────────┐
│  Wayland: PipeWire via xdg-desktop-portal             │
│  X11: XGetImage                                       │
│  frame BGRx, 1920×1080 (8,3 MB)                       │
└────────────────────────┬──────────────────────────────┘
                         │ mpsc::Receiver<Frame>
                         ▼
              take_latest()  ── descarta o que sobrou da fila
                         ▼
┌─ conversions ─────────────────────────────────────────┐
│  resize (fast_image_resize, rayon) → 1280×720         │
│  BGRx → YUV420P planar (bt601, limited)               │
└────────────────────────┬──────────────────────────────┘
                         ▼
┌─ encoder (ffmpeg-next) ───────────────────────────────┐
│  h264_nvenc · CBR · zerolatency · GOP 60 · baseline   │
└────────────────────────┬──────────────────────────────┘
                         ▼
┌─ bitstream + packager ────────────────────────────────┐
│  Annex-B → Access Units → AVCC                        │
│  avcC emitido só quando muda                          │
└────────────────────────┬──────────────────────────────┘
                         ▼
        Tauri Channel  [kind][payload]
                         ▼
┌─ frontend ────────────────────────────────────────────┐
│  VideoDecoder (WebCodecs) → ctx.drawImage → canvas    │
└───────────────────────────────────────────────────────┘

        ┄┄┄ (fase 2) mesmo payload AVCC → relay → QUIC ┄┄┄
```

O resize existe porque a fonte raramente tem o tamanho do alvo: 1920×1080 → 1280×720 é uma redução de 2,25x em pixels. Quando a fonte já é o alvo, o resize é pulado (`conversions.rs`).

## Contrato de fio

Toda mensagem do Rust para o WebView começa com um byte que diz o que vem depois. É o mesmo contrato lido em `src/components/ScreenShareView.tsx` — e o mesmo `kind` que vai reaparecer no header da rede.

| byte 0 | payload | quando |
|---|---|---|
| `0` | Access Unit em AVCC de um frame delta | a cada frame |
| `1` | Access Unit em AVCC de um keyframe | a cada IDR |
| `2` | `avcC` (AVCDecoderConfigurationRecord) | antes do primeiro keyframe, e quando muda |

O WebKitGTK só aceita H.264 em AVCC com `description` no `configure` — Annex-B puro não negocia. Por isso o packager converte e o frontend deriva o codec (`avc1.PPCCLL`) dos bytes 1..3 do `avcC`.

## Breakpoints de qualidade

Um **breakpoint** não é uma opção de menu: é um orçamento validado. Para um par (resolução, fps) valer como breakpoint, três coisas precisam concordar:

1. **A captura entrega essa taxa.** No Wayland/GNOME a captura é *damage-driven*: o compositor só emite frame quando a tela muda. O fps é uma propriedade do conteúdo, não do pipeline. Num vídeo de 60 fps a captura dá ~60; numa tela parada ela dá ~2.
2. **O encoder declara essa taxa.** O CBR divide o bitrate por `fps` para achar o orçamento de cada frame. Se o encoder declara 30 e a captura entrega 60, o stream gasta **o dobro** do bitrate configurado.
3. **O custo por frame cabe no orçamento com folga.** Regra da casa: ocupação abaixo de 50%, para que picos de keyframe e rajadas não derrubem a taxa.

Com fonte de 1920×1080 (o painel do notebook), os quatro breakpoints do produto:

| Breakpoint | Alvo | Pixels | Bitrate | Orçamento/frame | Custo por frame | Ocupação | Status |
|---|---|---|---|---|---|---|---|
| **720p30** | 1280×720 @ 30 | 0,92 M | 2,5 Mbps | 33,3 ms | 2,7 ms | ~8% | ✅ alcançado |
| **720p60** | 1280×720 @ 60 | 0,92 M | 5,0 Mbps | 16,7 ms | 2,7 ms | ~16% | ✅ medido |
| **1080p30** | 1920×1080 @ 30 | 2,07 M | 5,6 Mbps | 33,3 ms | ~2,4 ms ¹ | ~7% | ⏳ pendente |
| **1080p60** | 1920×1080 @ 60 | 2,07 M | 8–11 Mbps | 16,7 ms | ~2,4 ms ¹ | ~15% | ⏳ pendente |

¹ na fonte 1080p o resize é pulado, então 1080p custa *menos* por frame que 720p — só o YUV e o encode crescem.

O bitrate segue a regra `≈ 0,09 bpp × pixels × fps` (a 2,5 Mbps / 720p30). Conteúdo de tela comprime entre 300:1 e 500:1, então os valores de 1080p são teto, não piso: 8 Mbps já entrega 1080p60 com folga.

### O que falta em cada um

- **720p30 — ✅ alcançado.** É o modo atual: `TARGET_HEIGHT = 720`, encoder declarado em 30 fps. A captura entrega 47–60, e é justamente essa divergência que o item 2 acima proíbe (ver *Limitações*).
- **720p60 — ✅ medido.** Sustentado em 58–62 fps com o preview **fora** do monitor capturado. Falta ligar o seletor de qualidade ao backend e amarrar o `fps` do encoder à taxa real.
- **1080p30 — ⏳ pendente.** Só exige `TARGET_HEIGHT = 1080` e o mesmo wiring. É o breakpoint mais barato de entregar — e é o que a camada de rede usa.
- **1080p60 — ⏳ pendente, o mais apertado.** No eDP-1 a captura tem teto medido de ~75 fps (pico de 75,6), ou seja ~25% de margem sobre 60. No ultrawide (3440×1440) o readback do compositor cresce 2,4x e o teto cai para 30–45 fps: **1080p60 não é viável nessa fonte** com o caminho atual. É breakpoint de preview local; a rede fica em 30 fps.

## Camada de rede

O design está fechado em [`docs/superpowers/specs/2026-11-22-camada-de-rede-design.md`](docs/superpowers/specs/2026-11-22-camada-de-rede-design.md). O resumo:

**Relay próprio e burro.** Host e espectadores abrem conexão **de saída** para o relay; ninguém abre porta, não há ICE/STUN. O relay autentica o host (a identidade de quem transmite não é segredo) e trata espectadores como anônimos.

**QUIC.** Uma conexão por participante: **datagramas** para mídia — frame perdido é frame pulado, sem retransmissão e sem head-of-line bloqueando os demais — e **um stream confiável** para controle (entrada, aprovação, troca de camada, pedido de keyframe). TLS 1.3 embutido.

**AVCC no fio.** Annex-B existe apenas dentro do remetente. Fronteiras de NAL explícitas significam que fragmentar em datagramas não exige varrer start codes nem tratar start code partido na borda.

**Duas camadas.** O remetente codifica **1080p (~4 Mbps)** e **540p (~1,5 Mbps)** em paralelo. Ninguém é rebaixado por causa do pior espectador: o relay decide **por espectador**, com feedback de frames recebidos/decodificados, perda e jitter, e a troca acontece sempre em fronteira de IDR. Se só houver uma sessão de encoder (limite de NVENC em placa de consumidor), sobe uma camada só — modo suportado, não falha.

| Camada | Resolução | Bitrate | Papel |
|---|---|---|---|
| cheia | 1920×1080 @ 30 | ~4 Mbps | padrão |
| meia | 960×540 @ 30 | ~1,5 Mbps | rede apertada |

Os 4 Mbps da camada cheia ficam **abaixo** do teto de 5,6 Mbps do breakpoint 1080p30 porque o egress do relay soma: ~4 Mbps × N espectadores.

**Header em claro, payload cifrado.** O relay precisa rotear e cachear sem ler a tela. Cada datagrama leva versão, tipo (`0`/`1`/`2`), camada, flags, sessão, sequência de frame, índice e total de fragmentos, e timestamp de 90 kHz — ~20 bytes em claro — e o payload autenticado com esse header como AAD, para que ninguém altere `kind`, sequência ou timestamp sem invalidar o payload. Datagrama de mídia: ≤ 1200 bytes de payload.

**Entrada.** O link é `https://relay/s/<id>#<segredo>`: o segredo vive no fragmento da URL, nunca chega ao servidor, e uma página mínima redireciona para `screenshare://join#<segredo>`. Handshake Noise com PSK (`snow`) + ECDH por espectador — o segredo **autentica** a chave, não é a chave, então tráfego gravado antes de um vazamento de link continua inútil. Chave de grupo por transmissão, embrulhada com a pública de cada espectador aprovado.

O espectador digita um mini username; **público** = o host aprova sozinho, **privado** = o host clica. O modo é do host e trocável ao vivo — o relay nunca sabe qual é, e não tem poder de admitir ninguém por conta própria.

**Entrada no meio do fluxo.** Três peças: IDR periódico, cache de `avcC` + último IDR por camada no relay, e `request_keyframe()` — que já existe nos dois encoders (`encoder.rs:32`) e hoje nunca é chamado.

**Modelo de ameaça.** Relay honesto-mas-curioso: não decifra mídia nem controle, não forja aprovação e não altera header. O que ele ainda pode: negar serviço, descartar tráfego, forçar um espectador para a camada de baixo e observar metadados (quem fala com quem, quando, por quanto tempo, com que bitrate).

## Como medir

Cada segundo, o backend imprime duas linhas. A primeira é o throughput; a segunda é o diagnóstico.

```
fps: 58.4 | bitrate: 4446 kbps | resize: 1.7 ms | yuv: 0.4 ms | encode: 0.5 ms | compressão: 387:1 | ocupação: 15% | skip: 2 | drop: 0
  1280x720 | espera p50 13.1 p95 30.4 max 40.7 | proc p50 2.7 p95 3.9 max 6.6 | send p50 0.03 p95 0.05 | parse 0.01 pack 0.00 | pkt/frame 1.02 | capacidade ~373 fps | gargalo: captura | espera<5 3 5-15 29 15-25 20 25-40 5 >40 1
```

| Campo | O que significa |
|---|---|
| `ocupação` | `(resize + yuv + encode) / janela`. É a utilização da thread de encode — **não** inclui espera, IPC nem packager |
| `espera p50` | tempo parado, bloqueado esperando o próximo frame |
| `proc p50` | tempo processando o frame, do `recv` até o `send` terminar |
| `send p50` | custo do IPC para o WebView |
| `capacidade` | `1000 / proc_p50` — quantos fps o loop faria se a captura não parasse |
| `gargalo` | `espera > proc` → **captura**; senão → **consumo** |
| `espera<5 … >40` | histograma da espera, em ms. Espera regular cai quase toda num bucket; espera aos trancos se espalha |
| `skip` | `send_frame` não devolveu pacote (pipeline assíncrono do NVENC) |
| `drop` | frames descartados por `take_latest` — o consumidor não acompanhou |

A leitura que importa: **`ocupação` baixa com `espera` alta não é desperdício, é o loop esperando frames que não existem.** Um caixa com 15% de ocupação não atenderia 6x mais clientes se só chega um cliente a cada 20 ms.

## Limitações conhecidas

1. **PTS fixo.** O encoder usa `time_base = 1/30` e `pts += 1`. Com a captura entregando 47–60 fps, o stream declara menos tempo do que a realidade, o bitrate sai 1,5–2x o configurado e o GOP de 60 frames vira 1,0–1,25 s em vez de 2 s. É o bug mais importante aberto — e é pré-requisito da camada de rede, que carrega timestamp de 90 kHz no header.
2. **Recuperação de keyframe inexistente.** `request_keyframe()` está no trait, mas nunca é chamado. Quando o decoder do frontend atrasa (`decodeQueueSize > 3`), o `ScreenShareView` descarta deltas e espera um IDR — que pode levar até um GOP inteiro. Resultado: congelamento de ~1 s em vez de perda de frame.
3. **Desenho no callback do decoder.** O `output` do `VideoDecoder` pinta direto no canvas. Quem decide quantos refreshes cada frame fica é o compositor, o que amplifica o judder de cadência (60 fps num painel de 143,88 Hz cai em 2-3-2-3 refreshes).
4. **Seletor de qualidade é cosmético.** `ShareConfig` só carrega `monitor_id`; `720p30`/`720p60`/`1080p60` não chegam ao backend.
5. **Uma camada só.** `TARGET_HEIGHT = 720` é fixo e há um `Converter` e um encoder. Duas camadas exigem dois `Converter` e dois encoders alimentados pelo mesmo frame de captura.
6. **Não há rede.** Sem relay, sem QUIC, sem cifra — o design existe, o código não.
7. **Teto de captura do GNOME 46.** O `xcap` usa o caminho memfd do PipeWire, e o `mutter` responde com `clutter_stage_paint_to_buffer` — repaint + readback de 8,3 MB por frame, ~6,4 ms de `gnome-shell`. O caminho dmabuf existe no `mutter`, mas exige um cliente PipeWire que negocie `SPA_FORMAT_VIDEO_modifier`; o `xcap` não negocia.
8. **Preview no monitor capturado fecha um loop.** Se a janela do app estiver na tela que está sendo capturada, o único motivo para a tela mudar é o próprio preview — e o fps passa a ser `1/latência do pipeline` (~47). Regra de ouro: **capture um monitor, mostre o preview em outro.**

## Como rodar

Pré-requisitos: Rust, Node/pnpm, e as libs de desenvolvimento do FFmpeg (`libavcodec`, `libavformat`, `libavutil`, `libswscale`).

```bash
pnpm install
pnpm tauri dev
```

Ao iniciar o compartilhamento, o GNOME abre o diálogo do portal (`xdg-desktop-portal`). Escolha o monitor. O `xcap` no Wayland usa o primeiro stream retornado pelo portal.

## Estrutura

```
src-tauri/src/
  streaming.rs      loop de captura→encode→envio, métricas e diagnóstico
  conversions.rs    resize + BGRx→YUV420P
  encoder.rs        trait VideoEncoder, EncoderConfig, seleção de backend
  encoder/ffmpeg.rs h264_nvenc / amf / videotoolbox
  encoder/h264.rs   openh264 (fallback CPU)
  bitstream.rs      Annex-B → Access Units
  packager.rs       avcC + AVCC, contrato de fio
  monitors.rs       listagem de monitores (xcap)
  commands.rs       start_share / stop_share / available_monitors

src/
  components/ShareScreen.tsx     seleção de monitor e qualidade
  components/ScreenShareView.tsx WebCodecs + canvas
  utils/codec.ts                 avcC → string avc1.PPCCLL

docs/superpowers/specs/
  2026-11-22-camada-de-rede-design.md   design da camada de rede
```

A camada de rede entra como membro de workspace: um crate de protocolo compartilhado (header, mensagens de controle, cripto) e um crate do relay, para que as duas pontas não tenham duas definições do mesmo header.

## Documentação

- [`docs/superpowers/specs/2026-11-22-camada-de-rede-design.md`](docs/superpowers/specs/2026-11-22-camada-de-rede-design.md) — fonte de verdade para a camada de rede: relay, QUIC, AVCC no fio, duas camadas, cifra, entrada, entrada no meio do fluxo e modelo de ameaça. Traz também o que fica em aberto para o construtor (layout final do header, serialização do controle, política de fila do relay, reconexão, deploy).

> Nota: o risco nº 1 daquele documento ("o `encoder.rs` hoje só devolve OpenH264; o caminho FFmpeg/NVENC está comentado") já não vale. O `build_encoder` tenta `h264_nvenc` primeiro, e o caminho foi medido em ~0,5 ms/frame a 720p. O documento não foi atualizado nesse ponto.

## Roadmap

**Fase 1 — local**

1. **PTS real** (`time_base = 1/1_000_000`, timestamp por frame) e `fps` do encoder amarrado à taxa real — conserta bitrate, timing e GOP de uma vez.
2. **Ligar o seletor de qualidade** ao `ShareConfig` e aos breakpoints acima.
3. **Recuperação de keyframe**: chamar `request_keyframe()` quando o decoder atrasar, em vez de congelar.
4. **Desenho em `requestAnimationFrame`** no frontend, para não delegar a cadência ao compositor.
5. **Captura dmabuf** (PipeWire direto, sem `xcap`) para subir o teto de captura e permitir 1080p60 no ultrawide.

**Fase 2 — rede**

6. Crate de protocolo + relay (QUIC/datagramas, header, cache de config/IDR).
7. Duas camadas (1080p/540p) e troca por espectador em fronteira de IDR.
8. Noise/PSK, chave de grupo e entrada por link com modo público/privado.
9. Áudio (Opus, 20 ms, ~48 kbps, mesma chave de grupo e nonce separado).
