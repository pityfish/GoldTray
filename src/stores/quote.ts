import { defineStore } from "pinia";
import { ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { GoldQuote } from "../types/gold";
import * as api from "../api/goldtray";

export const useQuoteStore = defineStore("quote", () => {
  const quote = ref<GoldQuote | null>(null);
  const error = ref<string | null>(null);
  const refreshing = ref(false);
  let initialized = false;

  function apply(q: GoldQuote | null) {
    quote.value = q;
    error.value = null;
  }

  async function init() {
    if (initialized) return;
    initialized = true;

    // 订阅 Rust 后台调度器推送的行情更新
    await listen<GoldQuote>("quote-updated", (event) => {
      apply(event.payload);
      refreshing.value = false;
    });
    await listen<string>("quote-error", (event) => {
      error.value = event.payload;
      refreshing.value = false;
    });

    // 初始拉取一次缓存行情
    quote.value = await api.getLatestQuote();
  }

  async function refresh() {
    refreshing.value = true;
    try {
      apply(await api.refreshQuote());
    } catch (e) {
      error.value = String(e);
    } finally {
      refreshing.value = false;
    }
  }

  return { quote, error, refreshing, init, refresh };
});
