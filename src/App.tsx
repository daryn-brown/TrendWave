import { useEffect, useState } from "react";
import "./App.css";
import * as api from "./api";
import {
  BottleneckList,
  CandidateCard,
  ErrorBanner,
  IconRefresh,
  IconSearch,
  IconSparkles,
  IconSpinner,
  ProgressLog,
  PortfolioPanel,
  SettingsModal,
  UpdateBanner,
  WatchlistSidebar,
  type UpdatePhase,
} from "./components";
import type {
  AppErrorShape,
  Bottleneck,
  Candidate,
  ProgressEvent,
  ResearchResult,
  RobinhoodStatus,
  Settings,
  Watchlist,
} from "./types";
import { checkForUpdate, downloadAndInstall, relaunch, Update } from "./updater";
import { useTheme } from "./theme";

const EXAMPLES = [
  "Where are the bottlenecks in the AI data-center buildout?",
  "What's constraining solid-state battery production?",
  "Supply chokepoints in domestic semiconductor packaging?",
];

export default function App() {
  const [prompt, setPrompt] = useState("");
  const [running, setRunning] = useState(false);
  const [messages, setMessages] = useState<string[]>([]);
  const [bottlenecks, setBottlenecks] = useState<Bottleneck[]>([]);
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [result, setResult] = useState<ResearchResult | null>(null);
  const [error, setError] = useState<AppErrorShape | null>(null);

  const [watchlists, setWatchlists] = useState<Watchlist[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [saveName, setSaveName] = useState<string | null>(null);

  const [robinhood, setRobinhood] = useState<RobinhoodStatus | null>(null);
  const [rhBusy, setRhBusy] = useState(false);

  const [update, setUpdate] = useState<Update | null>(null);
  const [updatePhase, setUpdatePhase] = useState<UpdatePhase | null>(null);
  const [updateProgress, setUpdateProgress] = useState(0);
  const [updateToast, setUpdateToast] = useState<string | null>(null);

  const { theme, toggle: toggleTheme } = useTheme();

  useEffect(() => {
    api.getSettings().then(setSettings).catch(() => {});
    refreshWatchlists();
    // Reflect any previously-authorized Robinhood session (read-only).
    api.robinhoodStatus().then(setRobinhood).catch(() => {});
    // Silent check on launch; failures (e.g. running in dev) are ignored.
    checkForUpdate()
      .then((u) => {
        if (u) {
          setUpdate(u);
          setUpdatePhase("available");
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!updateToast) return;
    const t = setTimeout(() => setUpdateToast(null), 4000);
    return () => clearTimeout(t);
  }, [updateToast]);

  const refreshWatchlists = () =>
    api.listWatchlists().then(setWatchlists).catch(() => {});

  const resetView = () => {
    setMessages([]);
    setBottlenecks([]);
    setCandidates([]);
    setResult(null);
    setError(null);
  };

  const handleEvent = (event: ProgressEvent) => {
    switch (event.type) {
      case "stage":
        setMessages((m) => [...m, event.message]);
        break;
      case "bottlenecks":
        setBottlenecks(event.items);
        break;
      case "candidate":
        setCandidates((c) => [...c, event.candidate]);
        break;
      case "done":
        setResult(event.result);
        setBottlenecks(event.result.bottlenecks);
        setCandidates(event.result.candidates);
        break;
      case "failed":
        setError({ kind: event.kind, message: event.message });
        break;
    }
  };

  async function execute(run: () => Promise<ResearchResult>) {
    resetView();
    setRunning(true);
    try {
      const res = await run();
      setResult(res);
      setBottlenecks(res.bottlenecks);
      setCandidates(res.candidates);
      refreshWatchlists();
    } catch (err) {
      setError(err as AppErrorShape);
    } finally {
      setRunning(false);
    }
  }

  const handleSearch = (text: string) => {
    const q = text.trim();
    if (!q || running) return;
    setPrompt(q);
    setActiveId(null);
    execute(() => api.runResearch(q, handleEvent));
  };

  const handleSelectWatchlist = (w: Watchlist) => {
    setActiveId(w.id);
    setPrompt(w.prompt);
    setError(null);
    setMessages([]);
    if (w.last_result) {
      setResult(w.last_result);
      setBottlenecks(w.last_result.bottlenecks);
      setCandidates(w.last_result.candidates);
    } else {
      setResult(null);
      setBottlenecks([]);
      setCandidates([]);
    }
  };

  const handleRerun = () => {
    if (activeId == null || running) return;
    const id = activeId;
    execute(() => api.runWatchlist(id, handleEvent));
  };

  const handleNew = () => {
    setActiveId(null);
    setPrompt("");
    resetView();
  };

  const handleDelete = async (id: number) => {
    await api.deleteWatchlist(id).catch(() => {});
    if (activeId === id) handleNew();
    refreshWatchlists();
  };

  const confirmSave = async () => {
    const name = (saveName || "").trim();
    if (!name || !prompt.trim()) {
      setSaveName(null);
      return;
    }
    const created = await api.createWatchlist(name, prompt.trim()).catch(() => null);
    setSaveName(null);
    if (created) {
      setActiveId(created.id);
      refreshWatchlists();
    }
  };

  const handleSaveSettings = async (s: Settings) => {
    await api.saveSettings(s).catch(() => {});
    setSettings(s);
    setShowSettings(false);
  };

  const handleConnectRobinhood = async () => {
    setRhBusy(true);
    setError(null);
    try {
      setRobinhood(await api.robinhoodConnect());
    } catch (err) {
      setError(err as AppErrorShape);
    } finally {
      setRhBusy(false);
    }
  };

  const handleDisconnectRobinhood = async () => {
    setRhBusy(true);
    try {
      await api.robinhoodDisconnect();
      setRobinhood({ connected: false, portfolio: null });
    } catch (err) {
      setError(err as AppErrorShape);
    } finally {
      setRhBusy(false);
    }
  };

  const handleRefreshPortfolio = async () => {
    setRhBusy(true);
    setError(null);
    try {
      const portfolio = await api.robinhoodPortfolio();
      setRobinhood({ connected: true, portfolio });
    } catch (err) {
      setError(err as AppErrorShape);
    } finally {
      setRhBusy(false);
    }
  };

  const handleCheckUpdates = async () => {
    if (updatePhase === "downloading") return;
    setUpdateToast("Checking for updates…");
    try {
      const u = await checkForUpdate();
      if (u) {
        setUpdate(u);
        setUpdatePhase("available");
        setUpdateToast(null);
      } else {
        setUpdatePhase(null);
        setUpdateToast("You're on the latest version.");
      }
    } catch (err) {
      setUpdateToast(`Update check failed: ${String(err)}`);
    }
  };

  const handleInstallUpdate = async () => {
    if (!update) return;
    setUpdateProgress(0);
    setUpdatePhase("downloading");
    try {
      await downloadAndInstall(update, setUpdateProgress);
      setUpdatePhase("ready");
    } catch (err) {
      setUpdateToast(`Update failed: ${String(err)}`);
      setUpdatePhase("available");
    }
  };

  const handleRestart = () => {
    relaunch().catch(() => {});
  };

  const hasResults = bottlenecks.length > 0 || candidates.length > 0;

  return (
    <div className="flex h-full bg-[radial-gradient(circle_at_top,_#eff6ff,_#f8fafc_60%)] text-slate-900 dark:bg-[radial-gradient(circle_at_top,_#0b1220,_#020617_60%)] dark:text-slate-100">
      <WatchlistSidebar
        watchlists={watchlists}
        activeId={activeId}
        onSelect={handleSelectWatchlist}
        onDelete={handleDelete}
        onNew={handleNew}
        onOpenSettings={() => setShowSettings(true)}
        onCheckUpdates={handleCheckUpdates}
        theme={theme}
        onToggleTheme={toggleTheme}
      />

      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-8">
          {updatePhase && update && (
            <UpdateBanner
              version={update.version}
              phase={updatePhase}
              progress={updateProgress}
              onInstall={handleInstallUpdate}
              onRestart={handleRestart}
              onDismiss={() => setUpdatePhase(null)}
            />
          )}
          <header>
            <h1 className="text-2xl font-bold tracking-tight">Find the bottleneck. Find the stock.</h1>
            <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">
              Ask about an industry. TrendWave finds the supply chokepoints, then the public companies
              best positioned to solve or monopolize them — ranked by real revenue &amp; earnings growth
              (SEC EDGAR) and competitive positioning, reasoned locally via Ollama.
            </p>
          </header>

          <PromptBar
            value={prompt}
            running={running}
            onChange={setPrompt}
            onSubmit={() => handleSearch(prompt)}
          />

          {!hasResults && !running && (
            <div className="flex flex-wrap gap-2">
              {EXAMPLES.map((ex) => (
                <button
                  key={ex}
                  onClick={() => handleSearch(ex)}
                  className="rounded-full border border-slate-200 bg-white/70 px-3 py-1.5 text-xs text-slate-600 hover:border-sky-300 hover:text-sky-700 dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300 dark:hover:border-sky-500/50 dark:hover:text-sky-400"
                >
                  {ex}
                </button>
              ))}
            </div>
          )}

          {error && <ErrorBanner error={error} onDismiss={() => setError(null)} />}

          {robinhood?.connected && robinhood.portfolio && (
            <PortfolioPanel
              portfolio={robinhood.portfolio}
              busy={rhBusy}
              onRefresh={handleRefreshPortfolio}
            />
          )}

          {running && <ProgressLog messages={messages} running={running} />}

          {(hasResults || result) && (
            <ResultsHeader
              result={result}
              activeId={activeId}
              running={running}
              canSave={!!prompt.trim()}
              onRerun={handleRerun}
              onSave={() => setSaveName("")}
            />
          )}

          <BottleneckList items={bottlenecks} />

          {candidates.length > 0 && (
            <section className="space-y-3">
              <h3 className="text-sm font-semibold uppercase tracking-[0.2em] text-slate-500 dark:text-slate-400">
                Stock picks ({candidates.length})
              </h3>
              <div className="space-y-4">
                {candidates.map((c, i) => (
                  <CandidateCard key={`${c.ticker}-${i}`} candidate={c} />
                ))}
              </div>
            </section>
          )}

          {result && candidates.length === 0 && !running && (
            <p className="rounded-2xl border border-slate-200 bg-white/70 p-4 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-900/60 dark:text-slate-400">
              No stock picks came back this time — the model didn't return usable tickers. Try
              rephrasing the industry or re-running.
            </p>
          )}

          {result?.disclaimer && (
            <p className="pt-2 text-xs leading-5 text-slate-400 dark:text-slate-500">{result.disclaimer}</p>
          )}
        </div>
      </main>

      {showSettings && settings && (
        <SettingsModal
          settings={settings}
          onSave={handleSaveSettings}
          onClose={() => setShowSettings(false)}
          robinhood={robinhood}
          robinhoodBusy={rhBusy}
          onConnectRobinhood={handleConnectRobinhood}
          onDisconnectRobinhood={handleDisconnectRobinhood}
        />
      )}

      {saveName !== null && (
        <SaveDialog
          name={saveName}
          onChange={setSaveName}
          onConfirm={confirmSave}
          onCancel={() => setSaveName(null)}
        />
      )}

      {updateToast && (
        <div className="fixed bottom-4 left-1/2 z-50 -translate-x-1/2 rounded-full bg-slate-900 px-4 py-2 text-xs font-medium text-white shadow-lg dark:bg-slate-800 dark:ring-1 dark:ring-slate-700">
          {updateToast}
        </div>
      )}
    </div>
  );
}

function PromptBar({
  value,
  running,
  onChange,
  onSubmit,
}: {
  value: string;
  running: boolean;
  onChange: (v: string) => void;
  onSubmit: () => void;
}) {
  return (
    <div className="flex items-end gap-2 rounded-2xl border border-slate-200 bg-white p-2 shadow-sm focus-within:border-sky-300 dark:border-slate-800 dark:bg-slate-900 dark:focus-within:border-sky-500/60">
      <textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSubmit();
          }
        }}
        rows={2}
        placeholder="Where are the bottlenecks in…?"
        className="flex-1 resize-none bg-transparent px-3 py-2 text-sm text-slate-900 placeholder:text-slate-400 focus:outline-none dark:text-slate-100 dark:placeholder:text-slate-500"
      />
      <button
        onClick={onSubmit}
        disabled={running || !value.trim()}
        className="flex items-center gap-1.5 rounded-xl bg-slate-900 px-4 py-2.5 text-sm font-medium text-white transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-sky-600 dark:hover:bg-sky-500"
      >
        {running ? <IconSpinner /> : <IconSearch />}
        {running ? "Researching" : "Search"}
      </button>
    </div>
  );
}

