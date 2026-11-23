# Compartilhamento de tela — camada de rede

Data: 2026-11-22

## Contexto

O app hoje é um Tauri com captura via `xcap`, conversão de cor em Rust e
encoder H.264 (OpenH264 em software, com caminho NVENC/AMF/VideoToolbox via
FFmpeg). O encoder produz Annex-B; `streaming.rs` converte para AVCC, extrai
o `avcC` e envia tudo para o webview local por um `Channel` do Tauri, onde o
WebCodecs do WebKitGTK decodifica.

Ou seja: hoje existe um único consumidor, o próprio webview do host, e
nenhuma rede.

O objetivo é transmitir para outros Tauri em outras redes, passando por um
relay próprio, com o conteúdo cifrado ponta a ponta, suportando entrada no
meio do fluxo e degradação independente por espectador.

## Objetivos

- Um host transmite para ~5 espectadores simultâneos, todos em apps Tauri.
- O relay não consegue ler a tela.
- Quem entra no meio do fluxo vê imagem em menos de ~2 s.
- Rede ruim de um espectador não degrada a experiência dos outros.
- Entrada por link, com modo público ou privado, trocável ao vivo.

## Não-objetivos (fase 1)

- Áudio (fase 2; o protocolo reserva espaço desde já).
- Expulsar espectador (revogação é girar o link ou encerrar a transmissão).
- Espectador em navegador.
- Gravação.
- Contas de usuário.
- Transcodificação no relay.

## Decisões

### 1. Relay próprio, burro

Host e espectadores abrem conexão **de saída** para o relay. Nenhum dos dois
lados abre porta, e não há ICE/STUN. O relay autentica o host (a identidade
de quem transmite não é segredo); espectadores são anônimos.

O relay faz três coisas e só: repassa datagramas, mantém cache de config e
do último IDR por camada, e escolhe a camada de cada espectador. Ele nunca
decifra mídia nem controle.

### 2. Transporte: QUIC

Uma conexão QUIC por participante, com:

- **datagramas** para mídia — frame perdido é frame pulado, sem
  retransmissão e sem travar os demais;
- **um stream confiável** para controle (entrada, aprovação, troca de
  camada, pedido de keyframe).

Por quê: o requisito é pular frame antes de travar, e isso só é garantido
quando o transporte não transforma perda em fila. TCP faria o buffer do
kernel e do relay virarem latência. QUIC também evita head-of-line entre
mídia e controle, e traz TLS 1.3 embutido.

### 3. Formato no fio: AVCC

Annex-B existe apenas dentro do remetente, entre a saída do encoder e a
conversão. **O formato do fio é AVCC** — o mesmo que já vai para o webview
local hoje.

Por quê:

- fronteiras de NAL explícitas (prefixo de tamanho), então fragmentar em
  datagramas não exige varrer start codes nem tratar start code partido na
  borda do datagrama;
- o espectador entrega o payload direto ao WebCodecs, sem conversão;
- **o mesmo payload serve o preview local e a rede**, eliminando a classe de
  bug em que o preview funciona e o remoto não.

Consequência aceita: SPS/PPS não vão in-band. A config (`avcC`) é uma
mensagem própria, reenviada a cada keyframe em que mudar, e o relay a
cacheia junto do último IDR. É isso que sustenta a entrada no meio do fluxo,
não o formato.

### 4. Duas camadas

O remetente codifica duas camadas em paralelo: **1080p (cheia) e 540p
(meia)**, a ~4 Mbps e ~1,5 Mbps. Nunca se adapta ao pior espectador
globalmente.

Consequência no código atual: `conversions.rs:15` tem `TARGET_HEIGHT = 720`
fixo. Ele precisa virar parâmetro por camada, e o pipeline precisa de dois
`Converter` e dois encoders alimentados pelo mesmo frame de captura.

**Sinal de troca:** datagrama QUIC não forma fila observável como TCP — ele
é descartado quando a janela de congestionamento enche. Então o sinal
primário é **feedback do espectador** pelo stream de controle: frames
recebidos, frames decodificados, perda estimada e jitter. `quinn` fornece
estatísticas de caminho (RTT, perda) como sinal secundário. O relay agrega
por espectador e decide; a troca acontece sempre em fronteira de IDR.

