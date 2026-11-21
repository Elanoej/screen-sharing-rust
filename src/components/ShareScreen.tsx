import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useRef, useState } from "react";

type Monitor = { id: string; label: string };
type Quality = "720p30" | "720p60" | "1080p60";

const QUALITIES: {
  value: Quality;
  label: string;
  res: string;
  fps: number;
  w: number;
  h: number;
}[] = [
  { value: "720p30", label: "720p · 30", res: "720p", fps: 30, w: 1280, h: 720 },
  { value: "720p60", label: "720p · 60", res: "720p", fps: 60, w: 1280, h: 720 },
  { value: "1080p60", label: "1080p · 60", res: "1080p", fps: 60, w: 1920, h: 1080 },
];

const getAvailableMonitors = () => invoke<Monitor[]>("available_monitors");

/** Anima um número até o alvo (só visual). */
function useTween(target: number, ms = 600) {
  const [value, setValue] = useState(target);
  const from = useRef(target);
  useEffect(() => {
    const start = performance.now();
    const origin = from.current;
    let raf = 0;
    const tick = (now: number) => {
      const t = Math.min((now - start) / ms, 1);
      const eased = 1 - Math.pow(1 - t, 4);
      const v = origin + (target - origin) * eased;
      from.current = v;
      setValue(v);
      if (t < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [target, ms]);
  return Math.round(value);
}

function TitleBar() {
  const win = getCurrentWindow();
  const btn =
    "grid h-full w-11 place-items-center text-zinc-500 transition-colors hover:bg-white/[0.06] hover:text-white";
  return (
    <div
      data-tauri-drag-region
      className="relative z-10 flex h-10 shrink-0 items-center justify-between pl-5"
    >
      <div data-tauri-drag-region className="flex items-center gap-2.5">
        <span className="size-1.5 rounded-full bg-accent shadow-[0_0_10px_2px_rgba(139,108,255,0.7)]" />
        <span
          data-tauri-drag-region
          className="font-mono text-[10px] uppercase tracking-[0.3em] text-zinc-500"
        >
          Screen Share
        </span>
      </div>
      <div className="flex h-full">
        <button aria-label="Minimizar" className={btn} onClick={() => win.minimize()}>
          <svg viewBox="0 0 12 12" className="size-3" stroke="currentColor" strokeWidth="1.2"><path d="M2 6h8" /></svg>
        </button>
        <button aria-label="Maximizar" className={btn} onClick={() => win.toggleMaximize()}>
          <svg viewBox="0 0 12 12" className="size-3" fill="none" stroke="currentColor" strokeWidth="1.2"><rect x="2.5" y="2.5" width="7" height="7" /></svg>
        </button>
        <button
          aria-label="Fechar"
          className="grid h-full w-11 place-items-center text-zinc-500 transition-colors hover:bg-red-500/80 hover:text-white"
          onClick={() => win.close()}
        >
          <svg viewBox="0 0 12 12" className="size-3" stroke="currentColor" strokeWidth="1.2"><path d="m2.5 2.5 7 7M9.5 2.5l-7 7" /></svg>
        </button>
      </div>
    </div>
  );
}

function MonitorSelect({
  monitors,
  value,
  onChange,
}: {
  monitors: Monitor[];
  value: string;
  onChange: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const index = monitors.findIndex((m) => m.id === value);
  const selected = monitors[index];

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") return setOpen(false);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) return setOpen(true);
      const next = index + (e.key === "ArrowDown" ? 1 : -1);
      const target = monitors[Math.max(0, Math.min(monitors.length - 1, next))];
      if (target) onChange(target.id);
    }
  }

  return (
    <div ref={ref} className="relative" onKeyDown={onKeyDown}>
      <button
        type="button"
        id="monitor"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={`group flex w-full cursor-pointer items-center justify-between gap-3 border-b px-0 pb-3 pt-1 text-left text-[15px] text-white outline-none transition-colors duration-300 focus-visible:border-accent ${open ? "border-accent" : "border-white/15 hover:border-white/40"}`}
      >
        <span className="truncate">
          {selected?.label ?? "Nenhum monitor encontrado"}
        </span>
        <svg
          viewBox="0 0 24 24"
          className={`size-4 shrink-0 text-zinc-500 transition-all duration-300 group-hover:text-white ${open ? "rotate-180 text-accent" : ""}`}
          fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"
        >
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>

      {open && monitors.length > 0 && (
        <ul
          role="listbox"
          className="absolute inset-x-0 top-[calc(100%+10px)] z-50 m-0 max-h-60 origin-top animate-pop list-none overflow-auto rounded-xl border border-white/10 bg-[#100e18] p-1.5 shadow-[0_24px_60px_-12px_rgba(0,0,0,1),0_0_0_1px_rgba(0,0,0,0.5)]"
        >
          {monitors.map((m, i) => {
            const active = m.id === value;
            return (
              <li
                key={m.id}
                role="option"
                aria-selected={active}
                onClick={() => {
                  onChange(m.id);
                  setOpen(false);
                }}
                className={`flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2.5 text-sm transition-colors ${active ? "bg-accent/15 text-white" : "text-zinc-400 hover:bg-white/[0.06] hover:text-white"}`}
              >
                <span className="w-4 font-mono text-[10px] text-zinc-600">
                  {String(i + 1).padStart(2, "0")}
                </span>
                <span className="flex-1 truncate">{m.label}</span>
                {active && <span className="size-1.5 rounded-full bg-accent" />}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function Stage({
  monitor,
  q,
  fps,
  width,
  height,
}: {
  monitor?: Monitor;
  q: (typeof QUALITIES)[number];
  fps: number;
  width: number;
  height: number;
}) {
  const [tilt, setTilt] = useState({ x: 0, y: 0 });

  function onMove(e: React.MouseEvent<HTMLDivElement>) {
    const r = e.currentTarget.getBoundingClientRect();
    const px = (e.clientX - r.left) / r.width - 0.5;
    const py = (e.clientY - r.top) / r.height - 0.5;
    setTilt({ x: -py * 8, y: px * 10 });
  }

  const corner = "absolute size-3.5 border-accent/80";

  return (
    <div
      className="flex h-full flex-col items-center justify-center [perspective:1100px]"
      onMouseMove={onMove}
      onMouseLeave={() => setTilt({ x: 0, y: 0 })}
    >
      <div
        className="w-full max-w-[460px] animate-float transition-transform duration-300 ease-out"
        style={{ transform: `rotateX(${tilt.x}deg) rotateY(${tilt.y}deg)` }}
      >
        <div className="relative aspect-[16/10] overflow-hidden rounded-xl border border-white/10 bg-[#0b0a12] shadow-[0_40px_80px_-30px_rgba(139,108,255,0.45),inset_0_1px_0_rgba(255,255,255,0.06)]">
          {/* desktop abstrato */}
          <div aria-hidden className="absolute inset-0">
            <div className="absolute -left-10 -top-10 size-56 animate-drift-a rounded-full bg-accent/45 blur-[70px]" />
            <div className="absolute -bottom-16 -right-10 size-64 animate-drift-b rounded-full bg-indigo-500/30 blur-[80px]" />
            <div className="absolute left-[8%] top-[16%] h-[38%] w-[46%] rounded-md border border-white/10 bg-white/[0.04] backdrop-blur-sm" />
            <div className="absolute right-[8%] top-[28%] h-[44%] w-[34%] rounded-md border border-white/10 bg-white/[0.06] backdrop-blur-sm" />
            <div className="absolute inset-x-[18%] bottom-[5%] h-[7%] rounded-full border border-white/10 bg-white/[0.06]" />
          </div>

          {/* linha de varredura */}
          <div aria-hidden className="absolute inset-x-0 top-0 h-px animate-scan bg-gradient-to-r from-transparent via-accent to-transparent shadow-[0_0_14px_2px_rgba(139,108,255,0.6)]" />

          {/* flash ao trocar de monitor / qualidade */}
          <div
            key={`${monitor?.id}-${q.value}`}
            aria-hidden
            className="absolute inset-0 animate-flash bg-accent/30"
          />

          {/* marcas de canto */}
          <span className={`${corner} left-3 top-3 border-l border-t`} />
          <span className={`${corner} right-3 top-3 border-r border-t`} />
          <span className={`${corner} bottom-3 left-3 border-b border-l`} />
          <span className={`${corner} bottom-3 right-3 border-b border-r`} />

          {/* HUD */}
          <div className="absolute left-7 top-6 flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.2em] text-zinc-300">
            <span className="relative grid size-2 place-items-center">
              <span className="absolute size-2 animate-ping-slow rounded-full bg-emerald-400/70" />
              <span className="size-1.5 rounded-full bg-emerald-400" />
            </span>
            Pronto
          </div>
          <div className="absolute bottom-6 left-7 right-7 flex items-end justify-between">
            <span className="max-w-[60%] truncate font-mono text-[10px] uppercase tracking-[0.18em] text-zinc-400">
              {monitor?.label ?? "—"}
            </span>
            <span className="font-mono text-xs tabular-nums text-white">
              {width}×{height}
            </span>
          </div>
        </div>
        <div className="mx-auto mt-3 h-1 w-24 rounded-full bg-gradient-to-r from-transparent via-white/15 to-transparent" />
      </div>

      <div className="mt-8 flex w-full max-w-[460px] items-end justify-between">
        <div>
          <div className="font-mono text-[10px] uppercase tracking-[0.25em] text-zinc-600">
            Taxa de quadros
          </div>
          <div className="mt-1 flex items-baseline gap-1.5">
            <span className="text-5xl font-extralight tabular-nums tracking-tight text-white">
              {fps}
            </span>
            <span className="font-mono text-xs text-zinc-500">fps</span>
          </div>
        </div>
        <div className="text-right">
          <div className="font-mono text-[10px] uppercase tracking-[0.25em] text-zinc-600">
            Resolução
          </div>
          <div className="mt-1 text-2xl font-light tracking-tight text-zinc-200">
            {q.res}
          </div>
        </div>
      </div>
    </div>
  );
}

export default function ShareScreen() {
  const [availableMonitors, setAvailableMonitors] = useState<Monitor[]>([]);

  const [monitorId, setMonitorId] = useState("");
  const [quality, setQuality] = useState<Quality>("720p60");

  useEffect(() => {
    getAvailableMonitors()
      .then((list) => {
        setAvailableMonitors(list);
        setMonitorId(list[0]?.id ?? "");
      })
      .catch(console.error);
  }, []);

  function handleStart() {
    // TODO: invoke("start_stream", { monitorId, quality })
    console.log({ monitorId, quality });
  }

  const qIndex = QUALITIES.findIndex((o) => o.value === quality);
  const q = QUALITIES[qIndex];
  const fps = useTween(q.fps);
  const width = useTween(q.w);
  const height = useTween(q.h);
  const monitor = availableMonitors.find((m) => m.id === monitorId);

  // holofote que segue o cursor
  const [spot, setSpot] = useState({ x: 70, y: 20 });

  return (
    <div
      className="grain relative flex h-screen w-full flex-col overflow-hidden bg-ink text-zinc-100"
      onMouseMove={(e) =>
        setSpot({
          x: (e.clientX / window.innerWidth) * 100,
          y: (e.clientY / window.innerHeight) * 100,
        })
      }
    >
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 transition-[background] duration-500 ease-out"
        style={{
          background: `radial-gradient(600px circle at ${spot.x}% ${spot.y}%, rgba(139,108,255,0.16), transparent 60%)`,
        }}
      />

      <TitleBar />

      <div className="relative flex min-h-0 flex-1 gap-4 px-4 pb-4">
        {/* Controles */}
        <section className="relative z-20 flex w-[340px] shrink-0 flex-col rounded-2xl border border-white/[0.08] bg-white/[0.03] p-8 shadow-[inset_0_1px_0_rgba(255,255,255,0.05)] backdrop-blur-xl">
          <div className="animate-rise">
            <h1 className="m-0 text-[34px] font-extralight leading-[1.05] tracking-tight text-white">
              Transmitir
              <br />
              <span className="font-normal text-accent">sua tela.</span>
            </h1>
          </div>

          <div className="mt-12 animate-rise [animation-delay:120ms]">
            <label
              htmlFor="monitor"
              className="mb-1 block font-mono text-[10px] uppercase tracking-[0.25em] text-zinc-500"
            >
              01 — Monitor
            </label>
            <MonitorSelect
              monitors={availableMonitors}
              value={monitorId}
              onChange={setMonitorId}
            />
          </div>

          <fieldset className="m-0 mt-9 animate-rise border-0 p-0 [animation-delay:220ms]">
            <legend className="mb-3 p-0 font-mono text-[10px] uppercase tracking-[0.25em] text-zinc-500">
              02 — Qualidade
            </legend>
            <div className="relative grid grid-cols-3 rounded-xl border border-white/[0.08] bg-black/30 p-1">
              <span
                aria-hidden
                className="absolute inset-y-1 left-1 w-[calc((100%-0.5rem)/3)] rounded-lg bg-accent/20 shadow-[inset_0_0_0_1px_rgba(139,108,255,0.55),0_0_24px_-4px_rgba(139,108,255,0.5)] transition-transform duration-500 ease-[cubic-bezier(0.34,1.4,0.64,1)]"
                style={{ transform: `translateX(${qIndex * 100}%)` }}
              />
              {QUALITIES.map((option) => (
                <label key={option.value} className="relative cursor-pointer">
                  <input
                    type="radio"
                    name="quality"
                    value={option.value}
                    checked={quality === option.value}
                    onChange={() => setQuality(option.value)}
                    className="peer sr-only"
                  />
                  <span className="block rounded-lg py-2.5 text-center font-mono text-[11px] text-zinc-500 transition-colors duration-300 hover:text-zinc-200 peer-checked:text-white peer-focus-visible:ring-2 peer-focus-visible:ring-accent/60">
                    {option.label}
                  </span>
                </label>
              ))}
            </div>
          </fieldset>

          <div className="mt-auto animate-rise [animation-delay:320ms]">
            <button
              type="button"
              onClick={handleStart}
              disabled={!monitorId}
              className="group relative flex w-full cursor-pointer items-center justify-between overflow-hidden rounded-full bg-zinc-100 py-1.5 pl-6 pr-1.5 text-sm font-medium text-zinc-950 outline-none transition-all duration-300 hover:shadow-[0_0_44px_-6px_rgba(139,108,255,0.8)] focus-visible:ring-2 focus-visible:ring-accent active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:shadow-none"
            >
              <span
                aria-hidden
                className="absolute inset-0 origin-left scale-x-0 bg-accent transition-transform duration-500 ease-[cubic-bezier(0.16,1,0.3,1)] group-hover:scale-x-100 group-disabled:group-hover:scale-x-0"
              />
              <span className="relative transition-colors duration-300 group-hover:text-white">
                Iniciar transmissão
              </span>
              <span className="relative grid size-10 place-items-center rounded-full bg-zinc-950 text-white transition-transform duration-500 group-hover:rotate-[-45deg] group-hover:bg-white group-hover:text-accent">
                <svg viewBox="0 0 24 24" className="size-4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M5 12h14M13 6l6 6-6 6" />
                </svg>
              </span>
            </button>
          </div>
        </section>

        {/* Palco */}
        <section className="relative z-10 min-w-0 flex-1 animate-rise rounded-2xl border border-white/[0.06] bg-white/[0.015] p-8 [animation-delay:200ms]">
          <Stage monitor={monitor} q={q} fps={fps} width={width} height={height} />
        </section>
      </div>
    </div>
  );
}