function ResultsHeader({
  result,
  activeId,
  running,
  canSave,
  onRerun,
  onSave,
}: {
  result: ResearchResult | null;
  activeId: number | null;
  running: boolean;
  canSave: boolean;
  onRerun: () => void;
  onSave: () => void;
}) {
  return (
    <div className="rounded-2xl border border-slate-200 bg-white/80 p-5 dark:border-slate-800 dark:bg-slate-900/70">
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          <IconSparkles className="h-5 w-5 text-sky-600 dark:text-sky-400" />
          <h2 className="text-lg font-semibold capitalize">{result?.industry || "Research"}</h2>
        </div>
        <div className="flex shrink-0 gap-2">
          {activeId != null && (
            <button
              onClick={onRerun}
              disabled={running}
              className="flex items-center gap-1.5 rounded-lg border border-slate-200 px-3 py-1.5 text-xs font-medium text-slate-600 hover:bg-slate-50 disabled:opacity-40 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-800"
            >
              <IconRefresh className="h-3.5 w-3.5" /> Re-run
            </button>
          )}
          {activeId == null && canSave && (
            <button
              onClick={onSave}
              className="rounded-lg border border-slate-200 px-3 py-1.5 text-xs font-medium text-slate-600 hover:bg-slate-50 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-800"
            >
              Save watchlist
            </button>
          )}
        </div>
      </div>
      {result?.summary && <p className="mt-2 text-sm leading-6 text-slate-600 dark:text-slate-300">{result.summary}</p>}
    </div>
  );
}

function SaveDialog({
  name,
  onChange,
  onConfirm,
  onCancel,
}: {
  name: string;
  onChange: (v: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 dark:bg-black/60" onClick={onCancel}>
      <div className="w-full max-w-sm rounded-3xl bg-white p-6 shadow-2xl dark:bg-slate-900 dark:ring-1 dark:ring-slate-800" onClick={(e) => e.stopPropagation()}>
        <h2 className="text-lg font-bold">Save watchlist</h2>
        <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">Name this search so you can re-run it later.</p>
        <input
          autoFocus
          value={name}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && onConfirm()}
          placeholder="e.g. AI data-center bottlenecks"
          className="mt-4 w-full rounded-xl border border-slate-200 px-3 py-2 text-sm focus:border-sky-400 focus:outline-none dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100 dark:placeholder:text-slate-500"
        />
        <div className="mt-5 flex justify-end gap-2">
          <button onClick={onCancel} className="rounded-xl px-4 py-2 text-sm font-medium text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800">
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={!name.trim()}
            className="rounded-xl bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-800 disabled:opacity-40 dark:bg-sky-600 dark:hover:bg-sky-500"
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