**Degradação para uma camada:** se só houver uma sessão de encoder
disponível (limite de NVENC em placa de consumidor, ou CPU sem folga), o
remetente sobe **uma** camada e o protocolo aceita "camada 1 ausente". Isso
é modo suportado, não falha: o relay simplesmente não oferece a camada que
não existe.

Opcional dentro de cada camada: afinar o fps quando algum assinante daquela
camada está sofrendo. Isso afeta só o grupo que já está naquele degrau.

### 5. Header em claro, payload cifrado

O relay precisa rotear e cachear sem ler a tela. Então cada datagrama tem um
header em claro e um payload cifrado autenticado com o header como AAD —
o relay não consegue alterar `kind`, sequência ou timestamp sem invalidar o
payload.

Rascunho do header (números a fechar no plano):

| campo | tamanho | observação |
|---|---|---|
| versão | u8 | |
| tipo | u8 | 0 delta, 1 key, 2 config |
| camada | u8 | 0 cheia, 1 meia |
| flags | u8 | bit 0 = último fragmento |
| sessão | u32 | |
| sequência do frame | u32 | |
| índice do fragmento | u16 | |
| total de fragmentos | u16 | |
| timestamp | u32 | 90 kHz |

Alvo: datagrama de mídia com no máximo ~1200 bytes de payload.

### 6. Criptografia

- Link de convite com o **segredo no fragmento da URL**
  (`https://relay/s/<id>#<segredo>`), que nunca chega ao servidor.
- O segredo **autentica** a chave, não é a chave: handshake Noise com PSK
  (`snow`), ECDH por espectador. Tráfego gravado antes de um vazamento de
  link continua inútil.
- **O plano de controle também é ponta a ponta**, entre espectador e host:
  `join_request` com o mini username, `join_decision` e entrega da chave de
  grupo viajam cifrados, e o relay só repassa bytes opacos. O relay nunca vê
  usernames e não consegue forjar uma aprovação.
- Chave de grupo por transmissão; o host embrulha essa chave com a pública
  de cada espectador aprovado e manda pelo relay.
- **Sem rotação na entrada.** A chave de grupo vale para a transmissão
  inteira. Rotacionar a cada entrada invalidaria o cache de config e IDR que
  o relay guarda (eles estão cifrados com a chave vigente) e obrigaria um
  keyframe novo mais reembrulho para todos — caro e sem ganho, porque
  excluir quem já tem a chave é expulsão, que está fora da fase 1.
- **Nonce:** um contador global por transmissão, compartilhado pelas duas
  camadas, e a mídia entra no nonce. Sequência por camada com a mesma chave
  seria reuso de nonce, que quebra confidencialidade e integridade.

**Modelo de ameaça:** o relay é honesto-mas-curioso. Ele não decifra mídia
nem controle, não forja aprovação e não altera header de mídia (o header é
AAD do payload). O que ele ainda pode: negar serviço, descartar tráfego,
forçar um espectador para a camada de baixo, e observar metadados — quem
fala com quem, quando, por quanto tempo e com que bitrate. Resistir a um
relay malicioso nesses pontos não está no escopo.

### 7. Entrada

O espectador abre o link, digita um mini username para aquela transmissão, e
o relay segura a conexão em "pendente".

- **Modo público**: o app do host aprova sozinho.
- **Modo privado**: o host vê o mini username e clica para permitir.

Aprovado, o host embrulha a chave e a mídia começa. O modo é por transmissão
e pode mudar ao vivo: quem já está dentro fica, quem chega depois segue a
regra nova.

**O modo é do host, não do relay.** O relay nunca sabe se a transmissão é
pública ou privada; ele só segura conexões pendentes e repassa controle. A
admissão é sempre decidida no app do host, inclusive no modo público
(aprovação automática). Isso tira do relay qualquer poder de admitir alguém
por conta própria.

