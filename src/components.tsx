import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  resolveBuyOptions,
  hasAnyBroker,
  lastBroker,
  rememberBroker,
  type BuyOption,
  type BrokerId,
} from "./brokers";
import type {
  AppErrorShape,
  Bottleneck,
  Candidate,
  GrowthData,
  Portfolio,
  QuestradeStatus,
  RobinhoodStatus,
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
export const IconCart = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M2 3h2l2.4 12.2a2 2 0 0 0 2 1.6h7.7a2 2 0 0 0 2-1.6L21 7H6M9 21a1 1 0 1 0 0-2 1 1 0 0 0 0 2zm8 0a1 1 0 1 0 0-2 1 1 0 0 0 0 2z" />
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
export const IconSun = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
  </svg>
);
export const IconMoon = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" />
  </svg>
);
export const IconLock = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" />
  </svg>
);
export const IconChevronLeft = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="m15 18-6-6 6-6" />
  </svg>
);
export const IconChevronRight = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="m9 18 6-6-6-6" />
  </svg>
);
export const IconBriefcase = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <rect x="2" y="7" width="20" height="14" rx="2" /><path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16" />
  </svg>
);
export const IconCheck = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M20 6 9 17l-5-5" />
  </svg>
);
export const IconChip = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <rect x="6" y="6" width="12" height="12" rx="2" /><path d="M9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2" />
  </svg>
);

