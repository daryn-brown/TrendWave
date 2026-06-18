import { useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type {
  AppErrorShape,
  Bottleneck,
  Candidate,
  GrowthData,
  Settings,
  Watchlist,
} from "./types";

// ---- Icons (inline so we ship no icon dependency) ---------------------------

type IconProps = { className?: string };
const base = "h-4 w-4";

export const IconSearch = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="11" cy="11" r="8" /><path d="m21 21-4.3-4.3" />
  </svg>
);
export const IconSparkles = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2 2M16 16l2 2M18 6l-2 2M8 16l-2 2" />
  </svg>
);
export const IconPlus = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M5 12h14M12 5v14" />
  </svg>
);
export const IconTrash = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6" />
  </svg>
);
export const IconSettings = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </svg>
);
export const IconSpinner = ({ className = base }: IconProps) => (
  <svg className={`${className} animate-spin`} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M21 12a9 9 0 1 1-6.2-8.5" />
  </svg>
);
export const IconAlert = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0zM12 9v4M12 17h.01" />
  </svg>
);
export const IconExternal = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M15 3h6v6M10 14 21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
  </svg>
);
export const IconClose = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M18 6 6 18M6 6l12 12" />
  </svg>
);
export const IconRefresh = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M3 12a9 9 0 0 1 15-6.7L21 8M21 3v5h-5M21 12a9 9 0 0 1-15 6.7L3 16M3 21v-5h5" />
  </svg>
);
export const IconDownload = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3" />
  </svg>
);

// ---- Formatting helpers -----------------------------------------------------

export const formatPrice = (price: number, currency: string) => {
  try {
    return new Intl.NumberFormat("en-US", { style: "currency", currency: currency || "USD" }).format(price);
  } catch {
    return `${price.toFixed(2)} ${currency}`;
  }
};

export const formatPct = (pct: number) => `${pct >= 0 ? "+" : ""}${pct.toFixed(1)}%`;

// Growth metrics arrive as fractions (0.13 = +13%).
export const formatGrowthPct = (frac: number) =>
  `${frac >= 0 ? "+" : ""}${(frac * 100).toFixed(1)}%`;

const growthTone = (frac: number) =>
  frac > 0.001 ? "text-emerald-600" : frac < -0.001 ? "text-rose-600" : "text-slate-600";

