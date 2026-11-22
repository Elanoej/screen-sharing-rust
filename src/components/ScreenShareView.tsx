import { Channel, invoke } from "@tauri-apps/api/core";
import { useEffect, useRef } from "react"

type Props = { monitorId: number };

export const ScreenShareView = ({ monitorId }: Props) => {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    
    useEffect(() => {
        const canvas = canvasRef.current!;
        const ctx = canvas.getContext("2d")!;
        let canceled = false;

        const decoder = new VideoDecoder({
            output: (frame) => {
                if (canvas.width !== frame.displayWidth) {
                    canvas.width = frame.displayWidth;
                    canvas.height = frame.displayHeight;
                }
                ctx.drawImage(frame, 0, 0);
                frame.close();
            },
            error: (e) => console.error("Decoder: ", e),
        })
        decoder.configure({ codec: "avc.42E01F", optimizeForLatency: true });

        const t0 = performance.now();
        let gotKey = false;

        const channel = new Channel<ArrayBuffer>();
        channel.onmessage = (buf) => {
            if(canceled || decoder.state == "closed") return;

            const data = new Uint8Array(buf);
            const isKey = data[0] === 1;

            if (!gotKey && !isKey) return;
            gotKey = true;

            // backpressre: fila grande = decoder atrasado, descarta delta e espera keyframe
            if (decoder.decodeQueueSize > 3 && !isKey) {
                gotKey = false;
                return;
            }

            decoder.decode(
                new EncodedVideoChunk({
                    type: isKey ? "key": "delta",
                    timestamp: (performance.now() - t0) * 1000,
                    data: data.subarray(1), 
                })
            )

            invoke("start_share", { config: { monitor_id: monitorId }, onChunk: channel })
                .catch(console.error);

            return () => {
                canceled = true;
                invoke("stop_share").catch(console.error);
                if (decoder.state !== "closed") decoder.close();
            };
        }
    }, [monitorId]);

    return <canvas className="w-full bg-black" ref={canvasRef} />
}