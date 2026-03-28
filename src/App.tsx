import "./App.css";

const roadmap = [
  {
    phase: "Phase 1",
    title: "Tray-First Shell",
    detail: "The app now starts in the background and opens from the tray.",
  },
  {
    phase: "Phase 2",
    title: "SQLite Brain",
    detail: "We will persist tickers, daily metrics, and alerts locally.",
  },
  {
    phase: "Phase 3",
    title: "Async Scanner",
    detail: "Tokio and reqwest will power the background market polling loop.",
  },
  {
    phase: "Phase 4",
    title: "Dashboard",
    detail: "The UI will grow into a sector heatmap, alert feed, and settings view.",
  },
];

function App() {
  return (
    <main className="min-h-screen bg-[radial-gradient(circle_at_top,_#dbeafe,_#f8fafc_55%,_#e2e8f0)] px-6 py-10 text-slate-950">
      <div className="mx-auto flex max-w-5xl flex-col gap-8">
        <section className="overflow-hidden rounded-[2rem] border border-white/70 bg-white/80 p-8 shadow-[0_30px_80px_-40px_rgba(15,23,42,0.45)] backdrop-blur">
          <div className="flex flex-col gap-6 lg:flex-row lg:items-end lg:justify-between">
            <div className="max-w-2xl">
              <p className="text-sm font-semibold uppercase tracking-[0.35em] text-sky-700">
                TrendWave
              </p>
              <h1 className="mt-4 text-4xl font-semibold tracking-tight text-slate-950 sm:text-5xl">
                A background-first market scanner with a Rust core.
              </h1>
              <p className="mt-4 text-lg leading-8 text-slate-600">
                This dashboard is intentionally simple for now. Phase 1 is about
                the tray-first shell, so the real milestone is that TrendWave
                now opens from the tray instead of launching into your face.
              </p>
            </div>

            <div className="rounded-3xl border border-slate-200 bg-slate-950 px-5 py-4 text-sm text-slate-100 shadow-lg">
              <p className="font-medium text-emerald-300">Status</p>
              <p className="mt-2 text-slate-300">
                Tray shell in progress
              </p>
              <p className="mt-1 text-slate-400">
                SQLite and scanning arrive next
              </p>
            </div>
          </div>
        </section>

        <section className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          {roadmap.map((item) => (
            <article
              key={item.phase}
              className="rounded-[1.5rem] border border-slate-200/80 bg-white/85 p-5 shadow-[0_20px_50px_-35px_rgba(15,23,42,0.45)]"
            >
              <p className="text-sm font-semibold uppercase tracking-[0.25em] text-slate-500">
                {item.phase}
              </p>
              <h2 className="mt-3 text-2xl font-semibold text-slate-950">
                {item.title}
              </h2>
              <p className="mt-3 text-sm leading-7 text-slate-600">
                {item.detail}
              </p>
            </article>
          ))}
        </section>

        <section className="grid gap-4 lg:grid-cols-[1.4fr_1fr]">
          <article className="rounded-[1.75rem] border border-slate-200/80 bg-slate-950 p-6 text-slate-100 shadow-[0_24px_60px_-35px_rgba(15,23,42,0.6)]">
            <p className="text-sm font-semibold uppercase tracking-[0.3em] text-sky-300">
              What To Try
            </p>
            <ul className="mt-5 space-y-3 text-sm leading-7 text-slate-300">
              <li>Open the tray icon and choose "Open Dashboard".</li>
              <li>Close the window and notice that the app keeps running in the tray.</li>
              <li>Toggle "Pause Scanning" to see the tray state change before Phase 3 exists.</li>
            </ul>
          </article>

          <article className="rounded-[1.75rem] border border-white/80 bg-white/85 p-6 shadow-[0_24px_60px_-35px_rgba(15,23,42,0.45)]">
            <p className="text-sm font-semibold uppercase tracking-[0.3em] text-amber-600">
              Rust Focus
            </p>
            <p className="mt-4 text-sm leading-7 text-slate-600">
              This phase is our first ownership exercise: long-lived tray
              callbacks own the values they capture, while helper functions borrow
              shared app and window handles when they only need temporary access.
            </p>
          </article>
        </section>
      </div>
    </main>
  );
}

export default App;
