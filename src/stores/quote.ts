import { defineStore } from "pinia";
import { ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { GoldQuote } from "../types/gold";
import * as api from "../api/goldtray";

export const useQuoteStore = defineStore("quote", () => {
  const quote = ref<GoldQuote | null>(null);
  const error = ref<string | null>(null);
  const refreshing = ref(false);
  let initialization: Promise<void> | undefined;

  function apply(q: GoldQuote | null) {
    quote.value = q;
    error.value = null;
  }

  async function load() {
    const unlisteners: UnlistenFn[] = [];
    try {
      // 订阅 Rust 后台调度器推送的行情更新
      unlisteners.push(await listen<GoldQuote>("quote-updated", (event) => {
        apply(event.payload);
        refreshing.value = false;
      }));
      unlisteners.push(await listen<string>("quote-error", (event) => {
        error.value = event.payload;
        refreshing.value = false;
      }));

      // 初始拉取一次缓存行情
      quote.value = await api.getLatestQuote();
    } catch (error) {
      await Promise.allSettled(unlisteners.map(async (unlisten) => unlisten()));
      throw error;
    }
  }

  function init() {
    return (initialization ??= load().catch((error: unknown) => {
      initialization = undefined;
      throw error;
    }));
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