export const formatVolume = (v: number) => {
  if (v >= 1e9) return `${(v / 1e9).toFixed(1)}B`;
  if (v >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
  if (v >= 1e3) return `${(v / 1e3).toFixed(1)}K`;
  return `${Math.round(v)}`;
};

export const sentimentLabel = (s: number | null | undefined) => {
  if (s == null) return { text: "No signal", color: "text-slate-400" };
  if (s > 0.25) return { text: "Bullish", color: "text-emerald-600" };
  if (s < -0.25) return { text: "Bearish", color: "text-rose-600" };
  return { text: "Neutral", color: "text-slate-500" };
};

const severityStyle = (sev: number) => {
  if (sev >= 5) return "bg-rose-100 text-rose-700 ring-rose-200";
  if (sev >= 4) return "bg-orange-100 text-orange-700 ring-orange-200";
  if (sev >= 3) return "bg-amber-100 text-amber-700 ring-amber-200";
  return "bg-slate-100 text-slate-600 ring-slate-200";
};

const scoreColor = (score: number) => {
  if (score >= 70) return "text-emerald-600";
  if (score >= 55) return "text-sky-600";
  return "text-slate-500";
};

// ---- Error banner -----------------------------------------------------------

export function ErrorBanner({ error, onDismiss }: { error: AppErrorShape; onDismiss: () => void }) {
  const needsOllama = error.kind === "ollama_unavailable";
  const modelMissing = error.kind === "model_missing";
  return (
    <div className="rounded-2xl border border-rose-200 bg-rose-50 p-4 text-sm text-rose-800">
      <div className="flex items-start gap-3">
        <IconAlert className="mt-0.5 h-5 w-5 shrink-0 text-rose-500" />
        <div className="flex-1">
          <p className="font-semibold">{error.message}</p>
          {(needsOllama || modelMissing) && (
            <div className="mt-2 space-y-1 font-mono text-xs text-rose-700/90">
              {needsOllama && <p># Install: https://ollama.com — then run</p>}
              {needsOllama && <p>ollama serve</p>}
              {modelMissing && <p>ollama pull &lt;model&gt;</p>}
            </div>
          )}
        </div>
        <button onClick={onDismiss} className="rounded-lg p-1 text-rose-400 hover:bg-rose-100">
          <IconClose />
        </button>
      </div>
    </div>
  );
}

// ---- Update banner ----------------------------------------------------------

export type UpdatePhase = "available" | "downloading" | "ready";

export function UpdateBanner({
  version,
  phase,
  progress,
  onInstall,
  onRestart,
  onDismiss,
}: {
  version: string;
  phase: UpdatePhase;
  progress: number;
  onInstall: () => void;
  onRestart: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="rounded-2xl border border-sky-200 bg-sky-50 p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <IconDownload className="h-5 w-5 shrink-0 text-sky-600" />
          <div>
            <p className="text-sm font-semibold text-sky-900">
              {phase === "ready" ? `Update ready — v${version}` : `Update available — v${version}`}
            </p>
            <p className="text-xs text-sky-700">
              {phase === "available" && "A newer version of TrendWave is ready to install."}
              {phase === "downloading" && `Downloading… ${progress}%`}
              {phase === "ready" && "Restart to start using the new version."}
            </p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {phase === "available" && (
            <>
              <button
                onClick={onInstall}
                className="rounded-lg bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700"
              >
                Download &amp; install
              </button>
              <button
                onClick={onDismiss}
                className="rounded-lg px-2 py-1.5 text-sky-700 hover:bg-sky-100"
                title="Dismiss"
              >
                <IconClose className="h-3.5 w-3.5" />
              </button>
            </>
          )}
          {phase === "downloading" && <IconSpinner className="h-4 w-4 text-sky-600" />}
          {phase === "ready" && (
            <button
              onClick={onRestart}
              className="rounded-lg bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700"
            >
              Restart now
            </button>
          )}
        </div>
      </div>
      {phase === "downloading" && (
        <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-sky-100">
          <div
            className="h-full rounded-full bg-sky-500 transition-all"
            style={{ width: `${progress}%` }}
          />
        </div>
      )}
    </div>
  );
}

// ---- Progress log -----------------------------------------------------------

export function ProgressLog({ messages, running }: { messages: string[]; running: boolean }) {
  if (messages.length === 0) return null;
  return (
    <div className="rounded-2xl border border-slate-200 bg-white/70 p-4">
      <ul className="space-y-2 text-sm">
        {messages.map((m, i) => {
          const isLast = i === messages.length - 1;
          return (
            <li key={i} className="flex items-center gap-2 text-slate-600">
              {running && isLast ? (
                <IconSpinner className="h-4 w-4 text-sky-500" />
              ) : (
                <span className="flex h-4 w-4 items-center justify-center text-emerald-500">✓</span>
              )}
              <span className={running && isLast ? "text-slate-900" : ""}>{m}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

// ---- Bottleneck list --------------------------------------------------------

export function BottleneckList({ items }: { items: Bottleneck[] }) {
  if (items.length === 0) return null;
  return (
    <section className="space-y-3">
      <h3 className="text-sm font-semibold uppercase tracking-[0.2em] text-slate-500">
        Identified bottlenecks
      </h3>
      <div className="grid gap-3 md:grid-cols-2">
        {items.map((b, i) => (
          <div key={i} className="rounded-2xl border border-slate-200 bg-white p-4">
            <div className="flex items-start justify-between gap-2">
              <h4 className="font-semibold text-slate-900">{b.title}</h4>
              <span className={`shrink-0 rounded-full px-2 py-0.5 text-xs font-medium ring-1 ${severityStyle(b.severity)}`}>
                Severity {b.severity}/5
              </span>
            </div>
            <p className="mt-2 text-sm leading-6 text-slate-600">{b.description}</p>
          </div>
        ))}
      </div>
    </section>
  );
}

// ---- Candidate card ---------------------------------------------------------

function NewsLink({ url, children }: { url: string; children: ReactNode }) {
  return (
    <button
      onClick={() => url && openUrl(url).catch(() => {})}
      className="group inline-flex items-start gap-1.5 text-left text-sm text-slate-600 hover:text-sky-700"
    >
      <IconExternal className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-400 group-hover:text-sky-600" />
      <span className="underline-offset-2 group-hover:underline">{children}</span>
    </button>
  );
}

export function CandidateCard({ candidate }: { candidate: Candidate }) {
  const [showNews, setShowNews] = useState(false);
  const senti = sentimentLabel(candidate.sentiment);
  const price = candidate.price;
  return (
    <article className="rounded-3xl border border-slate-200 bg-white p-6 shadow-[0_18px_50px_-40px_rgba(15,23,42,0.5)]">
      <header className="flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h3 className="text-xl font-bold tracking-tight text-slate-900">{candidate.ticker}</h3>
            {price && (
              <span className="text-lg font-semibold text-slate-900">
                {formatPrice(price.price, price.currency)}
              </span>
            )}
            {price && (
              <span className={`text-sm font-medium ${price.change_pct >= 0 ? "text-emerald-600" : "text-rose-600"}`}>
                {formatPct(price.change_pct)}
              </span>
            )}
          </div>
          <p className="mt-0.5 text-sm text-slate-500">{candidate.verified_name ?? candidate.company}</p>
        </div>
        <div className="text-right">
          <div className={`text-2xl font-bold ${scoreColor(candidate.score)}`}>
            {Math.round(candidate.score)}
          </div>
          <div className="text-[10px] font-medium uppercase tracking-wider text-slate-400">score</div>
        </div>
      </header>

      {candidate.identity_mismatch && (
        <div className="mt-3 flex items-start gap-2 rounded-xl bg-amber-50 px-3 py-2 text-xs text-amber-800 ring-1 ring-amber-200">
          <span aria-hidden className="mt-px font-semibold">⚠</span>
          <span>
            <strong>{candidate.ticker}</strong> is{" "}
            <strong>{candidate.verified_name ?? candidate.company}</strong> — which may not match this
            thesis. The model proposed “{candidate.company}”; its score has been reduced. Confirm the
            ticker is the company you intend before acting on the figures below.
          </span>
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <span className="rounded-full bg-sky-50 px-2.5 py-1 text-xs font-medium text-sky-700 ring-1 ring-sky-100">
          {candidate.bottleneck}
        </span>
        <span className="rounded-full bg-indigo-50 px-2.5 py-1 text-xs font-medium text-indigo-700 ring-1 ring-indigo-100">
          Moat {candidate.moat}/5
        </span>
        {!candidate.identity_mismatch &&
          (candidate.growth?.revenue_growth_yoy != null ? (
            <span
              className={`rounded-full px-2.5 py-1 text-xs font-medium ring-1 ${
                candidate.growth.revenue_growth_yoy >= 0
                  ? "bg-emerald-50 text-emerald-700 ring-emerald-100"
                  : "bg-rose-50 text-rose-700 ring-rose-100"
              }`}
            >
              Rev {formatGrowthPct(candidate.growth.revenue_growth_yoy)} {candidate.growth.annual_growth ? "YoY" : "qtr"}
            </span>
          ) : (
            <span className="rounded-full bg-slate-50 px-2.5 py-1 text-xs font-medium text-slate-600 ring-1 ring-slate-100">
              Growth {Math.round(candidate.growth_score * 100)}
            </span>
          ))}
        <span className={`text-xs font-medium ${senti.color}`}>● {senti.text}</span>
        {price && price.avg_volume > 0 && (
          <span className="text-xs text-slate-400">avg vol {formatVolume(price.avg_volume)}</span>
        )}
      </div>

      <dl className="mt-4 space-y-3">
        {candidate.thesis && (
          <Field label="Why it's positioned to win" value={candidate.thesis} />
        )}
        {candidate.upside_rationale && (
          <Field label="Growth outlook (model view)" value={candidate.upside_rationale} />
        )}
      </dl>

      {candidate.identity_mismatch ? (
        <p className="mt-4 rounded-xl bg-slate-50 px-3 py-2 text-xs leading-5 text-slate-500 ring-1 ring-slate-200">
          Financials hidden — these SEC EDGAR / Yahoo figures are{" "}
          <strong className="font-semibold text-slate-600">{candidate.verified_name ?? candidate.ticker}</strong>’s
          actuals for ticker {candidate.ticker}, and don’t describe the “{candidate.company}” business
          in this thesis. They were excluded so they can’t be read as support for the pick.
        </p>
      ) : candidate.growth ? (
        <GrowthPanel growth={candidate.growth} />
      ) : (
        <p className="mt-3 text-xs text-slate-400">No growth fundamentals found for this ticker.</p>
      )}

      {candidate.news.length > 0 && (
        <div className="mt-4 border-t border-slate-100 pt-3">
          <button
            onClick={() => setShowNews((s) => !s)}
            className="text-xs font-medium text-slate-500 hover:text-slate-800"
          >
            {showNews ? "Hide" : "Show"} {candidate.news.length} headline
            {candidate.news.length > 1 ? "s" : ""}
          </button>
          {showNews && (
            <ul className="mt-2 space-y-1.5">
              {candidate.news.map((n, i) => (
                <li key={i}>
                  <NewsLink url={n.url}>{n.title}</NewsLink>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </article>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{label}</dt>
      <dd className="mt-0.5 text-sm leading-6 text-slate-700">{value}</dd>
    </div>
  );
}

function GrowthPanel({ growth }: { growth: GrowthData }) {
  const stats: { label: string; value: string; tone: string }[] = [];
  const period = growth.annual_growth ? "YoY" : "YoY (qtr)";
  const hasEdgar = growth.source.includes("EDGAR");
  if (growth.revenue_growth_yoy != null)
    stats.push({
      label: `Revenue ${period}`,
      value: formatGrowthPct(growth.revenue_growth_yoy),
      tone: growthTone(growth.revenue_growth_yoy),
    });
  if (growth.revenue_cagr != null)
    stats.push({
      label: growth.years ? `Revenue CAGR · ${growth.years}y` : "Revenue CAGR",
      value: formatGrowthPct(growth.revenue_cagr),
      tone: growthTone(growth.revenue_cagr),
    });
  if (growth.earnings_growth_yoy != null)
    stats.push({
      label: `Earnings ${period}`,
      value: formatGrowthPct(growth.earnings_growth_yoy),
      tone: growthTone(growth.earnings_growth_yoy),
    });
  if (growth.profitable != null)
    stats.push({
      label: "Profitability",
      value: growth.profitable ? "Profitable" : "Unprofitable",
      tone: growth.profitable ? "text-emerald-600" : "text-rose-600",
    });
  if (growth.analyst_upside != null)
    stats.push({
      label: "Analyst target",
      value: formatGrowthPct(growth.analyst_upside),
      tone: growthTone(growth.analyst_upside),
    });
  if (growth.forward_pe != null)
    stats.push({ label: "Forward P/E", value: growth.forward_pe.toFixed(1), tone: "text-slate-700" });

  if (stats.length === 0) return null;
  return (
    <div className="mt-4 rounded-2xl bg-slate-50 p-3 ring-1 ring-slate-100">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
          Growth research
        </span>
        {growth.source && <span className="text-[10px] text-slate-400">via {growth.source}</span>}
      </div>
      <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-3">
        {stats.map((s) => (
          <div key={s.label}>
            <dt className="text-[10px] uppercase tracking-wide text-slate-400">{s.label}</dt>
            <dd className={`text-sm font-semibold ${s.tone}`}>{s.value}</dd>
          </div>
        ))}
      </dl>
      {!hasEdgar && (
        <p className="mt-2 text-[10px] leading-4 text-amber-600">
          No SEC filings found for this listing — figures are Yahoo’s latest-quarter market data,
          not audited annual results. Treat with caution.
        </p>
      )}
    </div>
  );
}

// ---- Watchlist sidebar ------------------------------------------------------

export function WatchlistSidebar({
  watchlists,
  activeId,
  onSelect,
  onDelete,
  onNew,
  onOpenSettings,
  onCheckUpdates,
}: {
  watchlists: Watchlist[];
  activeId: number | null;
  onSelect: (w: Watchlist) => void;
  onDelete: (id: number) => void;
  onNew: () => void;
  onOpenSettings: () => void;
  onCheckUpdates: () => void;
}) {
  return (
    <aside className="flex w-64 shrink-0 flex-col gap-3 border-r border-slate-200 bg-white/60 p-4">
      <div className="flex items-center gap-2 px-1">
        <IconSparkles className="h-5 w-5 text-sky-600" />
        <span className="text-lg font-bold tracking-tight text-slate-900">TrendWave</span>
      </div>

      <button
        onClick={onNew}
        className="flex items-center justify-center gap-1.5 rounded-xl bg-slate-900 px-3 py-2 text-sm font-medium text-white hover:bg-slate-800"
      >
        <IconPlus /> New search
      </button>

      <div className="mt-1 px-1 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
        Saved watchlists
      </div>
      <div className="flex-1 space-y-1 overflow-y-auto">
        {watchlists.length === 0 && (
          <p className="px-1 text-xs text-slate-400">
            Save a search to re-run it later with one click.
          </p>
        )}
        {watchlists.map((w) => (
          <div
            key={w.id}
            className={`group flex items-center gap-1 rounded-xl px-2 py-2 text-sm ${
              activeId === w.id ? "bg-sky-50 text-sky-900" : "text-slate-600 hover:bg-slate-100"
            }`}
          >
            <button onClick={() => onSelect(w)} className="flex-1 truncate text-left" title={w.prompt}>
              {w.name}
            </button>
            <button
              onClick={() => onDelete(w.id)}
              className="opacity-0 transition group-hover:opacity-100"
              title="Delete"
            >
              <IconTrash className="h-3.5 w-3.5 text-slate-400 hover:text-rose-500" />
            </button>
          </div>
        ))}
      </div>

      <div className="space-y-1">
        <button
          onClick={onCheckUpdates}
          className="flex w-full items-center gap-2 rounded-xl px-2 py-2 text-sm text-slate-500 hover:bg-slate-100 hover:text-slate-800"
        >
          <IconDownload /> Check for updates
        </button>
        <button
          onClick={onOpenSettings}
          className="flex w-full items-center gap-2 rounded-xl px-2 py-2 text-sm text-slate-500 hover:bg-slate-100 hover:text-slate-800"
        >
          <IconSettings /> Settings
        </button>
      </div>
    </aside>
  );
}

// ---- Settings modal ---------------------------------------------------------

export function SettingsModal({
  settings,
  onSave,
  onClose,
}: {
  settings: Settings;
  onSave: (s: Settings) => void;
  onClose: () => void;
}) {
  const [form, setForm] = useState<Settings>(settings);
  const update = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onClick={onClose}>
      <div className="w-full max-w-md rounded-3xl bg-white p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-bold text-slate-900">Settings</h2>
          <button onClick={onClose} className="rounded-lg p-1 text-slate-400 hover:bg-slate-100">
            <IconClose />
          </button>
        </div>

        <div className="mt-4 space-y-4">
          <TextField label="Ollama model" value={form.model} onChange={(v) => update("model", v)} placeholder="llama3.1:8b" />
          <TextField label="Ollama endpoint" value={form.ollama_endpoint} onChange={(v) => update("ollama_endpoint", v)} placeholder="http://localhost:11434" />
          <NumberField label="Max results" value={form.max_results} onChange={(v) => update("max_results", v)} step={1} />
          <label className="flex items-center gap-2 text-sm text-slate-700">
            <input type="checkbox" checked={form.use_news} onChange={(e) => update("use_news", e.target.checked)} className="h-4 w-4 rounded border-slate-300" />
            Scan news & compute sentiment (slower)
          </label>
          <label className="flex items-center gap-2 text-sm text-slate-700">
            <input type="checkbox" checked={form.use_fundamentals} onChange={(e) => update("use_fundamentals", e.target.checked)} className="h-4 w-4 rounded border-slate-300" />
            Research real growth (SEC EDGAR + Yahoo) to rank picks
          </label>
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button onClick={onClose} className="rounded-xl px-4 py-2 text-sm font-medium text-slate-600 hover:bg-slate-100">
            Cancel
          </button>
          <button onClick={() => onSave(form)} className="rounded-xl bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-800">
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

function TextField({ label, value, onChange, placeholder }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <label className="block">
      <span className="text-xs font-semibold uppercase tracking-wider text-slate-400">{label}</span>
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2 text-sm focus:border-sky-400 focus:outline-none"
      />
    </label>
  );
}

function NumberField({ label, value, onChange, step }: { label: string; value: number; onChange: (v: number) => void; step?: number }) {
  return (
    <label className="block">
      <span className="text-xs font-semibold uppercase tracking-wider text-slate-400">{label}</span>
      <input
        type="number"
        value={value}
        step={step}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2 text-sm focus:border-sky-400 focus:outline-none"
      />
    </label>
  );
}
