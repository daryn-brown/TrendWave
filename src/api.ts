import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Portfolio,
  ProgressEvent,
  ResearchResult,
  RobinhoodStatus,
  Settings,
  Watchlist,
} from "./types";

// Tauri maps camelCase JS keys to the snake_case Rust command parameters.

export function runResearch(
  prompt: string,
  onEvent: (event: ProgressEvent) => void,
): Promise<ResearchResult> {
  const channel = new Channel<ProgressEvent>();
  channel.onmessage = onEvent;
  return invoke<ResearchResult>("run_research", { prompt, onEvent: channel });
}

export function runWatchlist(
  id: number,
  onEvent: (event: ProgressEvent) => void,
): Promise<ResearchResult> {
  const channel = new Channel<ProgressEvent>();
  channel.onmessage = onEvent;
  return invoke<ResearchResult>("run_watchlist", { id, onEvent: channel });
}

export const getSettings = () => invoke<Settings>("get_settings");

export const saveSettings = (settings: Settings) =>
  invoke<void>("save_settings", { settings });

export const listWatchlists = () => invoke<Watchlist[]>("list_watchlists");

export const createWatchlist = (name: string, prompt: string) =>
  invoke<Watchlist>("create_watchlist", { name, prompt });

export const deleteWatchlist = (id: number) =>
  invoke<void>("delete_watchlist", { id });

// --- Robinhood (read-only MCP integration) ---------------------------------

export const robinhoodStatus = () => invoke<RobinhoodStatus>("robinhood_status");

// Opens the system browser for OAuth; resolves once authorized + first snapshot.
export const robinhoodConnect = () => invoke<RobinhoodStatus>("robinhood_connect");

export const robinhoodDisconnect = () => invoke<void>("robinhood_disconnect");

export const robinhoodPortfolio = () => invoke<Portfolio>("robinhood_portfolio");
