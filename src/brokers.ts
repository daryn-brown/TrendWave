// Buy routing for research picks.
//
// TrendWave stays read-only: the Buy action never places an order. It deep-links
// the chosen brokerage's page for the right ticker in the system browser. A broker
// is only offered when the pick is available there — a deterministic market/exchange
// gate, refined by live lookups for Robinhood (public) and a connected Questrade.
// For Canadian brokers we prefer a same-security interlisted Canadian listing so
// the purchase is in CAD with no FX conversion.

import * as api from "./api";
import type { Candidate, ListingInfo, PriceData } from "./types";

export type Market = "us" | "ca" | "other";

export type BrokerId =
  | "robinhood"
  | "questrade"
  | "fidelity"
  | "schwab"
  | "etrade"
  | "webull"
  | "wealthsimple";

interface BrokerDef {
  id: BrokerId;
  label: string;
  /** Listing markets this broker lets a retail account trade. */
  markets: Market[];
  /** Canadian brokers can route to a CAD interlisting to avoid FX. */
  canadian?: boolean;
}

export const BUY_BROKERS: BrokerDef[] = [
  { id: "robinhood", label: "Robinhood", markets: ["us"] },
  { id: "fidelity", label: "Fidelity", markets: ["us"] },
  { id: "schwab", label: "Charles Schwab", markets: ["us"] },
  { id: "etrade", label: "E*TRADE", markets: ["us"] },
  { id: "webull", label: "Webull", markets: ["us"] },
  { id: "questrade", label: "Questrade", markets: ["us", "ca"], canadian: true },
  { id: "wealthsimple", label: "Wealthsimple", markets: ["us", "ca"], canadian: true },
];

/** A resolved, ready-to-open Buy destination for one broker. */
export interface BuyOption {
  id: BrokerId;
  label: string;
  /** Exact symbol the user will trade on this broker. */
  symbol: string;
  currency: "USD" | "CAD";
  /** Only set for Canadian brokers: whether the route avoids an FX conversion. */
  fxNote?: "cad-native" | "usd-fx";
  /** Deep link to the ticker page (some brokers require sign-in first). */
  url: string;
}

const CA_SUFFIXES = [".TO", ".V", ".NE", ".CN"];

/** Root ticker without any exchange suffix (`SHOP.TO` -> `SHOP`). */
export function baseSymbol(ticker: string): string {
  return (ticker || "").trim().toUpperCase().split(".")[0];
}

/** Infer the listing's market from its Yahoo suffix, with currency as a tiebreak. */
export function inferMarket(ticker: string, price?: PriceData | null): Market {
  const t = (ticker || "").trim().toUpperCase();
  const currency = (price?.currency || "").toUpperCase();

  if (CA_SUFFIXES.some((s) => t.endsWith(s))) return "ca";

  // A non-Canadian exchange suffix (e.g. .L, .AX, .HK, .DE) means foreign.
  if (t.includes(".")) {
    if (currency === "USD") return "us";
    if (currency === "CAD") return "ca";
    return "other";
  }

  // No suffix: lean on currency, defaulting to US (the app's focus).
  if (currency === "CAD") return "ca";
  if (currency && currency !== "USD") return "other";
  return "us";
}

/** Map a Yahoo exchange (display or code) to Webull's URL prefix. */
function webullPrefix(exchange?: string | null): string | null {
  const e = (exchange || "").toLowerCase();
  if (!e) return null;
  if (e.includes("nasdaq") || e === "nms" || e === "ngm" || e === "ncm") return "nasdaq";
  if (e.includes("american") || e === "ase") return "amex";
  if (e.includes("arca") || e === "pcx") return "nyse";
  if (e.includes("nyse") || e === "nyq" || e.includes("new york")) return "nyse";
  return null;
}

interface UrlParts {
  usSymbol: string;
  usExchange?: string | null;
  caSymbol?: string;
  market: Market;
}

/** Build the most direct ticker-page URL we can for a broker. */
function brokerUrl(id: BrokerId, p: UrlParts): string {
  const us = p.usSymbol.toUpperCase();
  switch (id) {
    case "robinhood":
      return `https://robinhood.com/stocks/${us}`;
    case "fidelity":
      return `https://research2.fidelity.com/fidelity/research/quotes/summary?symbols=${us}`;
    case "schwab":
      return `https://www.schwab.com/stocks/${us}`;
    case "etrade":
      return `https://us.etrade.com/markets/stocks/${us}`;
    case "webull": {
      const prefix = webullPrefix(p.usExchange);
      return prefix
        ? `https://www.webull.com/quote/${prefix}-${us.toLowerCase()}`
        : `https://www.webull.com/center?search=${encodeURIComponent(us)}`;
    }
    case "questrade": {
      const sym = (p.caSymbol ?? us).toUpperCase();
      return `https://trading.questrade.com/quote/${encodeURIComponent(sym)}`;
    }
    case "wealthsimple": {
      const root = baseSymbol(p.caSymbol ?? us);
      const mkt = p.market === "ca" || p.caSymbol ? "CA" : "US";
      return `https://trade.wealthsimple.com/app/stocks/${root}:${mkt}`;
    }
  }
}

