<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useQuoteStore } from "../stores/quote";
import { useSettingsStore } from "../stores/settings";

const quoteStore = useQuoteStore();
const settingsStore = useSettingsStore();

onMounted(() => {
  // 悬浮条窗口：背景需透明（窗口本身 transparent）
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  void quoteStore.init();
  void settingsStore.init();
});

const price = computed(() => {
  if (!quoteStore.quote) return "--.--";
  const n = Number(quoteStore.quote.customerBuyPrice);
  return Number.isFinite(n) ? n.toFixed(2) : quoteStore.quote.customerBuyPrice;
});

const changeText = computed(() => {
  if (!settingsStore.settings.enableChangeDisplay) return "";
  const n = Number(quoteStore.quote?.rawChange ?? NaN);
  if (!Number.isFinite(n) || n === 0) return "";
  return n > 0 ? `↑${n.toFixed(2)}` : `↓${Math.abs(n).toFixed(2)}`;
});

const isStale = computed(() => {
  if (!quoteStore.quote) return false;
  const fetchedAt = new Date(quoteStore.quote.fetchedAt).getTime();
  if (Number.isNaN(fetchedAt)) return true;
  return (
    (Date.now() - fetchedAt) / 1000 > settingsStore.settings.stalenessThresholdSecs
  );
});

const hasError = computed(() => !!quoteStore.error);
</script>

<template>
  <div
    class="hud"
    data-tauri-drag-region
    :class="{ stale: isStale || hasError }"
  >
    <span class="hud-au" data-tauri-drag-region>Au</span>
    <span class="hud-price" data-tauri-drag-region>¥{{ price }}</span>
    <span
      v-if="changeText && !isStale && !hasError"
      data-tauri-drag-region
      class="hud-change"
      :class="changeText.startsWith('↑') ? 'price-up' : 'price-down'"
    >
      {{ changeText }}
    </span>
    <span v-if="isStale || hasError" class="hud-warn">⚠</span>
  </div>
</template>

<style scoped>
.hud {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  width: 100vw;
  height: 100vh;
  box-sizing: border-box;
  padding: 0 10px;
  border-radius: 9px;
  background: rgba(24, 26, 32, 0.82);
  border: 1px solid rgba(255, 255, 255, 0.12);
  color: #fff;
  font-size: 17px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  cursor: move;
  user-select: none;
  white-space: nowrap;
}
.hud.stale {
  border-color: rgba(240, 100, 100, 0.8);
}
.hud-au {
  color: #f6c244;
}
.hud-price {
  letter-spacing: 0.3px;
}
.hud-change {
  font-size: 13px;
  font-weight: 600;
}
.hud-warn {
  font-size: 13px;
  color: #f56c6c;
}
</style>