// ---- Broker brand marks (filled monogram badges, so the two connections are
// visually distinct at a glance) --------------------------------------------
export const IconRobinhood = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
    <rect x="1.5" y="1.5" width="21" height="21" rx="6" fill="#00C805" />
    <text
      x="12"
      y="16.7"
      textAnchor="middle"
      fontFamily="ui-sans-serif, system-ui, -apple-system, sans-serif"
      fontSize="13"
      fontWeight="700"
      fill="#ffffff"
    >
      R
    </text>
  </svg>
);
export const IconQuestrade = ({ className = base }: IconProps) => (
  <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
    <rect x="1.5" y="1.5" width="21" height="21" rx="6" fill="#0B63CE" />
    <text
      x="12"
      y="16.7"
      textAnchor="middle"
      fontFamily="ui-sans-serif, system-ui, -apple-system, sans-serif"
      fontSize="13"
      fontWeight="700"
      fill="#ffffff"
    >
      Q
    </text>
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
  frac > 0.001
    ? "text-emerald-600 dark:text-emerald-400"
    : frac < -0.001
      ? "text-rose-600 dark:text-rose-400"
      : "text-slate-600 dark:text-slate-300";

export const formatVolume = (v: number) => {
  if (v >= 1e9) return `${(v / 1e9).toFixed(1)}B`;
  if (v >= 1e6) return `${(v / 1e6).toFixed(1)}M`;
  if (v >= 1e3) return `${(v / 1e3).toFixed(1)}K`;
  return `${Math.round(v)}`;
};

export const sentimentLabel = (s: number | null | undefined) => {
  if (s == null) return { text: "No signal", color: "text-slate-400 dark:text-slate-500" };
  if (s > 0.25) return { text: "Bullish", color: "text-emerald-600 dark:text-emerald-400" };
  if (s < -0.25) return { text: "Bearish", color: "text-rose-600 dark:text-rose-400" };
  return { text: "Neutral", color: "text-slate-500 dark:text-slate-400" };
};

const severityStyle = (sev: number) => {
  if (sev >= 5) return "bg-rose-100 text-rose-700 ring-rose-200 dark:bg-rose-500/15 dark:text-rose-300 dark:ring-rose-500/25";
  if (sev >= 4) return "bg-orange-100 text-orange-700 ring-orange-200 dark:bg-orange-500/15 dark:text-orange-300 dark:ring-orange-500/25";
  if (sev >= 3) return "bg-amber-100 text-amber-700 ring-amber-200 dark:bg-amber-500/15 dark:text-amber-300 dark:ring-amber-500/25";
  return "bg-slate-100 text-slate-600 ring-slate-200 dark:bg-slate-800 dark:text-slate-300 dark:ring-slate-700";
};

const scoreColor = (score: number) => {
  if (score >= 70) return "text-emerald-600 dark:text-emerald-400";
  if (score >= 55) return "text-sky-600 dark:text-sky-400";
  return "text-slate-500 dark:text-slate-400";
};

// ---- Error banner -----------------------------------------------------------

export function ErrorBanner({ error, onDismiss }: { error: AppErrorShape; onDismiss: () => void }) {
  const needsOllama = error.kind === "ollama_unavailable";
  const modelMissing = error.kind === "model_missing";
  return (
    <div className="rounded-2xl border border-rose-200 bg-rose-50 p-4 text-sm text-rose-800 dark:border-rose-500/30 dark:bg-rose-500/10 dark:text-rose-200">
      <div className="flex items-start gap-3">
        <IconAlert className="mt-0.5 h-5 w-5 shrink-0 text-rose-500" />
        <div className="flex-1">
          <p className="font-semibold">{error.message}</p>
          {(needsOllama || modelMissing) && (
            <div className="mt-2 space-y-1 font-mono text-xs text-rose-700/90 dark:text-rose-300/90">
              {needsOllama && <p># Install: https://ollama.com — then run</p>}
              {needsOllama && <p>ollama serve</p>}
              {modelMissing && <p>ollama pull &lt;model&gt;</p>}
            </div>
          )}
        </div>
        <button onClick={onDismiss} className="rounded-lg p-1 text-rose-400 hover:bg-rose-100 dark:hover:bg-rose-500/20">
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
    <div className="rounded-2xl border border-sky-200 bg-sky-50 p-4 dark:border-sky-500/30 dark:bg-sky-500/10">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <IconDownload className="h-5 w-5 shrink-0 text-sky-600 dark:text-sky-400" />
          <div>
            <p className="text-sm font-semibold text-sky-900 dark:text-sky-200">
              {phase === "ready" ? `Update ready — v${version}` : `Update available — v${version}`}
            </p>
            <p className="text-xs text-sky-700 dark:text-sky-300">
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
                className="rounded-lg bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700 dark:hover:bg-sky-500"
              >
                Download &amp; install
              </button>
              <button
                onClick={onDismiss}
                className="rounded-lg px-2 py-1.5 text-sky-700 hover:bg-sky-100 dark:text-sky-300 dark:hover:bg-sky-500/20"
                title="Dismiss"
              >
                <IconClose className="h-3.5 w-3.5" />
              </button>
            </>
          )}
          {phase === "downloading" && <IconSpinner className="h-4 w-4 text-sky-600 dark:text-sky-400" />}
          {phase === "ready" && (
            <button
              onClick={onRestart}
              className="rounded-lg bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700 dark:hover:bg-sky-500"
            >
              Restart now
            </button>
          )}
        </div>
      </div>
      {phase === "downloading" && (
        <div className="mt-3 h-1.5 w-full overflow-hidden rounded-full bg-sky-100 dark:bg-sky-500/20">
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
    <div className="rounded-2xl border border-slate-200 bg-white/70 p-4 dark:border-slate-800 dark:bg-slate-900/60">
      <ul className="space-y-2 text-sm">
        {messages.map((m, i) => {
          const isLast = i === messages.length - 1;
          return (
            <li key={i} className="flex items-center gap-2 text-slate-600 dark:text-slate-300">
              {running && isLast ? (
                <IconSpinner className="h-4 w-4 text-sky-500" />
              ) : (
                <span className="flex h-4 w-4 items-center justify-center text-emerald-500">✓</span>
              )}
              <span className={running && isLast ? "text-slate-900 dark:text-slate-100" : ""}>{m}</span>
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
      <h3 className="text-sm font-semibold uppercase tracking-[0.2em] text-slate-500 dark:text-slate-400">
        Identified bottlenecks
      </h3>
      <div className="grid gap-3 md:grid-cols-2">
        {items.map((b, i) => (
          <div key={i} className="rounded-2xl border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-900">
            <div className="flex items-start justify-between gap-2">
              <h4 className="font-semibold text-slate-900 dark:text-slate-100">{b.title}</h4>
              <span className={`shrink-0 rounded-full px-2 py-0.5 text-xs font-medium ring-1 ${severityStyle(b.severity)}`}>
                Severity {b.severity}/5
              </span>
            </div>
            <p className="mt-2 text-sm leading-6 text-slate-600 dark:text-slate-300">{b.description}</p>
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
      className="group inline-flex items-start gap-1.5 text-left text-sm text-slate-600 hover:text-sky-700 dark:text-slate-300 dark:hover:text-sky-400"
    >
      <IconExternal className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-400 group-hover:text-sky-600 dark:group-hover:text-sky-400" />
      <span className="underline-offset-2 group-hover:underline">{children}</span>
    </button>
  );
}

export function CandidateCard({
  candidate,
  questradeConnected = false,
}: {
  candidate: Candidate;
  questradeConnected?: boolean;
}) {
  const [showNews, setShowNews] = useState(false);
  const senti = sentimentLabel(candidate.sentiment);
  const price = candidate.price;
  return (
    <article className="rounded-3xl border border-slate-200 bg-white p-6 shadow-[0_18px_50px_-40px_rgba(15,23,42,0.5)] dark:border-slate-800 dark:bg-slate-900 dark:shadow-[0_18px_50px_-40px_rgba(0,0,0,0.8)]">
      <header className="flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h3 className="text-xl font-bold tracking-tight text-slate-900 dark:text-slate-100">{candidate.ticker}</h3>
            {price && (
              <span className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                {formatPrice(price.price, price.currency)}
              </span>
            )}
            {price && (
              <span className={`text-sm font-medium ${price.change_pct >= 0 ? "text-emerald-600 dark:text-emerald-400" : "text-rose-600 dark:text-rose-400"}`}>
                {formatPct(price.change_pct)}
              </span>
            )}
          </div>
          <p className="mt-0.5 text-sm text-slate-500 dark:text-slate-400">{candidate.verified_name ?? candidate.company}</p>
        </div>
        <div className="text-right">
          <div className={`text-2xl font-bold ${scoreColor(candidate.score)}`}>
            {Math.round(candidate.score)}
          </div>
          <div className="text-[10px] font-medium uppercase tracking-wider text-slate-400 dark:text-slate-500">score</div>
        </div>
      </header>

      {candidate.identity_mismatch && (
        <div className="mt-3 flex items-start gap-2 rounded-xl bg-amber-50 px-3 py-2 text-xs text-amber-800 ring-1 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-200 dark:ring-amber-500/25">
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
        {candidate.owned && (
          <span className="rounded-full bg-emerald-50 px-2.5 py-1 text-xs font-semibold text-emerald-700 ring-1 ring-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-300 dark:ring-emerald-500/25">
            In your portfolio
          </span>
        )}
        <span className="rounded-full bg-sky-50 px-2.5 py-1 text-xs font-medium text-sky-700 ring-1 ring-sky-100 dark:bg-sky-500/10 dark:text-sky-300 dark:ring-sky-500/20">
          {candidate.bottleneck}
        </span>
        <span className="rounded-full bg-indigo-50 px-2.5 py-1 text-xs font-medium text-indigo-700 ring-1 ring-indigo-100 dark:bg-indigo-500/10 dark:text-indigo-300 dark:ring-indigo-500/20">
          Moat {candidate.moat}/5
        </span>
        {!candidate.identity_mismatch &&
          (candidate.growth?.revenue_growth_yoy != null ? (
            <span
              className={`rounded-full px-2.5 py-1 text-xs font-medium ring-1 ${
                candidate.growth.revenue_growth_yoy >= 0
                  ? "bg-emerald-50 text-emerald-700 ring-emerald-100 dark:bg-emerald-500/10 dark:text-emerald-300 dark:ring-emerald-500/20"
                  : "bg-rose-50 text-rose-700 ring-rose-100 dark:bg-rose-500/10 dark:text-rose-300 dark:ring-rose-500/20"
              }`}
            >
              Rev {formatGrowthPct(candidate.growth.revenue_growth_yoy)} {candidate.growth.annual_growth ? "YoY" : "qtr"}
            </span>
          ) : (
            <span className="rounded-full bg-slate-50 px-2.5 py-1 text-xs font-medium text-slate-600 ring-1 ring-slate-100 dark:bg-slate-800 dark:text-slate-300 dark:ring-slate-700">
              Growth {Math.round(candidate.growth_score * 100)}
            </span>
          ))}
        <span className={`text-xs font-medium ${senti.color}`}>● {senti.text}</span>
        {price && price.avg_volume > 0 && (
          <span className="text-xs text-slate-400 dark:text-slate-500">avg vol {formatVolume(price.avg_volume)}</span>
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
        <p className="mt-4 rounded-xl bg-slate-50 px-3 py-2 text-xs leading-5 text-slate-500 ring-1 ring-slate-200 dark:bg-slate-800/50 dark:text-slate-400 dark:ring-slate-700">
          Financials hidden — these SEC EDGAR / Yahoo figures are{" "}
          <strong className="font-semibold text-slate-600 dark:text-slate-300">{candidate.verified_name ?? candidate.ticker}</strong>’s
          actuals for ticker {candidate.ticker}, and don’t describe the “{candidate.company}” business
          in this thesis. They were excluded so they can’t be read as support for the pick.
        </p>
      ) : candidate.growth ? (
        <GrowthPanel growth={candidate.growth} />
      ) : (
        <p className="mt-3 text-xs text-slate-400 dark:text-slate-500">No growth fundamentals found for this ticker.</p>
      )}

      {candidate.news.length > 0 && (
        <div className="mt-4 border-t border-slate-100 pt-3 dark:border-slate-800">
          <button
            onClick={() => setShowNews((s) => !s)}
            className="text-xs font-medium text-slate-500 hover:text-slate-800 dark:text-slate-400 dark:hover:text-slate-100"
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

      <BuyPanel candidate={candidate} questradeConnected={questradeConnected} />
    </article>
  );
}

function buyOptionLabel(o: BuyOption): string {
  if (o.fxNote === "cad-native") return `${o.label} · ${o.symbol} (CAD, no FX)`;
  if (o.fxNote === "usd-fx") return `${o.label} · ${o.symbol} (USD, FX applies)`;
  return `${o.label} · ${o.symbol}`;
}

function BuyPanel({
  candidate,
  questradeConnected,
}: {
  candidate: Candidate;
  questradeConnected: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [options, setOptions] = useState<BuyOption[] | null>(null);
  const [qty, setQty] = useState(1);
  const [brokerId, setBrokerId] = useState<BrokerId | null>(null);

  if (!hasAnyBroker(candidate.ticker, candidate.price)) return null;

  const price = candidate.price;

  const onToggle = () => {
    const next = !open;
    setOpen(next);
    if (next && options === null && !loading) {
      setLoading(true);
      resolveBuyOptions(candidate, questradeConnected)
        .then((opts) => {
          setOptions(opts);
          const preferred = lastBroker();
          const initial = opts.find((o) => o.id === preferred) ?? opts[0] ?? null;
          setBrokerId(initial ? initial.id : null);
        })
        .finally(() => setLoading(false));
    }
  };

  const selected = options?.find((o) => o.id === brokerId) ?? null;
  const estCost = price && qty > 0 ? qty * price.price : null;

  const onOpenBroker = () => {
    if (!selected) return;
    rememberBroker(selected.id);
    void openUrl(selected.url).catch(() => {});
  };

  return (
    <div className="mt-4 border-t border-slate-100 pt-3 dark:border-slate-800">
      <button
        onClick={onToggle}
        aria-expanded={open}
        className="inline-flex items-center gap-1.5 rounded-full bg-sky-600 px-3.5 py-1.5 text-xs font-semibold text-white shadow-sm transition hover:bg-sky-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 dark:bg-sky-500 dark:hover:bg-sky-400"
      >
        <IconCart className="h-3.5 w-3.5" />
        {open ? "Hide buy options" : "Buy"}
      </button>

      {open && (
        <div className="mt-3 rounded-2xl bg-slate-50 p-4 ring-1 ring-slate-200 dark:bg-slate-800/40 dark:ring-slate-700">
          {loading ? (
            <div className="flex items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
              <IconSpinner className="h-4 w-4 text-sky-500" />
              Checking availability…
            </div>
          ) : !options || options.length === 0 ? (
            <p className="text-xs text-slate-500 dark:text-slate-400">
              Not available at a supported brokerage.
            </p>
          ) : (
            <div className="space-y-3">
              <div className="flex flex-wrap items-end gap-3">
                <label className="flex flex-col gap-1">
                  <span className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">Quantity</span>
                  <input
                    type="number"
                    min={1}
                    step={1}
                    value={qty}
                    onChange={(e) => {
                      const n = Math.floor(Number(e.target.value));
                      setQty(Number.isFinite(n) && n > 0 ? n : 1);
                    }}
                    className="w-24 rounded-lg border border-slate-300 bg-white px-2.5 py-1.5 text-sm text-slate-900 focus:border-sky-400 focus:outline-none focus:ring-2 focus:ring-sky-200 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100 dark:focus:ring-sky-500/30"
                  />
                </label>
                <label className="flex min-w-[12rem] flex-1 flex-col gap-1">
                  <span className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">Brokerage</span>
                  <select
                    value={brokerId ?? ""}
                    onChange={(e) => setBrokerId(e.target.value as BrokerId)}
                    className="w-full rounded-lg border border-slate-300 bg-white px-2.5 py-1.5 text-sm text-slate-900 focus:border-sky-400 focus:outline-none focus:ring-2 focus:ring-sky-200 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100 dark:focus:ring-sky-500/30"
                  >
                    {options.map((o) => (
                      <option key={o.id} value={o.id}>
                        {buyOptionLabel(o)}
                      </option>
                    ))}
                  </select>
                </label>
              </div>

              {estCost != null && price && (
                <p className="text-xs text-slate-500 dark:text-slate-400">
                  Estimated cost ≈{" "}
                  <span className="font-semibold text-slate-700 dark:text-slate-200">{formatPrice(estCost, price.currency)}</span>{" "}
                  ({qty} × {formatPrice(price.price, price.currency)})
                  {selected?.fxNote === "cad-native" && price.currency !== "CAD" && (
                    <> — trades in CAD, FX-free; final CAD cost set at the market.</>
                  )}
                  {selected?.fxNote === "usd-fx" && <> — trades in USD; your broker applies FX.</>}
                </p>
              )}

              <button
                onClick={onOpenBroker}
                disabled={!selected}
                className="inline-flex items-center gap-1.5 rounded-full bg-slate-900 px-4 py-2 text-xs font-semibold text-white shadow-sm transition hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 dark:bg-white dark:text-slate-900 dark:hover:bg-slate-200"
              >
                <IconExternal className="h-3.5 w-3.5" />
                Open in {selected?.label ?? "broker"}
              </button>

              <p className="text-[11px] leading-5 text-slate-400 dark:text-slate-500">
                Opens {selected?.label ?? "the broker"}’s page for {selected?.symbol ?? candidate.ticker} in your
                browser so you can review and place the order there — enter the quantity to confirm. Some brokers
                require sign-in. TrendWave never places trades. Not financial advice.
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">{label}</dt>
      <dd className="mt-0.5 text-sm leading-6 text-slate-700 dark:text-slate-200">{value}</dd>
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
      tone: growth.profitable ? "text-emerald-600 dark:text-emerald-400" : "text-rose-600 dark:text-rose-400",
    });
  if (growth.analyst_upside != null)
    stats.push({
      label: "Analyst target",
      value: formatGrowthPct(growth.analyst_upside),
      tone: growthTone(growth.analyst_upside),
    });
  if (growth.forward_pe != null)
    stats.push({ label: "Forward P/E", value: growth.forward_pe.toFixed(1), tone: "text-slate-700 dark:text-slate-200" });

  if (stats.length === 0) return null;
  return (
    <div className="mt-4 rounded-2xl bg-slate-50 p-3 ring-1 ring-slate-100 dark:bg-slate-800/50 dark:ring-slate-700">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">
          Growth research
        </span>
        {growth.source && <span className="text-[10px] text-slate-400 dark:text-slate-500">via {growth.source}</span>}
      </div>
      <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-3">
        {stats.map((s) => (
          <div key={s.label}>
            <dt className="text-[10px] uppercase tracking-wide text-slate-400 dark:text-slate-500">{s.label}</dt>
            <dd className={`text-sm font-semibold ${s.tone}`}>{s.value}</dd>
          </div>
        ))}
      </dl>
      {!hasEdgar && (
        <p className="mt-2 text-[10px] leading-4 text-amber-600 dark:text-amber-400">
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
  theme,
  onToggleTheme,
}: {
  watchlists: Watchlist[];
  activeId: number | null;
  onSelect: (w: Watchlist) => void;
  onDelete: (id: number) => void;
  onNew: () => void;
  onOpenSettings: () => void;
  onCheckUpdates: () => void;
  theme: "light" | "dark";
  onToggleTheme: () => void;
}) {
  return (
    <aside className="flex w-64 shrink-0 flex-col gap-3 border-r border-slate-200 bg-white/60 p-4 dark:border-slate-800 dark:bg-slate-900/60">
      <div className="flex items-center gap-2 px-1">
        <IconSparkles className="h-5 w-5 text-sky-600 dark:text-sky-400" />
        <span className="text-lg font-bold tracking-tight text-slate-900 dark:text-slate-100">TrendWave</span>
      </div>

      <button
        onClick={onNew}
        className="flex items-center justify-center gap-1.5 rounded-xl bg-slate-900 px-3 py-2 text-sm font-medium text-white hover:bg-slate-800 dark:bg-sky-600 dark:hover:bg-sky-500"
      >
        <IconPlus /> New search
      </button>

      <div className="mt-1 px-1 text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">
        Saved watchlists
      </div>
      <div className="flex-1 space-y-1 overflow-y-auto">
        {watchlists.length === 0 && (
          <p className="px-1 text-xs text-slate-400 dark:text-slate-500">
            Save a search to re-run it later with one click.
          </p>
        )}
        {watchlists.map((w) => (
          <div
            key={w.id}
            className={`group flex items-center gap-1 rounded-xl px-2 py-2 text-sm ${
              activeId === w.id
                ? "bg-sky-50 text-sky-900 dark:bg-sky-500/15 dark:text-sky-200"
                : "text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
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
          onClick={onToggleTheme}
          className="flex w-full items-center gap-2 rounded-xl px-2 py-2 text-sm text-slate-500 hover:bg-slate-100 hover:text-slate-800 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100"
        >
          {theme === "dark" ? <IconSun /> : <IconMoon />}
          {theme === "dark" ? "Light mode" : "Dark mode"}
        </button>
        <button
          onClick={onCheckUpdates}
          className="flex w-full items-center gap-2 rounded-xl px-2 py-2 text-sm text-slate-500 hover:bg-slate-100 hover:text-slate-800 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100"
        >
          <IconDownload /> Check for updates
        </button>
        <button
          onClick={onOpenSettings}
          className="flex w-full items-center gap-2 rounded-xl px-2 py-2 text-sm text-slate-500 hover:bg-slate-100 hover:text-slate-800 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100"
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
  robinhood,
  robinhoodBusy,
  onConnectRobinhood,
  onDisconnectRobinhood,
  biometricAvailable,
  questrade,
  questradeBusy,
  onConnectQuestrade,
  onDisconnectQuestrade,
}: {
  settings: Settings;
  onSave: (s: Settings) => void;
  onClose: () => void;
  robinhood: RobinhoodStatus | null;
  robinhoodBusy: boolean;
  onConnectRobinhood: () => void;
  onDisconnectRobinhood: () => void;
  biometricAvailable: boolean;
  questrade: QuestradeStatus | null;
  questradeBusy: boolean;
  onConnectQuestrade: (token: string) => void;
  onDisconnectQuestrade: () => void;
}) {
  const [form, setForm] = useState<Settings>(settings);
  const update = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 dark:bg-black/60" onClick={onClose}>
      <div className="w-full max-w-md rounded-3xl bg-white p-6 shadow-2xl dark:bg-slate-900 dark:ring-1 dark:ring-slate-800" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-bold text-slate-900 dark:text-slate-100">Settings</h2>
          <button onClick={onClose} className="rounded-lg p-1 text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-800">
            <IconClose />
          </button>
        </div>

        <div className="mt-4 space-y-4">
          <TextField label="Ollama model" value={form.model} onChange={(v) => update("model", v)} placeholder="llama3.1:8b" />
          <TextField label="Ollama endpoint" value={form.ollama_endpoint} onChange={(v) => update("ollama_endpoint", v)} placeholder="http://localhost:11434" />
          <NumberField label="Max results" value={form.max_results} onChange={(v) => update("max_results", v)} step={1} />
          <label className="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-200">
            <input type="checkbox" checked={form.use_news} onChange={(e) => update("use_news", e.target.checked)} className="h-4 w-4 rounded border-slate-300 dark:border-slate-600 dark:bg-slate-800" />
            Scan news & compute sentiment (slower)
          </label>
          <label className="flex items-center gap-2 text-sm text-slate-700 dark:text-slate-200">
            <input type="checkbox" checked={form.use_fundamentals} onChange={(e) => update("use_fundamentals", e.target.checked)} className="h-4 w-4 rounded border-slate-300 dark:border-slate-600 dark:bg-slate-800" />
            Research real growth (SEC EDGAR + Yahoo) to rank picks
          </label>
          <RobinhoodSection
            status={robinhood}
            busy={robinhoodBusy}
            onConnect={onConnectRobinhood}
            onDisconnect={onDisconnectRobinhood}
            biometricAvailable={biometricAvailable}
            requireBiometric={form.require_biometric_unlock}
            onToggleRequireBiometric={(v) => update("require_biometric_unlock", v)}
          />
          <QuestradeSection
            status={questrade}
            busy={questradeBusy}
            onConnect={onConnectQuestrade}
            onDisconnect={onDisconnectQuestrade}
          />
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button onClick={onClose} className="rounded-xl px-4 py-2 text-sm font-medium text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800">
            Cancel
          </button>
          <button onClick={() => onSave(form)} className="rounded-xl bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-800 dark:bg-sky-600 dark:hover:bg-sky-500">
            Save
          </button>
        </div>
      </div>
    </div>
  );
}

// ---- Broker connections (read-only) ----------------------------------------

export type BrokerKind = "robinhood" | "questrade";

function BrokerIcon({ broker, className }: { broker: BrokerKind; className?: string }) {
  return broker === "robinhood" ? (
    <IconRobinhood className={className} />
  ) : (
    <IconQuestrade className={className} />
  );
}

const brokerLabel = (broker: BrokerKind) =>
  broker === "robinhood" ? "Robinhood" : "Questrade";

function RobinhoodSection({
  status,
  busy,
  onConnect,
  onDisconnect,
  biometricAvailable,
  requireBiometric,
  onToggleRequireBiometric,
}: {
  status: RobinhoodStatus | null;
  busy: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  biometricAvailable: boolean;
  requireBiometric: boolean;
  onToggleRequireBiometric: (v: boolean) => void;
}) {
  const connected = status?.connected ?? false;
  return (
    <div className="rounded-2xl border border-slate-200 p-3 dark:border-slate-800">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <IconRobinhood className="h-6 w-6 shrink-0" />
          <div>
            <p className="text-sm font-semibold text-slate-900 dark:text-slate-100">Robinhood</p>
            <p className="text-xs text-slate-500 dark:text-slate-400">
              {connected
                ? "Connected · read-only"
                : "Use your holdings as research context"}
            </p>
          </div>
        </div>
        {connected ? (
          <button
            onClick={onDisconnect}
            disabled={busy}
            className="rounded-lg border border-slate-200 px-3 py-1.5 text-xs font-medium text-slate-600 hover:bg-slate-50 disabled:opacity-40 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-800"
          >
            Disconnect
          </button>
        ) : (
          <button
            onClick={onConnect}
            disabled={busy}
            className="rounded-lg bg-slate-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-800 disabled:opacity-40 dark:bg-sky-600 dark:hover:bg-sky-500"
          >
            {busy ? "Connecting…" : "Connect"}
          </button>
        )}
      </div>
      <p className="mt-2 text-[11px] leading-4 text-slate-400 dark:text-slate-500">
        Read-only. TrendWave reads your positions to flag picks you already own — it can’t place,
        modify, or cancel trades. A browser window opens for you to sign in to Robinhood.
      </p>
      {biometricAvailable && (
        <label className="mt-3 flex items-start gap-2 border-t border-slate-100 pt-3 text-sm text-slate-700 dark:border-slate-800 dark:text-slate-200">
          <input
            type="checkbox"
            checked={requireBiometric}
            onChange={(e) => onToggleRequireBiometric(e.target.checked)}
            className="mt-0.5 h-4 w-4 rounded border-slate-300 dark:border-slate-600 dark:bg-slate-800"
          />
          <span>
            Require Touch ID / Windows Hello on launch
            <span className="mt-0.5 block text-[11px] font-normal leading-4 text-slate-400 dark:text-slate-500">
              Unlock with biometrics before your saved Robinhood session is shown.
            </span>
          </span>
        </label>
      )}
    </div>
  );
}

function QuestradeSection({
  status,
  busy,
  onConnect,
  onDisconnect,
}: {
  status: QuestradeStatus | null;
  busy: boolean;
  onConnect: (token: string) => void;
  onDisconnect: () => void;
}) {
  const connected = status?.connected ?? false;
  const [token, setToken] = useState("");

  return (
    <div className="rounded-2xl border border-slate-200 p-3 dark:border-slate-800">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <IconQuestrade className="h-6 w-6 shrink-0" />
          <div>
            <p className="text-sm font-semibold text-slate-900 dark:text-slate-100">Questrade</p>
            <p className="text-xs text-slate-500 dark:text-slate-400">
              {connected
                ? "Connected · read-only"
                : "Use your holdings as research context"}
            </p>
          </div>
        </div>
        {connected && (
          <button
            onClick={onDisconnect}
            disabled={busy}
            className="rounded-lg border border-slate-200 px-3 py-1.5 text-xs font-medium text-slate-600 hover:bg-slate-50 disabled:opacity-40 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-800"
          >
            Disconnect
          </button>
        )}
      </div>

      {connected ? (
        <p className="mt-2 text-[11px] leading-4 text-slate-400 dark:text-slate-500">
          Read-only. TrendWave reads your positions to flag picks you already own — it can’t place,
          modify, or cancel trades.
        </p>
      ) : (
        <div className="mt-3 space-y-2">
          <input
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && token.trim()) onConnect(token.trim());
            }}
            placeholder="Manual authorization token"
            className="w-full rounded-xl border border-slate-200 px-3 py-2 text-sm focus:border-sky-400 focus:outline-none dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100 dark:placeholder:text-slate-500"
          />
          <button
            onClick={() => onConnect(token.trim())}
            disabled={busy || !token.trim()}
            className="w-full rounded-lg bg-slate-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-800 disabled:opacity-40 dark:bg-sky-600 dark:hover:bg-sky-500"
          >
            {busy ? "Connecting…" : "Connect"}
          </button>
          <p className="text-[11px] leading-4 text-slate-400 dark:text-slate-500">
            In Questrade, open{" "}
            <button
              onClick={() =>
                openUrl("https://www.questrade.com/api/documentation/getting-started").catch(
                  () => {},
                )
              }
              className="font-medium text-sky-600 hover:underline dark:text-sky-400"
            >
              API centre
            </button>{" "}
            → register a personal app → generate a manual authorization token, then paste it above.
            Read-only: TrendWave can’t place, modify, or cancel trades.
          </p>
        </div>
      )}
    </div>
  );
}

/// A tiny inline price sparkline, stroked green when up / red when down.
function Sparkline({ data, positive }: { data: number[]; positive: boolean }) {
  if (!data || data.length < 2) return null;
  const w = 60;
  const h = 20;
  const pad = 2;
  const min = Math.min(...data);
  const max = Math.max(...data);
  const range = max - min || 1;
  const points = data
    .map((v, i) => {
      const x = pad + (i / (data.length - 1)) * (w - pad * 2);
      const y = pad + (1 - (v - min) / range) * (h - pad * 2);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  const stroke = positive ? "#10b981" : "#ef4444";
  return (
    <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden="true" className="shrink-0">
      <polyline
        points={points}
        fill="none"
        stroke={stroke}
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export interface BrokerTab {
  broker: BrokerKind;
  content: ReactNode;
}

/// Switches between connected brokers' portfolio tables. A single broker renders
/// its panel bare; two or more get a pill tab bar and a horizontal slide (the
/// active panel's measured height drives the container so the slide stays smooth
/// even when the two tables differ in length).
export function BrokerPortfolioTabs({ tabs }: { tabs: BrokerTab[] }) {
  const [active, setActive] = useState(0);
  const safeActive = Math.min(active, Math.max(tabs.length - 1, 0));
  const panelRefs = useRef<(HTMLDivElement | null)[]>([]);
  const [height, setHeight] = useState<number | undefined>(undefined);

  useLayoutEffect(() => {
    const el = panelRefs.current[safeActive];
    if (!el) return;
    const measure = () => setHeight(el.offsetHeight);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [safeActive, tabs.length]);

  if (tabs.length === 0) return null;
  if (tabs.length === 1) return <>{tabs[0].content}</>;

  return (
    <section>
      <div
        role="tablist"
        aria-label="Connected brokers"
        className="mb-3 flex gap-1 rounded-2xl bg-slate-100 p-1 dark:bg-slate-800/60"
      >
        {tabs.map((t, i) => {
          const selected = safeActive === i;
          return (
            <button
              key={t.broker}
              role="tab"
              aria-selected={selected}
              onClick={() => setActive(i)}
              className={`flex flex-1 items-center justify-center gap-2 rounded-xl px-3 py-1.5 text-xs font-semibold transition-colors ${
                selected
                  ? "bg-white text-slate-900 shadow-sm dark:bg-slate-900 dark:text-slate-100"
                  : "text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200"
              }`}
            >
              <BrokerIcon broker={t.broker} className="h-4 w-4 shrink-0" />
              {brokerLabel(t.broker)}
            </button>
          );
        })}
      </div>
      <div
        className="relative overflow-hidden transition-[height] duration-300 ease-out motion-reduce:transition-none"
        style={{ height }}
      >
        <div
          className="flex transition-transform duration-300 ease-out motion-reduce:transition-none"
          style={{ transform: `translateX(-${safeActive * 100}%)` }}
        >
          {tabs.map((t, i) => (
            <div
              key={t.broker}
              ref={(el) => {
                panelRefs.current[i] = el;
              }}
              aria-hidden={safeActive !== i}
              className="w-full shrink-0 self-start"
            >
              {t.content}
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

/// Right-side rail that keeps the connected brokers' portfolio tables out of the
/// main column (so search results stay at the top of the page) while remaining a
/// click away. Collapses to a slim bar; the choice is remembered across launches.
const RAIL_STORAGE_KEY = "trendwave-portfolio-rail";

function getRailCollapsed(): boolean {
  try {
    return localStorage.getItem(RAIL_STORAGE_KEY) === "collapsed";
  } catch {
    return false;
  }
}

export function PortfolioRail({ tabs }: { tabs: BrokerTab[] }) {
  const [collapsed, setCollapsed] = useState(getRailCollapsed);

  const toggle = () =>
    setCollapsed((c) => {
      const next = !c;
      try {
        localStorage.setItem(RAIL_STORAGE_KEY, next ? "collapsed" : "open");
      } catch {
        /* persistence is best-effort */
      }
      return next;
    });

  if (tabs.length === 0) return null;

  if (collapsed) {
    return (
      <aside className="flex w-12 shrink-0 flex-col items-center gap-3 border-l border-slate-200 bg-white/60 py-3 dark:border-slate-800 dark:bg-slate-900/60">
        <button
          onClick={toggle}
          aria-expanded={false}
          title="Show portfolio"
          className="flex h-8 w-8 items-center justify-center rounded-lg text-slate-500 hover:bg-slate-100 hover:text-slate-800 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100"
        >
          <IconChevronLeft className="h-4 w-4" />
        </button>
        <div className="h-px w-6 bg-slate-200 dark:bg-slate-800" />
        <div className="flex flex-col items-center gap-2">
          {tabs.map((t) => (
            <button
              key={t.broker}
              onClick={toggle}
              title={`Show ${brokerLabel(t.broker)} portfolio`}
              className="flex h-8 w-8 items-center justify-center rounded-lg hover:bg-slate-100 dark:hover:bg-slate-800"
            >
              <BrokerIcon broker={t.broker} className="h-5 w-5" />
            </button>
          ))}
        </div>
        <span className="mt-1 text-[10px] font-semibold uppercase tracking-wider text-slate-400 [writing-mode:vertical-rl] dark:text-slate-500">
          Portfolio
        </span>
      </aside>
    );
  }

  return (
    <aside className="flex w-96 shrink-0 flex-col border-l border-slate-200 bg-white/60 dark:border-slate-800 dark:bg-slate-900/60">
      <div className="flex items-center justify-between gap-2 border-b border-slate-200 px-4 py-3 dark:border-slate-800">
        <div className="flex items-center gap-2">
          <IconBriefcase className="h-5 w-5 text-sky-600 dark:text-sky-400" />
          <span className="text-sm font-semibold text-slate-900 dark:text-slate-100">Portfolio</span>
        </div>
        <button
          onClick={toggle}
          aria-expanded={true}
          title="Collapse portfolio"
          className="flex h-8 w-8 items-center justify-center rounded-lg text-slate-500 hover:bg-slate-100 hover:text-slate-800 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100"
        >
          <IconChevronRight className="h-4 w-4" />
        </button>
      </div>
      <div className="flex-1 overflow-y-auto p-4">
        <BrokerPortfolioTabs tabs={tabs} />
      </div>
    </aside>
  );
}

export function PortfolioPanel({
  portfolio,
  busy,
  onRefresh,
  broker,
}: {
  portfolio: Portfolio;
  busy: boolean;
  onRefresh: () => void;
  broker: BrokerKind;
}) {
  const acct = portfolio.account;
  const held = portfolio.positions
    .filter((p) => p.quantity > 0)
    .sort((a, b) => (b.market_value ?? 0) - (a.market_value ?? 0));

  const stat = (label: string, value: number | null | undefined, currency: string) =>
    value == null ? null : (
      <div>
        <div className="text-[10px] font-medium uppercase tracking-wider text-slate-400 dark:text-slate-500">
          {label}
        </div>
        <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">
          {formatPrice(value, currency)}
        </div>
      </div>
    );

  return (
    <section className="rounded-3xl border border-slate-200 bg-white p-5 shadow-[0_18px_50px_-40px_rgba(15,23,42,0.5)] dark:border-slate-800 dark:bg-slate-900 dark:shadow-[0_18px_50px_-40px_rgba(0,0,0,0.8)]">
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          <BrokerIcon broker={broker} className="h-5 w-5 shrink-0" />
          <h2 className="text-sm font-semibold uppercase tracking-[0.2em] text-slate-500 dark:text-slate-400">
            Your {brokerLabel(broker)} portfolio
          </h2>
          <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[10px] font-medium text-emerald-700 ring-1 ring-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-300 dark:ring-emerald-500/25">
            Read-only
          </span>
        </div>
        <button
          onClick={onRefresh}
          disabled={busy}
          className="rounded-lg border border-slate-200 px-2.5 py-1 text-xs font-medium text-slate-600 hover:bg-slate-50 disabled:opacity-40 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-800"
        >
          {busy ? "Refreshing…" : "Refresh"}
        </button>
      </div>

      {acct && (
        <div className="mt-3 flex flex-wrap gap-6">
          {stat("Portfolio value", acct.portfolio_value, acct.currency)}
          {stat("Buying power", acct.buying_power, acct.currency)}
          {stat("Cash", acct.cash, acct.currency)}
        </div>
      )}

      {held.length > 0 ? (
        <div className="mt-4 overflow-hidden rounded-2xl ring-1 ring-slate-100 dark:ring-slate-800">
          <table className="w-full text-sm">
            <thead>
              <tr className="bg-slate-50 text-left text-[10px] uppercase tracking-wider text-slate-400 dark:bg-slate-800/60 dark:text-slate-500">
                <th className="px-3 py-2 font-medium">Ticker</th>
                <th className="px-3 py-2 text-right font-medium">Today</th>
                <th className="px-3 py-2 text-right font-medium">Qty</th>
                <th className="px-3 py-2 text-right font-medium">Value</th>
              </tr>
            </thead>
            <tbody>
              {held.map((p) => {
                const chg = p.change_pct;
                const positive =
                  chg != null
                    ? chg >= 0
                    : p.spark && p.spark.length > 1
                      ? p.spark[p.spark.length - 1] >= p.spark[0]
                      : true;
                return (
                  <tr key={p.ticker} className="border-t border-slate-100 dark:border-slate-800">
                    <td className="px-3 py-2">
                      <span className="font-semibold text-slate-900 dark:text-slate-100">{p.ticker}</span>
                      {p.name && (
                        <span className="ml-2 text-xs text-slate-400 dark:text-slate-500">{p.name}</span>
                      )}
                    </td>
                    <td className="px-3 py-2">
                      <div className="flex items-center justify-end gap-2">
                        {p.spark && p.spark.length > 1 && (
                          <Sparkline data={p.spark} positive={positive} />
                        )}
                        {chg != null && (
                          <span
                            className={`text-xs font-medium tabular-nums ${
                              positive
                                ? "text-emerald-600 dark:text-emerald-400"
                                : "text-red-600 dark:text-red-400"
                            }`}
                          >
                            {positive ? "+" : ""}
                            {chg.toFixed(2)}%
                          </span>
                        )}
                        {chg == null && (!p.spark || p.spark.length < 2) && (
                          <span className="text-xs text-slate-300 dark:text-slate-600">—</span>
                        )}
                      </div>
                    </td>
                    <td className="px-3 py-2 text-right tabular-nums text-slate-600 dark:text-slate-300">
                      {p.quantity}
                    </td>
                    <td className="px-3 py-2 text-right tabular-nums text-slate-900 dark:text-slate-100">
                      {p.market_value == null ? "—" : formatPrice(p.market_value, p.currency)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="mt-3 text-sm text-slate-500 dark:text-slate-400">No equity positions found.</p>
      )}

      <p className="mt-2 text-[10px] text-slate-400 dark:text-slate-500">
        As of {new Date(portfolio.as_of).toLocaleString()}
        {portfolio.tools_used.length > 0 && ` · via ${portfolio.tools_used.join(", ")}`}
      </p>
      {portfolio.debug && portfolio.debug.length > 0 && (
        <p className="mt-1 text-[10px] text-amber-600 dark:text-amber-400/80">
          Live values unavailable for some rows — {portfolio.debug.join("; ")}
        </p>
      )}
    </section>
  );
}

export function PortfolioLocked({ busy, onUnlock }: { busy: boolean; onUnlock: () => void }) {
  return (
    <section className="rounded-3xl border border-slate-200 bg-white p-5 shadow-[0_18px_50px_-40px_rgba(15,23,42,0.5)] dark:border-slate-800 dark:bg-slate-900 dark:shadow-[0_18px_50px_-40px_rgba(0,0,0,0.8)]">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <IconRobinhood className="h-5 w-5 shrink-0" />
          <h2 className="text-sm font-semibold uppercase tracking-[0.2em] text-slate-500 dark:text-slate-400">
            Your Robinhood portfolio
          </h2>
          <span className="flex items-center gap-1 rounded-full bg-amber-50 px-2 py-0.5 text-[10px] font-medium text-amber-700 ring-1 ring-amber-200 dark:bg-amber-500/10 dark:text-amber-300 dark:ring-amber-500/25">
            <IconLock className="h-3 w-3" /> Locked
          </span>
        </div>
        <button
          onClick={onUnlock}
          disabled={busy}
          className="rounded-lg bg-slate-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-800 disabled:opacity-40 dark:bg-sky-600 dark:hover:bg-sky-500"
        >
          {busy ? "Unlocking…" : "Unlock"}
        </button>
      </div>
      <p className="mt-3 text-sm text-slate-500 dark:text-slate-400">
        Your saved Robinhood session is protected. Unlock with Touch ID or Windows Hello to view your
        positions — research still works without unlocking.
      </p>
    </section>
  );
}

export function PortfolioEmpty({
  busy,
  onLoad,
  broker,
}: {
  busy: boolean;
  onLoad: () => void;
  broker: BrokerKind;
}) {
  return (
    <section className="rounded-3xl border border-slate-200 bg-white p-5 shadow-[0_18px_50px_-40px_rgba(15,23,42,0.5)] dark:border-slate-800 dark:bg-slate-900 dark:shadow-[0_18px_50px_-40px_rgba(0,0,0,0.8)]">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <BrokerIcon broker={broker} className="h-5 w-5 shrink-0" />
          <h2 className="text-sm font-semibold uppercase tracking-[0.2em] text-slate-500 dark:text-slate-400">
            Your {brokerLabel(broker)} portfolio
          </h2>
          <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[10px] font-medium text-emerald-700 ring-1 ring-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-300 dark:ring-emerald-500/25">
            Connected · read-only
          </span>
        </div>
        <button
          onClick={onLoad}
          disabled={busy}
          className="rounded-lg bg-slate-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-slate-800 disabled:opacity-40 dark:bg-sky-600 dark:hover:bg-sky-500"
        >
          {busy ? "Loading…" : "Load portfolio"}
        </button>
      </div>
      <p className="mt-3 text-sm text-slate-500 dark:text-slate-400">
        No snapshot loaded yet. Click <span className="font-medium">Load portfolio</span> to pull your
        positions — they’ll be used (read-only) to badge picks you already own.
      </p>
    </section>
  );
}

function TextField({ label, value, onChange, placeholder }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string }) {
  return (
    <label className="block">
      <span className="text-xs font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">{label}</span>
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2 text-sm focus:border-sky-400 focus:outline-none dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100 dark:placeholder:text-slate-500"
      />
    </label>
  );
}

function NumberField({ label, value, onChange, step }: { label: string; value: number; onChange: (v: number) => void; step?: number }) {
  return (
    <label className="block">
      <span className="text-xs font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">{label}</span>
      <input
        type="number"
        value={value}
        step={step}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-1 w-full rounded-xl border border-slate-200 px-3 py-2 text-sm focus:border-sky-400 focus:outline-none dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100"
      />
    </label>
  );
}