**Como o link chega no app:** o link é `https://`, porque link de app
customizado não é clicável na maioria dos mensageiros. O relay serve uma
página mínima que lê o fragmento no cliente e redireciona para
`screenshare://join#<segredo>`, esquema registrado pelo Tauri. O segredo
nunca sai do cliente, então não aparece em log nem em histórico do servidor.

### 8. Entrada no meio do fluxo

Três peças:

1. IDR periódico — hoje o GOP é de 60 frames (~2 s a 30 fps), que é o pior
   caso de espera.
2. Cache no relay do `avcC` e do último IDR de cada camada, entregues ao
   novo espectador antes do fluxo ao vivo.
3. Pedido de keyframe: `request_keyframe()` já existe nos dois encoders
   (`encoder.rs:32`) e hoje nunca é chamado. Um pedido de qualquer
   espectador força um IDR para todos, o que com GOP de 2 s custa pouco.

### 9. Host, sessão e operação

- **Identidade do host:** par de chaves por instalação, gerado no primeiro
  uso e registrado no relay com um token de inscrição de uso único, gerado
  pelo operador. Sem contas, sem senha. A identidade é estável entre
  transmissões, o que é conveniente mas não é requisito.
- **Id de sessão:** 128 bits aleatórios, não sequencial. O relay limita
  taxa de conexões por IP e por sessão, para que "pendente" não vire vetor
  de esgotamento.
- **O relay é Rust**, no mesmo repositório, como membro de workspace, com um
  crate de protocolo compartilhado com o app (header, tipos de controle,
  cripto). Evita duas definições do mesmo header divergindo.

## Fluxos

**Host inicia**: autentica no relay, cria a sessão, gera o link com segredo,
começa a codificar as duas camadas e a enviar datagramas + config.

**Espectador entra**: abre o link, handshake Noise, `join_request` com o
mini username. Público → `join_decision` automática. Privado → o host
decide. Aprovado → recebe config + IDR em cache, assina uma camada, e entra
no ao vivo.

**Troca de camada**: o relay vê no feedback do espectador que a recepção
está piorando, anuncia a troca, espera o próximo IDR da camada de destino e
passa a repassar aquela camada.

**Troca de modo ao vivo**: acontece só no app do host; o relay não participa.
Vale apenas para quem chegar depois, porque quem já está dentro não é
reavaliado.

**Host encerra**: relay fecha as conexões dos espectadores.

## Fase 2: áudio

Opus, frames de 20 ms (~48 kbps), um stream só para todos — áudio não entra
no simulcast. Mesma chave de grupo, nonce separado. Usar PLC e FEC in-band,
e considerar redundância do frame anterior, porque perda de áudio vira buraco
audível e não artefato.

A captura é a peça de maior risco: `xcap` não captura áudio; no Linux é o
monitor do PipeWire, no Windows é WASAPI loopback. Cada plataforma tem
caminho próprio.

Decisão pendente para a fase 2: decodificar áudio no webview (junto do
vídeo, um relógio só) ou no Rust (dois relógios, precisa de sincronia por
timestamp). Depende de o WebKitGTK ter `AudioDecoder` no WebCodecs —
verificar, não supor.

## O que fica com o construtor

- Layout final do header e tamanho de fragmento.
- Formato de serialização das mensagens de controle.
- Política exata de fila e de troca de camada no relay.
- Reconexão e tratamento de queda do relay.
- Onde o relay roda e como é feito o deploy/TLS.

## Riscos

- **Encode em software de duas camadas a 1080p30 é o caminho crítico.** O
  `encoder.rs` hoje só devolve OpenH264; o caminho FFmpeg/NVENC está
  comentado. Medir isso cedo: se não couber, a saída é restaurar o caminho de
  GPU ou cair para uma camada.
- Captura de áudio cross-platform (fase 2).
- Limite de sessões NVENC em placa de consumidor (2 a 8 conforme driver);
  duas camadas cabem, três não são garantidas.
- Egress do relay escala com o número de espectadores (~4 Mbps × N na
  camada cheia, ~1,5 Mbps na meia).
- WebKitGTK e WebCodecs de áudio (fase 2).
