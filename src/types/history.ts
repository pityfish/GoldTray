/** 历史行情周期选项 */
export const HISTORY_PERIODS = [
  { label: "2天", value: "48H" },
  { label: "14天", value: "14d" },
  { label: "1月", value: "1M" },
  { label: "3月", value: "3M" },
  { label: "6月", value: "6M" },
  { label: "12月", value: "12M" },
] as const;

export type HistoryPeriodValue = (typeof HISTORY_PERIODS)[number]["value"];

/** 标准化后的单点行情 */
export interface HistoricalPricePoint {
  timestamp: string;
  minPrice: number | null;
  maxPrice: number | null;
  avgPrice: number | null;
  quality: "VALID" | "CARRIED" | "MISSING" | "SUSPECT";
}

/** 区间统计 */
export interface HistoryStats {
  maxPrice: string | null;
  maxPriceTime: string | null;
  minPrice: string | null;
  minPriceTime: string | null;
  avgPrice: string | null;
  changePercent: string | null;
}

/** 走势图数据载荷 */
export interface HistoryPayload {
  period: string;
  points: HistoricalPricePoint[];
  stats: HistoryStats;
  fetchedAt: string;
  fromCache: boolean;
  stale: boolean;
}
