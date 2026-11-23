import { Channel, invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { codecFromAvcC } from "../utils/codec";

type Props = { monitorId: number };

// Primeiro byte de cada mensagem vinda do Rust (ver src-tauri/src/streaming.rs):
// 0 = delta, 1 = keyframe, 2 = config (avcC).
const KIND_KEY = 1;
const KIND_CONFIG = 2;

export const ScreenShareView = ({ monitorId }: Props) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [hasFrame, setHasFrame] = useState(false);

  useEffect(() => {
    const canvas = canvasRef.current!;
    const ctx = canvas.getContext("2d")!;
    let canceled = false;

    setHasFrame(false);

    const decoder = new VideoDecoder({
      output: (frame) => {
        if (
          canvas.width !== frame.displayWidth ||
          canvas.height !== frame.displayHeight
        ) {
          canvas.width = frame.displayWidth;
          canvas.height = frame.displayHeight;
        }
        ctx.drawImage(frame, 0, 0);
        frame.close();
        setHasFrame(true);
      },
      error: (e) => console.error("[share] decoder error:", e),
    });

    const t0 = performance.now();
    let gotKey = false;

    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buf) => {
      if (canceled) return;

      const data = new Uint8Array(buf);
      const kind = data[0];

      // No Linux o WebKitGTK (GStreamer) só aceita H.264 em AVCC e exige o
      // avcC em `description`; Annex-B puro nem negocia. O Rust manda a config
      // logo antes do primeiro keyframe.
      if (kind === KIND_CONFIG) {
        const description = data.subarray(1);
        const codec = codecFromAvcC(description);
        if (!codec) {
          console.error("[share] avcC inválido:", description);
          return;
        }
        try {
          decoder.configure({ codec, description, optimizeForLatency: true });
          console.log(
            "[share] decoder configurado:",
            codec,
            `(${description.length} bytes avcC)`,
          );
        } catch (e) {
          console.error("[share] configure falhou:", codec, e);
        }
        return;
      }

      if (decoder.state !== "configured") return;

      const isKey = kind === KIND_KEY;
      if (!gotKey && !isKey) return;

      // backpressure: fila grande = decoder atrasado, descarta delta e espera keyframe
      if (decoder.decodeQueueSize > 3 && !isKey) {
        gotKey = false;
        return;
      }

      try {
        decoder.decode(
          new EncodedVideoChunk({
            type: isKey ? "key" : "delta",
            timestamp: Math.round((performance.now() - t0) * 1000),
            data: data.subarray(1),
          }),
        );

        if (isKey) {
          gotKey = true;
        }
      } catch (e) {
        console.error("[share] decode falhou:", e);
        gotKey = false;
      }
    };

    // precisa ficar fora do onmessage: o canal só recebe depois que o Rust começa a enviar
    invoke("start_share", {
      config: { monitor_id: monitorId },
      onChunk: channel,
    }).catch((e) => console.error("[share] start_share falhou:", e));

    return () => {
      canceled = true;
      invoke("stop_share").catch((e) =>
        console.error("[share] stop_share falhou:", e),
      );
      if (decoder.state !== "closed") decoder.close();
    };
  }, [monitorId]);

  return (
    <div className="relative flex min-h-0 flex-1 items-center justify-center">
      <canvas className="max-h-full max-w-full bg-black" ref={canvasRef} />
      {!hasFrame && (
        <p className="absolute inset-0 grid place-items-center text-sm text-zinc-400">
          Conectando…
        </p>
      )}
    </div>
  );
};
