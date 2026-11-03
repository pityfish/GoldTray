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
    :class="{ degraded: isStale || hasError }"
  >
    <div class="hud-badge">Au</div>
    <div class="hud-price-wrap">
      <span class="hud-cny">¥</span>
      <span class="hud-price">{{ price }}</span>
      <span
        v-if="changeText && !isStale && !hasError"
        class="hud-change"
        :class="changeText.startsWith('↑') ? 'up' : 'down'"
      >
        {{ changeText }}
      </span>
    </div>
    <span v-if="isStale || hasError" class="hud-warn">⚠</span>
  </div>
</template>

<style scoped>
.hud {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100vw;
  height: 100vh;
  box-sizing: border-box;
  padding: 0 12px 0 10px;
  border-radius: 13px;
  background: linear-gradient(180deg, rgba(38, 41, 50, 0.95), rgba(24, 26, 32, 0.95));
  border: 1px solid rgba(255, 255, 255, 0.1);
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
  cursor: move;
  user-select: none;
  white-space: nowrap;
}
.hud.degraded {
  border-color: rgba(245, 166, 35, 0.6);
}
/* 装饰元素禁用指针事件：点击全部落到根元素，整条可拖动 */
.hud-badge {
  pointer-events: none;
  width: 26px;
  height: 26px;
  border-radius: 8px;
  background: linear-gradient(135deg, #f8d477, #e8a80c);
  color: #5c3a00;
  font-weight: 800;
  font-size: 13px;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.45);
  flex-shrink: 0;
}
.hud-price-wrap {
  pointer-events: none;
  display: flex;
  align-items: baseline;
  gap: 4px;
}
.hud-cny {
  color: rgba(255, 255, 255, 0.55);
  font-size: 13px;
  font-weight: 600;
}
.hud-price {
  color: #fff;
  font-size: 19px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.3px;
}
.hud-change {
  font-size: 12.5px;
  font-weight: 700;
}
.hud-change.up {
  color: #ff6b6b;
}
.hud-change.down {
  color: #4ade80;
}
.hud-warn {
  pointer-events: none;
  margin-left: auto;
  color: #f5a623;
  font-size: 14px;
}
</style>
