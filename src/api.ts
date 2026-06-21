import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  Listing,
  ListingInfo,
  OllamaStatus,
  Portfolio,
  ProgressEvent,
  QuestradeStatus,
  ResearchResult,
  RobinhoodStatus,
  Settings,
  SystemReport,
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

// --- First-run setup (onboarding) ------------------------------------------

// Whether the setup wizard has run. False only on a fresh install.
export const onboardingStatus = () => invoke<boolean>("onboarding_status");

// Persist the chosen model and mark setup complete.
export const completeOnboarding = (model: string) =>
  invoke<void>("complete_onboarding", { model });

// Machine specs + the model shortlist (best fit flagged) for the setup wizard.
export const systemReport = () => invoke<SystemReport>("system_report");

// Whether Ollama is installed / running and which models are pulled.
export const ollamaStatus = () => invoke<OllamaStatus>("ollama_status");

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

// --- Biometric unlock (Touch ID / Windows Hello) ---------------------------

// Whether this device exposes a biometric / device-auth unlock prompt.
export const biometricAvailable = () => invoke<boolean>("biometric_available");

// Prompts for Touch ID / Windows Hello; resolves true when the saved Robinhood
// session is unlocked, false when the user dismisses or fails the prompt.
export const biometricUnlock = () => invoke<boolean>("biometric_unlock");

export const robinhoodPortfolio = () => invoke<Portfolio>("robinhood_portfolio");

// --- Questrade (read-only REST integration) --------------------------------

export const questradeStatus = () => invoke<QuestradeStatus>("questrade_status");

// Exchanges the pasted manual authorization token; resolves once connected + first snapshot.
export const questradeConnect = (token: string) =>
  invoke<QuestradeStatus>("questrade_connect", { token });

export const questradeDisconnect = () => invoke<void>("questrade_disconnect");

export const questradePortfolio = () => invoke<Portfolio>("questrade_portfolio");

// --- Buy routing (read-only listing lookups; never places orders) ----------

// Resolves the US/base listing (with exchange) plus a same-security Canadian
// interlisting when one exists, so the Buy panel can route each broker correctly.
export const resolveListings = (symbol: string, company: string) =>
  invoke<ListingInfo>("resolve_listings", { symbol, company });

// Whether a ticker is an active, tradable Robinhood listing (public lookup).
export const robinhoodSymbolAvailable = (symbol: string) =>
  invoke<boolean>("robinhood_symbol_available", { symbol });

// Best tradable Questrade listing for a ticker (prefers a CAD listing). Rejects
// when Questrade isn't connected, so callers can fall back to the market heuristic.
export const questradeFindListing = (symbol: string) =>
  invoke<Listing | null>("questrade_find_listing", { symbol });
