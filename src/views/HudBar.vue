<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useQuoteStore } from "../stores/quote";
import { useSettingsStore } from "../stores/settings";
import iconUrl from "../assets/icon.png";

const quoteStore = useQuoteStore();
const settingsStore = useSettingsStore();

const rootEl = ref<HTMLDivElement | null>(null);
let resizeObserver: ResizeObserver | null = null;

/** 悬浮条背景透明度（0.3~1，默认 0.95，设置页可调） */
const hudOpacity = ref(0.95);

async function reloadOpacity() {
  try {
    const pairs: Record<string, string | null> = await invoke("get_settings", {
      keys: ["hud_opacity"],
    });
    const raw = pairs["hud_opacity"];
    const v = raw === null || raw === undefined ? NaN : Number(raw);
    hudOpacity.value = Number.isFinite(v) ? Math.min(1, Math.max(0.3, v)) : 0.95;
  } catch {
    hudOpacity.value = 0.95;
  }
}

/** 设计基准：窗口高 52px 对应基准字号 14px，其余尺寸用 em 等比缩放 */
const BASE_HEIGHT = 52;
const BASE_FONT = 14;

/** 按窗口高度等比缩放内容（拉大窗口文字/徽章一起放大） */
function applyScale() {
  const el = rootEl.value;
  if (!el) return;
  const h = el.clientHeight;
  if (h <= 0) return;
  const font = Math.min(64, Math.max(9, BASE_FONT * (h / BASE_HEIGHT)));
  el.style.fontSize = `${font.toFixed(1)}px`;
}

onMounted(() => {
  // 悬浮条窗口：背景需透明（窗口本身 transparent）
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  void quoteStore.init();
  void settingsStore.init();
  void reloadOpacity();
  applyScale();
  resizeObserver = new ResizeObserver(() => applyScale());
  if (rootEl.value) {
    resizeObserver.observe(rootEl.value);
  }
  // 设置页调整透明度时实时生效
  void listen("settings-changed", (event) => {
    if (event.payload === "hud_opacity") {
      void reloadOpacity();
    }
  });
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  resizeObserver = null;
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
    ref="rootEl"
    class="hud"
    data-tauri-drag-region
    :class="{ degraded: isStale || hasError }"
    :style="{ '--hud-opacity': hudOpacity }"
  >
    <img class="badge" :src="iconUrl" alt="GoldTray" />
    <div class="price-wrap">
      <span class="cny">¥</span>
      <span class="price">{{ price }}</span>
      <span
        v-if="changeText && !isStale && !hasError"
        class="change"
        :class="changeText.startsWith('↑') ? 'up' : 'down'"
      >
        {{ changeText }}
      </span>
      <span v-if="isStale || hasError" class="warn">⚠</span>
    </div>
  </div>
</template>

<style scoped>
.hud {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.7em;
  width: 100vw;
  height: 100vh;
  box-sizing: border-box;
  padding: 0 0.86em;
  border-radius: 0.93em;
  background: linear-gradient(
    180deg,
    rgba(38, 41, 50, var(--hud-opacity, 0.95)),
    rgba(24, 26, 32, var(--hud-opacity, 0.95))
  );
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
.badge {
  pointer-events: none;
  width: 1.86em;
  height: 1.86em;
  border-radius: 50%;
  object-fit: cover;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
  flex-shrink: 0;
}
.price-wrap {
  pointer-events: none;
  display: flex;
  align-items: baseline;
  gap: 0.29em;
}
.cny {
  color: rgba(255, 255, 255, 0.55);
  font-size: 0.93em;
  font-weight: 600;
}
.price {
  color: #fff;
  font-size: 1.36em;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.02em;
}
.change {
  font-size: 0.9em;
  font-weight: 700;
}
.change.up {
  color: #ff6b6b;
}
.change.down {
  color: #4ade80;
}
.warn {
  color: #f5a623;
  font-size: 1em;
}
</style>
