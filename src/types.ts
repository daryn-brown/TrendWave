// Mirrors the serializable types emitted by the Rust backend (src-tauri/src/model.rs).

export interface Bottleneck {
  title: string;
  description: string;
  severity: number; // 1..5
}

export interface PriceData {
  price: number;
  currency: string;
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

export interface Candidate {
  ticker: string;
  company: string;
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
}

export interface ResearchResult {
  industry: string;
  summary: string;
  bottlenecks: Bottleneck[];
  candidates: Candidate[];
  disclaimer: string;
}

export type ProgressEvent =
  | { type: "stage"; stage: string; message: string }
  | { type: "bottlenecks"; items: Bottleneck[] }
  | { type: "candidate"; candidate: Candidate }
  | { type: "done"; result: ResearchResult }
  | { type: "failed"; kind: string; message: string };

export interface Settings {
  ollama_endpoint: string;
  model: string;
  max_results: number;
  use_news: boolean;
  use_fundamentals: boolean;
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
