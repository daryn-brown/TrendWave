// Mirrors the serializable types emitted by the Rust backend (src-tauri/src/model.rs).

export interface Bottleneck {
  title: string;
  description: string;
  severity: number; // 1..5
}

export interface PriceData {
  price: number;
  currency: string;
  name?: string | null;
  change_pct: number;
  last_volume: number;
  avg_volume: number;
}

export interface GrowthData {
  revenue_growth_yoy?: number | null;
  revenue_cagr?: number | null;
  earnings_growth_yoy?: number | null;
  profitable?: boolean | null;
  forward_pe?: number | null;
  analyst_upside?: number | null;
  years?: number | null;
  annual_growth?: boolean;
  source: string;
}

export interface NewsItem {
  title: string;
  url: string;
  source: string;
  published?: string | null;
  sentiment?: number | null;
}

// Per-term weighted contributions behind a candidate's score (mirrors
// src-tauri/src/scoring.rs SignalBreakdown). Lets the UI explain the ranking.
export interface SignalBreakdown {
  severity: number;
  moat: number;
  growth: number;
  sentiment: number;
  momentum: number;
  inflection: number;
  technical: number;
  revisions: number;
  insider: number;
  filing: number;
  total: number;
}

export interface Candidate {
  ticker: string;
  company: string;
  verified_name?: string | null;
  identity_mismatch?: boolean;
  price?: PriceData | null;
  bottleneck: string;
  thesis: string;
  moat: number; // 1..5
  upside: number; // 1..5 — model's own guess; no longer drives ranking
  upside_rationale: string;
  growth?: GrowthData | null;
  growth_score: number; // 0..1 data-derived score used in ranking
  sentiment?: number | null;
  news: NewsItem[];
  score: number;
  breakdown?: SignalBreakdown | null; // per-term contributions behind `score`
  timing?: string | null; // cycle-timing label (Early/Building/Extended/Late)
  discovery?: string | null; // how the pick was surfaced (model/screener/both)
  owned?: boolean; // held in a connected brokerage account (read-only context)
}

export interface Position {
  ticker: string;
  name?: string | null;
  quantity: number;
  market_value?: number | null;
  average_buy_price?: number | null;
  unrealized_plpc?: number | null;
  currency: string;
  price?: number | null;
  change_pct?: number | null;
  spark?: number[];
}

export interface AccountSummary {
  portfolio_value?: number | null;
  buying_power?: number | null;
  cash?: number | null;
  currency: string;
}

export interface Portfolio {
  positions: Position[];
  account?: AccountSummary | null;
  as_of: string;
  tools_used: string[];
  debug?: string[];
}

export interface RobinhoodStatus {
  connected: boolean;
  locked: boolean;
  portfolio?: Portfolio | null;
}

export interface QuestradeStatus {
  connected: boolean;
  portfolio?: Portfolio | null;
}

export interface ResearchResult {
  industry: string;
  summary: string;
  bottlenecks: Bottleneck[];
  candidates: Candidate[];
  disclaimer: string;
}

// --- Buy routing (mirrors src-tauri/src/model.rs) --------------------------

export interface Listing {
  symbol: string;
  exchange?: string | null;
  currency?: string | null;
}

export interface ListingInfo {
  us_symbol: string;
  us_exchange?: string | null;
  canadian?: Listing | null;
}

export type ProgressEvent =
  | { type: "stage"; stage: string; message: string }
  | { type: "bottlenecks"; items: Bottleneck[] }
  | { type: "candidate"; candidate: Candidate }
  | { type: "done"; result: ResearchResult }
  | { type: "failed"; kind: string; message: string };

// Scoring profile (mirrors src-tauri/src/scoring.rs ScoringMode). `legacy`
// reproduces the original five-term formula; `early_detection` weights the
// forward/inflection signals.
export type ScoringMode = "legacy" | "early_detection";

// Market-data provider (mirrors src-tauri/src/providers.rs ProviderKind).
// `free` uses only SEC EDGAR + Yahoo and needs no key; paid modes use a
// bring-your-own-key source stored in the OS keychain.
export type ProviderKind = "free" | "fmp";

export interface Settings {
  ollama_endpoint: string;
  model: string;
  max_results: number;
  use_news: boolean;
  use_fundamentals: boolean;
  scoring_mode: ScoringMode;
  data_provider: ProviderKind;
  require_biometric_unlock: boolean;
}

// Status of the optional paid data provider (mirrors commands::DataProviderStatus).
// `has_key` reflects whether an API key is stored, read from a flag so opening
// Settings never triggers a keychain password prompt.
export interface DataProviderStatus {
  provider: ProviderKind;
  has_key: boolean;
}

export interface Watchlist {
  id: number;
  name: string;
  prompt: string;
  last_result?: ResearchResult | null;
  last_run_at?: string | null;
  created_at: string;
}

// Shape thrown by failed commands (src-tauri/src/error.rs).
export interface AppErrorShape {
  kind: string;
  message: string;
}

// --- First-run setup / onboarding (mirrors src-tauri/src/onboarding.rs) -----

export interface SystemSpecs {
  os: string; // "macos" | "windows" | "linux" | …
  arch: string; // e.g. "aarch64", "x86_64"
  total_ram_gb: number;
  cpu_cores: number;
}

export interface ModelOption {
  id: string; // Ollama model id, e.g. "llama3.1:8b"
  label: string;
  params: string; // e.g. "8B"
  min_ram_gb: number;
  download_gb: number;
  blurb: string;
  can_run: boolean; // detected RAM clears this model's comfortable minimum
  recommended: boolean; // best fit for this machine (exactly one)
}

export interface SystemReport {
  specs: SystemSpecs;
  options: ModelOption[];
  recommended_id: string;
}

export interface OllamaStatus {
  installed: boolean; // binary/app present on disk (even if not started)
  running: boolean; // local server answered
  models: string[]; // already-pulled model ids
}
