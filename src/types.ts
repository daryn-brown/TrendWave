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
  upside: number; // 1..5
  upside_rationale: string;
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
