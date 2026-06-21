import { useEffect, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as api from "./api";
import {
  IconAlert,
  IconCheck,
  IconChevronLeft,
  IconChip,
  IconDownload,
  IconExternal,
  IconLock,
  IconRefresh,
  IconSearch,
  IconSparkles,
  IconSpinner,
} from "./components";
import type { ModelOption, OllamaStatus, SystemReport, SystemSpecs } from "./types";

const OLLAMA_DOWNLOAD_URL = "https://ollama.com/download";
const TOTAL_STEPS = 5;

/// First-run setup wizard: agree to terms, set up the local model, and learn the
/// basics. Rendered in place of the app until `onComplete` resolves.
export default function Onboarding({
  onComplete,
}: {
  onComplete: (model: string) => Promise<void>;
}) {
  const [step, setStep] = useState(0);
  const [agreed, setAgreed] = useState(false);
  const [report, setReport] = useState<SystemReport | null>(null);
  const [ollama, setOllama] = useState<OllamaStatus | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [checking, setChecking] = useState(false);
  const [finishing, setFinishing] = useState(false);

  useEffect(() => {
    api
      .systemReport()
      .then((r) => {
        setReport(r);
        setSelectedModel((m) => m || r.recommended_id);
      })
      .catch(() => {});
    refreshOllama();
  }, []);

  const refreshOllama = async () => {
    setChecking(true);
    try {
      setOllama(await api.ollamaStatus());
    } catch {
      /* detection is best-effort; the app still works once Ollama is up */
    } finally {
      setChecking(false);
    }
  };

  const next = () => setStep((s) => Math.min(s + 1, TOTAL_STEPS - 1));
  const back = () => setStep((s) => Math.max(s - 1, 0));

  const finish = async () => {
    setFinishing(true);
    try {
      await onComplete(selectedModel);
    } finally {
      setFinishing(false);
    }
  };

  const isLast = step === TOTAL_STEPS - 1;
  const blockContinue = step === 1 && !agreed;

  return (
    <div className="flex h-full w-full items-center justify-center bg-[radial-gradient(circle_at_top,_#eff6ff,_#f8fafc_60%)] p-4 text-slate-900 dark:bg-[radial-gradient(circle_at_top,_#0b1220,_#020617_60%)] dark:text-slate-100">
      <div className="flex h-[min(680px,94vh)] w-full max-w-xl flex-col overflow-hidden rounded-3xl border border-slate-200 bg-white shadow-2xl dark:border-slate-800 dark:bg-slate-900">
        <div className="flex items-center justify-between border-b border-slate-100 px-6 py-4 dark:border-slate-800">
          <div className="flex items-center gap-2">
            <IconSparkles className="h-5 w-5 text-sky-600 dark:text-sky-400" />
            <span className="text-sm font-semibold">TrendWave setup</span>
          </div>
          <Dots total={TOTAL_STEPS} active={step} />
        </div>

        <div className="flex-1 overflow-y-auto px-6 py-6">
          {step === 0 && <Welcome />}
          {step === 1 && <Terms agreed={agreed} onAgree={setAgreed} />}
          {step === 2 && (
            <LocalAi
              report={report}
              ollama={ollama}
              checking={checking}
              selectedModel={selectedModel}
              onSelect={setSelectedModel}
              onRecheck={refreshOllama}
            />
          )}
          {step === 3 && <HowItWorks />}
          {step === 4 && <WhatYouGet />}
        </div>

        <div className="flex items-center justify-between border-t border-slate-100 px-6 py-4 dark:border-slate-800">
          <button
            onClick={back}
            disabled={step === 0}
            className="flex items-center gap-1 rounded-xl px-3 py-2 text-sm font-medium text-slate-500 hover:bg-slate-100 disabled:invisible dark:text-slate-400 dark:hover:bg-slate-800"
          >
            <IconChevronLeft className="h-4 w-4" /> Back
          </button>
          {isLast ? (
            <button
              onClick={finish}
              disabled={finishing}
              className="flex items-center gap-2 rounded-xl bg-slate-900 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-slate-800 disabled:opacity-50 dark:bg-sky-600 dark:hover:bg-sky-500"
            >
              {finishing && <IconSpinner className="h-4 w-4" />}
              {finishing ? "Finishing…" : "Get started"}
            </button>
          ) : (
            <button
              onClick={next}
              disabled={blockContinue}
              className="rounded-xl bg-slate-900 px-5 py-2.5 text-sm font-semibold text-white transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-sky-600 dark:hover:bg-sky-500"
            >
              {step === 1 ? "Agree & continue" : "Continue"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function Dots({ total, active }: { total: number; active: number }) {
  return (
    <div className="flex items-center gap-1.5" aria-label={`Step ${active + 1} of ${total}`}>
      {Array.from({ length: total }).map((_, i) => (
        <span
          key={i}
          className={`h-1.5 rounded-full transition-all ${
            i === active
              ? "w-5 bg-sky-500"
              : i < active
                ? "w-1.5 bg-sky-400/70"
                : "w-1.5 bg-slate-300 dark:bg-slate-700"
          }`}
        />
      ))}
    </div>
  );
}

// ---- Step 1: welcome --------------------------------------------------------

function Welcome() {
  const items = [
    {
      icon: <IconSearch className="h-5 w-5" />,
      title: "Ask in plain English",
      body: "Describe an industry or trend — no tickers or jargon required.",
    },
    {
      icon: <IconSparkles className="h-5 w-5" />,
      title: "Find the bottleneck",
      body: "TrendWave maps supply-chain chokepoints, then the public companies best placed to win them.",
    },
    {
      icon: <IconLock className="h-5 w-5" />,
      title: "Local & private",
      body: "Reasoning runs on your machine via Ollama. No accounts, no API keys, nothing sent to the cloud.",
    },
  ];
  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <h1 className="text-2xl font-bold tracking-tight">Welcome to TrendWave</h1>
        <p className="text-sm leading-6 text-slate-500 dark:text-slate-400">
          Let’s get you set up — it takes about a minute.
        </p>
      </div>
      <ul className="space-y-3">
        {items.map((it) => (
          <li
            key={it.title}
            className="flex gap-3 rounded-2xl border border-slate-200 bg-white/60 p-4 dark:border-slate-800 dark:bg-slate-800/40"
          >
            <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-sky-100 text-sky-600 dark:bg-sky-500/15 dark:text-sky-300">
              {it.icon}
            </span>
            <div>
              <p className="text-sm font-semibold">{it.title}</p>
              <p className="mt-0.5 text-xs leading-5 text-slate-500 dark:text-slate-400">{it.body}</p>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ---- Step 2: terms & conditions --------------------------------------------

function Terms({ agreed, onAgree }: { agreed: boolean; onAgree: (v: boolean) => void }) {
  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <h1 className="text-xl font-bold tracking-tight">Terms &amp; Conditions</h1>
        <p className="text-sm text-slate-500 dark:text-slate-400">
          Please read and accept these terms to continue.
        </p>
      </div>
      <div className="h-64 space-y-3 overflow-y-auto rounded-2xl border border-slate-200 bg-slate-50 p-4 text-[13px] leading-6 text-slate-600 dark:border-slate-800 dark:bg-slate-950/40 dark:text-slate-300">
        <TermsClause n="1" title="Acceptance">
          By using TrendWave you agree to these terms. If you do not agree, do not use the app.
        </TermsClause>
        <TermsClause n="2" title="Not financial advice">
          TrendWave is a research and educational tool. Its output is heuristic, generated in part by
          a local language model, and may be inaccurate or incomplete. Nothing it produces is
          investment, legal, tax, or financial advice, and it is not a recommendation to buy, sell,
          or hold any security. Always do your own research and consult a licensed professional
          before investing.
        </TermsClause>
        <TermsClause n="3" title="No warranty">
          The app is provided “as is”, without warranties of any kind. We do not guarantee that
          results are accurate, reliable, timely, or available without interruption.
        </TermsClause>
        <TermsClause n="4" title="Third-party data & software">
          TrendWave queries free public data sources (such as Yahoo Finance and SEC EDGAR) and runs
          models through Ollama. Those services have their own terms, and their data may be delayed
          or wrong. You are responsible for complying with any third-party terms you connect to,
          including brokerage accounts you link for read-only context.
        </TermsClause>
        <TermsClause n="5" title="Local processing & your responsibility">
          Reasoning and your data stay on your device. You are responsible for the security of your
          machine and for any decisions you make using the app.
        </TermsClause>
        <TermsClause n="6" title="Limitation of liability">
          To the maximum extent permitted by law, the authors are not liable for any losses or
          damages, including trading or investment losses, arising from your use of TrendWave.
        </TermsClause>
        <TermsClause n="7" title="Changes">
          These terms may be updated in future versions. Continued use after an update means you
          accept the revised terms.
        </TermsClause>
      </div>
      <label className="flex cursor-pointer items-start gap-2.5 rounded-2xl border border-slate-200 p-3 text-sm text-slate-700 dark:border-slate-800 dark:text-slate-200">
        <input
          type="checkbox"
          checked={agreed}
          onChange={(e) => onAgree(e.target.checked)}
          className="mt-0.5 h-4 w-4 rounded border-slate-300 dark:border-slate-600 dark:bg-slate-800"
        />
        <span>
          I have read and agree to the Terms &amp; Conditions, and I understand TrendWave is a
          research tool — <span className="font-semibold">not financial advice</span>.
        </span>
      </label>
    </div>
  );
}

function TermsClause({ n, title, children }: { n: string; title: string; children: ReactNode }) {
  return (
    <p>
      <span className="font-semibold text-slate-700 dark:text-slate-200">
        {n}. {title}.
      </span>{" "}
      {children}
    </p>
  );
}

// ---- Step 3: local AI / Ollama ---------------------------------------------

function LocalAi({
  report,
  ollama,
  checking,
  selectedModel,
  onSelect,
  onRecheck,
}: {
  report: SystemReport | null;
  ollama: OllamaStatus | null;
  checking: boolean;
  selectedModel: string;
  onSelect: (id: string) => void;
  onRecheck: () => void;
}) {
  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <h1 className="text-xl font-bold tracking-tight">Set up local AI</h1>
        <p className="text-sm leading-6 text-slate-500 dark:text-slate-400">
          TrendWave thinks using a model that runs on your computer with{" "}
          <span className="font-medium text-slate-700 dark:text-slate-200">Ollama</span> — free, and
          fully private.
        </p>
      </div>

      <OllamaBanner ollama={ollama} checking={checking} onRecheck={onRecheck} />

      {report && <SpecsRow specs={report.specs} />}

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <p className="text-xs font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">
            Choose a model
          </p>
          {report && (
            <span className="text-[11px] text-slate-400 dark:text-slate-500">
              Recommended for your machine
            </span>
          )}
        </div>
        {report ? (
          <div className="space-y-2">
            {report.options.map((o) => (
              <ModelCard
                key={o.id}
                option={o}
                selected={selectedModel === o.id}
                onSelect={onSelect}
              />
            ))}
          </div>
        ) : (
          <p className="rounded-2xl border border-slate-200 bg-slate-50 p-3 text-xs leading-5 text-slate-500 dark:border-slate-800 dark:bg-slate-950/40 dark:text-slate-400">
            Couldn’t read your system specs. You can still continue — TrendWave will use its default
            model, and you can change it anytime in Settings.
          </p>
        )}
      </div>

      <PullHint ollama={ollama} selectedModel={selectedModel} />
    </div>
  );
}

function OllamaBanner({
  ollama,
  checking,
  onRecheck,
}: {
  ollama: OllamaStatus | null;
  checking: boolean;
  onRecheck: () => void;
}) {
  let tone = "neutral";
  let icon = <IconSpinner className="h-4 w-4" />;
  let title = "Checking for Ollama…";
  let body: ReactNode = "Looking for a local Ollama install.";

  if (ollama) {
    if (ollama.running) {
      tone = "ok";
      icon = <IconCheck className="h-4 w-4" />;
      title = "Ollama is running";
      body =
        ollama.models.length > 0
          ? `${ollama.models.length} model${ollama.models.length === 1 ? "" : "s"} already installed.`
          : "No models pulled yet — pick one below.";
    } else if (ollama.installed) {
      tone = "warn";
      icon = <IconAlert className="h-4 w-4" />;
      title = "Ollama is installed, but not running";
      body = "Open the Ollama app to start it, then re-check.";
    } else {
      tone = "info";
      icon = <IconDownload className="h-4 w-4" />;
      title = "Ollama isn’t installed yet";
      body = "Install Ollama to run models locally, then re-check.";
    }
  }

  const tones: Record<string, string> = {
    neutral:
      "border-slate-200 bg-slate-50 text-slate-600 dark:border-slate-800 dark:bg-slate-950/40 dark:text-slate-300",
    ok: "border-emerald-200 bg-emerald-50 text-emerald-800 dark:border-emerald-500/30 dark:bg-emerald-500/10 dark:text-emerald-200",
    warn: "border-amber-200 bg-amber-50 text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200",
    info: "border-sky-200 bg-sky-50 text-sky-800 dark:border-sky-500/30 dark:bg-sky-500/10 dark:text-sky-200",
  };

  return (
    <div className={`rounded-2xl border p-3 ${tones[tone]}`}>
      <div className="flex items-start gap-2.5">
        <span className="mt-0.5 shrink-0">{icon}</span>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold">{title}</p>
          <p className="mt-0.5 text-xs leading-5 opacity-90">{body}</p>
          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            {ollama && !ollama.installed && (
              <button
                onClick={() => openUrl(OLLAMA_DOWNLOAD_URL).catch(() => {})}
                className="inline-flex items-center gap-1.5 rounded-lg bg-slate-900 px-3 py-1.5 text-xs font-semibold text-white hover:bg-slate-800 dark:bg-sky-600 dark:hover:bg-sky-500"
              >
                <IconDownload className="h-3.5 w-3.5" /> Install Ollama
                <IconExternal className="h-3 w-3 opacity-80" />
              </button>
            )}
            <button
              onClick={onRecheck}
              disabled={checking}
              className="inline-flex items-center gap-1.5 rounded-lg border border-current/30 px-3 py-1.5 text-xs font-medium hover:bg-black/5 disabled:opacity-50 dark:hover:bg-white/5"
            >
              {checking ? (
                <IconSpinner className="h-3.5 w-3.5" />
              ) : (
                <IconRefresh className="h-3.5 w-3.5" />
              )}
              Re-check
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function SpecsRow({ specs }: { specs: SystemSpecs }) {
  return (
    <div className="grid grid-cols-3 gap-2">
      <SpecChip label="Memory" value={`${specs.total_ram_gb} GB`} />
      <SpecChip label="CPU" value={`${specs.cpu_cores} cores`} />
      <SpecChip label="Platform" value={`${prettyOs(specs.os)} · ${specs.arch}`} />
    </div>
  );
}

function SpecChip({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-2 rounded-xl border border-slate-200 bg-white/60 px-2.5 py-2 dark:border-slate-800 dark:bg-slate-800/40">
      <IconChip className="h-4 w-4 shrink-0 text-slate-400 dark:text-slate-500" />
      <div className="min-w-0">
        <p className="truncate text-[11px] uppercase tracking-wider text-slate-400 dark:text-slate-500">
          {label}
        </p>
        <p className="truncate text-xs font-semibold text-slate-700 dark:text-slate-200">{value}</p>
      </div>
    </div>
  );
}

function ModelCard({
  option,
  selected,
  onSelect,
}: {
  option: ModelOption;
  selected: boolean;
  onSelect: (id: string) => void;
}) {
  return (
    <button
      onClick={() => onSelect(option.id)}
      className={`w-full rounded-2xl border p-3 text-left transition ${
        selected
          ? "border-sky-400 ring-1 ring-sky-300 dark:border-sky-500 dark:ring-sky-500/40"
          : "border-slate-200 hover:border-slate-300 dark:border-slate-800 dark:hover:border-slate-700"
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold">{option.label}</span>
          <span className="rounded-md bg-slate-100 px-1.5 py-0.5 text-[11px] font-medium text-slate-500 dark:bg-slate-800 dark:text-slate-400">
            {option.params}
          </span>
          {option.recommended && (
            <span className="rounded-md bg-sky-100 px-1.5 py-0.5 text-[11px] font-semibold text-sky-700 dark:bg-sky-500/15 dark:text-sky-300">
              Recommended
            </span>
          )}
        </div>
        <span
          className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-full border ${
            selected
              ? "border-sky-500 bg-sky-500 text-white"
              : "border-slate-300 text-transparent dark:border-slate-600"
          }`}
        >
          <IconCheck className="h-3 w-3" />
        </span>
      </div>
      <p className="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{option.blurb}</p>
      <div className="mt-2 flex items-center gap-2 text-[11px] text-slate-400 dark:text-slate-500">
        <span>~{option.download_gb} GB download</span>
        <span aria-hidden>·</span>
        <span className={option.can_run ? "" : "font-medium text-amber-600 dark:text-amber-400"}>
          {option.can_run ? "Runs on your machine" : `Best with ~${option.min_ram_gb} GB RAM`}
        </span>
      </div>
    </button>
  );
}

function PullHint({ ollama, selectedModel }: { ollama: OllamaStatus | null; selectedModel: string }) {
  if (!ollama?.running || !selectedModel) return null;
  const family = selectedModel.split(":")[0];
  const pulled = ollama.models.some((m) => m === selectedModel || m.split(":")[0] === family);
  if (pulled) {
    return (
      <p className="flex items-center gap-1.5 text-[11px] text-emerald-600 dark:text-emerald-400">
        <IconCheck className="h-3.5 w-3.5" /> “{selectedModel}” is installed and ready.
      </p>
    );
  }
  return (
    <p className="text-[11px] leading-5 text-slate-400 dark:text-slate-500">
      TrendWave downloads <code className="rounded bg-slate-100 px-1 py-0.5 dark:bg-slate-800">{selectedModel}</code>{" "}
      on your first search, or run{" "}
      <code className="rounded bg-slate-100 px-1 py-0.5 dark:bg-slate-800">ollama pull {selectedModel}</code>{" "}
      now to grab it ahead of time.
    </p>
  );
}

function prettyOs(os: string): string {
  switch (os) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return os.charAt(0).toUpperCase() + os.slice(1);
  }
}

// ---- Step 4: how it works ---------------------------------------------------

function HowItWorks() {
  const examples = [
    "Where are the bottlenecks in the AI data-center buildout?",
    "What’s constraining solid-state battery production?",
    "Supply chokepoints in domestic semiconductor packaging?",
  ];
  const steps = [
    {
      title: "Ask a question",
      body: "Type a plain-English question about an industry, trend, or supply chain.",
    },
    {
      title: "TrendWave researches",
      body: "It identifies the chokepoints, finds the public companies positioned around them, and pulls real growth data (SEC EDGAR), prices, and news — streaming progress as it goes.",
    },
    {
      title: "Get a ranked shortlist",
      body: "Picks are ranked by data-derived growth and competitive positioning — not hype.",
    },
  ];
  return (
    <div className="space-y-5">
      <div className="space-y-1">
        <h1 className="text-xl font-bold tracking-tight">How it works</h1>
        <p className="text-sm text-slate-500 dark:text-slate-400">Three steps, one window.</p>
      </div>
      <ol className="space-y-3">
        {steps.map((s, i) => (
          <li
            key={s.title}
            className="flex gap-3 rounded-2xl border border-slate-200 bg-white/60 p-4 dark:border-slate-800 dark:bg-slate-800/40"
          >
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-slate-900 text-xs font-bold text-white dark:bg-sky-600">
              {i + 1}
            </span>
            <div>
              <p className="text-sm font-semibold">{s.title}</p>
              <p className="mt-0.5 text-xs leading-5 text-slate-500 dark:text-slate-400">{s.body}</p>
            </div>
          </li>
        ))}
      </ol>
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">
          Try asking
        </p>
        <div className="flex flex-wrap gap-2">
          {examples.map((ex) => (
            <span
              key={ex}
              className="rounded-full border border-slate-200 bg-white/70 px-3 py-1.5 text-xs text-slate-600 dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
            >
              {ex}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}

// ---- Step 5: what you get ---------------------------------------------------

function WhatYouGet() {
  const items = [
    {
      title: "Real growth, not vibes",
      body: "Each pick shows audited revenue & earnings growth from SEC EDGAR, with the sources so you can verify the thesis.",
    },
    {
      title: "Positioning & sentiment",
      body: "See how dominant a company is around a bottleneck, plus recent news and sentiment at a glance.",
    },
    {
      title: "Save & re-run",
      body: "Keep a question as a watchlist and re-run it whenever you want a fresh read.",
    },
    {
      title: "Optional portfolio context",
      body: "Connect a brokerage read-only to flag picks you already own. TrendWave can never place trades.",
    },
  ];
  return (
    <div className="space-y-5">
      <div className="space-y-1">
        <h1 className="text-xl font-bold tracking-tight">What you get</h1>
        <p className="text-sm text-slate-500 dark:text-slate-400">
          A sourced shortlist you can actually dig into.
        </p>
      </div>
      <ul className="space-y-3">
        {items.map((it) => (
          <li key={it.title} className="flex gap-3">
            <span className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-emerald-100 text-emerald-600 dark:bg-emerald-500/15 dark:text-emerald-300">
              <IconCheck className="h-3.5 w-3.5" />
            </span>
            <div>
              <p className="text-sm font-semibold">{it.title}</p>
              <p className="mt-0.5 text-xs leading-5 text-slate-500 dark:text-slate-400">{it.body}</p>
            </div>
          </li>
        ))}
      </ul>
      <p className="rounded-2xl border border-slate-200 bg-slate-50 p-3 text-[11px] leading-5 text-slate-500 dark:border-slate-800 dark:bg-slate-950/40 dark:text-slate-400">
        Reminder: TrendWave is a research tool, not financial advice. Always verify against primary
        sources before making any investment decision.
      </p>
    </div>
  );
}
