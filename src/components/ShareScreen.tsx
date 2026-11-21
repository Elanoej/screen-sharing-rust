import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type Monitor = { id: string; label: string };
type Quality = "720p30" | "720p60" | "1080p60";

const QUALITIES: { value: Quality; label: string }[] = [
  { value: "720p30", label: "720p · 30 fps" },
  { value: "720p60", label: "720p · 60 fps" },
  { value: "1080p60", label: "1080p · 60 fps" },
];

const getAvailableMonitors = () => invoke<Monitor[]>("available_monitors");

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

  return (
    <main className="flex min-h-screen w-full flex-col gap-3.5 bg-zinc-50 p-5 text-zinc-900">
      <h1 className="m-0 text-base font-semibold">Compartilhar tela</h1>

      <label htmlFor="monitor" className="text-xs font-semibold text-zinc-500">
        Monitor
      </label>
      <select
        id="monitor"
        value={monitorId}
        onChange={(event) => setMonitorId(event.target.value)}
        className="w-full rounded-lg border border-zinc-300 bg-white px-2.5 py-2 text-sm"
      >
        {availableMonitors.map((monitor) => (
          <option key={monitor.id} value={monitor.id}>
            {monitor.label}
          </option>
        ))}
      </select>

      <fieldset className="m-0 border-0 p-0">
        <legend className="text-xs font-semibold text-zinc-500">
          Qualidade
        </legend>
        {QUALITIES.map((option) => (
          <label
            key={option.value}
            className="flex items-center gap-2 py-1 text-sm"
          >
            <input
              type="radio"
              name="quality"
              value={option.value}
              checked={quality === option.value}
              onChange={() => setQuality(option.value)}
              className="accent-blue-600"
            />
            {option.label}
          </label>
        ))}
      </fieldset>

      <button
        type="button"
        onClick={handleStart}
        className="mt-1.5 w-full rounded-lg bg-blue-600 px-3 py-2.5 font-semibold text-white transition-colors hover:bg-blue-700"
      >
        ▶ Iniciar
      </button>
    </main>
  );
}
