import { Channel, invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react"

type Props = { monitorId: number };

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
                if (canvas.width !== frame.displayWidth) {
                    canvas.width = frame.displayWidth;
                    canvas.height = frame.displayHeight;
                }
                ctx.drawImage(frame, 0, 0);
                frame.close();
                setHasFrame(true);
            },
            error: (e) => console.error("[share] decoder error:", e),
        })
        decoder.configure({ codec: "avc1.42E01F", optimizeForLatency: true });

        const t0 = performance.now();
        let gotKey = false;

        const channel = new Channel<ArrayBuffer>();
        channel.onmessage = (buf) => {
            if (canceled || decoder.state !== "configured") return;

            const data = new Uint8Array(buf);
            const isKey = data[0] === 1;

            if (!gotKey && !isKey) return;
            gotKey = true;

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
                    })
                )
            } catch (e) {
                console.error("[share] decode falhou:", e);
                gotKey = false;
            }
        }

        // precisa ficar fora do onmessage: o canal só recebe depois que o Rust começa a enviar
        invoke("start_share", { config: { monitor_id: monitorId }, onChunk: channel })
            .catch((e) => console.error("[share] start_share falhou:", e));

        return () => {
            canceled = true;
            invoke("stop_share").catch((e) =>
                console.error("[share] stop_share falhou:", e)
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
}