const cache = new Map<string, Promise<BuyOption[]>>();

/**
 * Resolve the brokers a pick can be bought through, each with the right symbol,
 * currency, and a direct ticker URL. Applies the market gate, then refines with
 * live lookups (Robinhood public availability; Questrade when connected) and a
 * Canadian interlisting preference. Fails open on lookup errors. Cached per pick.
 */
export function resolveBuyOptions(
  candidate: Candidate,
  questradeConnected: boolean,
): Promise<BuyOption[]> {
  const key = `${candidate.ticker}|${questradeConnected}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const pending = computeBuyOptions(candidate, questradeConnected).catch(() => {
    cache.delete(key); // let a transient failure be retried on reopen
    return [] as BuyOption[];
  });
  cache.set(key, pending);
  return pending;
}

async function computeBuyOptions(
  candidate: Candidate,
  questradeConnected: boolean,
): Promise<BuyOption[]> {
  const ticker = (candidate.ticker || "").trim();
  if (!ticker) return [];

  const market = inferMarket(ticker, candidate.price);
  const serving = BUY_BROKERS.filter((b) => b.markets.includes(market));
  if (serving.length === 0) return [];

  const us = baseSymbol(ticker);
  const company = candidate.verified_name || candidate.company || "";

  let info: ListingInfo | null = null;
  try {
    info = await api.resolveListings(us, company);
  } catch {
    info = null;
  }
  const usExchange = info?.us_exchange ?? null;

  // Prefer a same-security Canadian interlisting for CA brokers on US picks.
  let caListing = info?.canadian ?? null;
  const wantsCanadian = market === "us" && serving.some((b) => b.canadian);
  if (questradeConnected && wantsCanadian) {
    try {
      const q = await api.questradeFindListing(us);
      if (q && (q.currency || "").toUpperCase() === "CAD") caListing = q;
    } catch {
      // not connected / no match — keep the Yahoo interlisting (or none)
    }
  }

  // Live Robinhood availability (US market only; fail open on error).
  let robinhoodOk: boolean | null = null;
  if (serving.some((b) => b.id === "robinhood")) {
    try {
      robinhoodOk = await api.robinhoodSymbolAvailable(us);
    } catch {
      robinhoodOk = null;
    }
  }

  const options: BuyOption[] = [];
  for (const b of serving) {
    if (b.id === "robinhood" && robinhoodOk === false) continue;

    if (b.canadian) {
      options.push(canadianOption(b, { ticker, us, usExchange, market, caListing }));
    } else {
      options.push({
        id: b.id,
        label: b.label,
        symbol: us,
        currency: "USD",
        url: brokerUrl(b.id, { usSymbol: us, usExchange, market: "us" }),
      });
    }
  }
  return options;
}

function canadianOption(
  b: BrokerDef,
  ctx: {
    ticker: string;
    us: string;
    usExchange: string | null;
    market: Market;
    caListing: { symbol: string } | null;
  },
): BuyOption {
  // The pick is already a Canadian listing.
  if (ctx.market === "ca") {
    const sym = ctx.ticker.toUpperCase();
    return {
      id: b.id,
      label: b.label,
      symbol: sym,
      currency: "CAD",
      fxNote: "cad-native",
      url: brokerUrl(b.id, { usSymbol: ctx.us, caSymbol: sym, market: "ca" }),
    };
  }
  // A same-security Canadian interlisting exists — buy it in CAD, no FX.
  if (ctx.caListing?.symbol) {
    const sym = ctx.caListing.symbol.toUpperCase();
    return {
      id: b.id,
      label: b.label,
      symbol: sym,
      currency: "CAD",
      fxNote: "cad-native",
      url: brokerUrl(b.id, { usSymbol: ctx.us, caSymbol: sym, market: "ca" }),
    };
  }
  // No Canadian listing — the broker can still buy the US listing, with FX.
  return {
    id: b.id,
    label: b.label,
    symbol: ctx.us,
    currency: "USD",
    fxNote: "usd-fx",
    url: brokerUrl(b.id, { usSymbol: ctx.us, usExchange: ctx.usExchange, market: "us" }),
  };
}

/** Whether a pick can be bought anywhere — used to show/hide the Buy button fast. */
export function hasAnyBroker(ticker: string, price?: PriceData | null): boolean {
  const market = inferMarket(ticker, price);
  return BUY_BROKERS.some((b) => b.markets.includes(market));
}

const LAST_BROKER_KEY = "trendwave.buy.lastBroker";

export function rememberBroker(id: BrokerId): void {
  try {
    localStorage.setItem(LAST_BROKER_KEY, id);
  } catch {
    // ignore storage failures (private mode, etc.)
  }
}

export function lastBroker(): BrokerId | null {
  try {
    return (localStorage.getItem(LAST_BROKER_KEY) as BrokerId | null) ?? null;
  } catch {
    return null;
  }
}
