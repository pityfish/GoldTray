<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useQuoteStaleness } from "../api/quoteStaleness";
import { useComponentListener } from "../api/componentEvents";
import {
  NAlert,
  NButton,
  NCard,
  NIcon,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { Refresh } from "@vicons/ionicons5";
import { useQuoteStore } from "../stores/quote";
import { useSettingsStore } from "../stores/settings";
import * as api from "../api/history";
import type { StabilizationAlert } from "../types/stabilization";
import HistoryChart from "./HistoryChart.vue";
import StabilizationCard from "./StabilizationCard.vue";

const quoteStore = useQuoteStore();
const settingsStore = useSettingsStore();
const message = useMessage();

/** 启用中的企稳规则（用于图表观察价水平线联动） */
const stabAlerts = ref<StabilizationAlert[]>([]);
const observationLines = computed(() =>
  stabAlerts.value
    .filter((a) => a.enabled)
    .map((a) => Number(a.observationPrice))
    .filter((v) => Number.isFinite(v) && v > 0),
);

async function reloadStabAlerts() {
  try {
    stabAlerts.value = await api.listStabilizationAlerts();
  } catch {
    // 企稳规则加载失败不影响行情展示
  }
}

const registerListener = useComponentListener("stabilization-changed", () => {
  void reloadStabAlerts();
});

onMounted(async () => {
  await Promise.all([quoteStore.init(), settingsStore.init(), reloadStabAlerts()]);
  await registerListener();
});

const price = computed(() => {
  if (!quoteStore.quote) return "--.--";
  const n = Number(quoteStore.quote.customerBuyPrice);
  return Number.isFinite(n) ? n.toFixed(2) : quoteStore.quote.customerBuyPrice;
});

const change = computed(() => {
  if (!quoteStore.quote?.rawChange || !settingsStore.settings.enableChangeDisplay)
    return null;
  const n = Number(quoteStore.quote.rawChange);
  if (!Number.isFinite(n) || n === 0) return null;
  return { positive: n > 0, text: Math.abs(n).toFixed(2) };
});

/** 行情是否已过期（超过过期阈值视为过期） */
const isStale = useQuoteStaleness(
  () => quoteStore.quote?.fetchedAt,
  () => settingsStore.settings.stalenessThresholdSecs,
);

const fetchedAtText = computed(() => {
  if (!quoteStore.quote) return "--";
  return new Date(quoteStore.quote.fetchedAt).toLocaleTimeString("zh-CN", {
    hour12: false,
  });
});

/** 设置了价差时，估算客户卖出价 = 买入参考价 − 价差 */
const estimatedSell = computed(() => {
  const spread = Number(settingsStore.settings.estimatedSpread ?? NaN);
  const buy = Number(quoteStore.quote?.customerBuyPrice ?? NaN);
  if (!Number.isFinite(spread) || spread < 0 || !Number.isFinite(buy)) {
    return null;
  }
  return (buy - spread).toFixed(2);
});

async function refresh() {
  try {
    await quoteStore.refresh();
  } catch {
    message.error("行情获取失败，请稍后重试");
  }
}
</script>

<template>
  <n-spin :show="quoteStore.refreshing && !quoteStore.quote">
    <div class="dashboard">
      <n-card size="small">
        <template #header>
          <span class="product-title">
            {{ quoteStore.quote?.bankName ?? "招商银行" }} ·
            {{ quoteStore.quote?.productName ?? "黄金活期" }}
          </span>
        </template>
        <template #header-extra>
          <n-tag size="small" type="warning" :bordered="false">
            第三方参考买入价
          </n-tag>
        </template>

        <div class="price-block">
          <span class="price">¥ {{ price }}</span>
          <span class="unit">/克</span>
          <n-tag size="tiny" type="info" :bordered="false" class="price-tag">
            买入价
          </n-tag>
        </div>
        <div v-if="change" class="change-block">
          <span
            :class="change.positive ? 'price-up' : 'price-down'"
            class="change-text"
          >
            {{ change.positive ? "↑" : "↓" }} {{ change.positive ? "+" : "-" }}{{ change.text }}
          </span>
          <span class="change-note">较昨日</span>
        </div>
        <div v-if="estimatedSell" class="sell-block">
          估算卖出价：¥ {{ estimatedSell }} /克
          <span class="sell-note">按价差 {{ settingsStore.settings.estimatedSpread }} 估算，请以银行 App 实际为准</span>
        </div>
        <div class="meta-block">
          最近获取：{{ fetchedAtText }}
          <n-tag
            v-if="isStale && quoteStore.quote"
            size="small"
            type="error"
            style="margin-left: 8px"
          >
            数据已过期
          </n-tag>
        </div>
      </n-card>

      <HistoryChart :observation-lines="observationLines" />
      <StabilizationCard />

      <n-alert
        v-if="quoteStore.error"
        type="error"
        title="行情获取失败"
        style="margin-top: 12px"
      >
        {{ quoteStore.error }}
      </n-alert>

      <n-alert
        v-if="isStale && quoteStore.quote"
        type="warning"
        style="margin-top: 12px"
      >
        行情长时间未更新，显示的可能是过期数据。请检查网络或稍后刷新。
      </n-alert>

      <div class="refresh-row">
        <n-button
          type="primary"
          secondary
          :loading="quoteStore.refreshing"
          @click="refresh"
        >
          <template #icon><n-icon><Refresh /></n-icon></template>
          立即刷新
        </n-button>
      </div>
    </div>
  </n-spin>
</template>

<style scoped>
.dashboard {
  padding-top: 8px;
}
.product-title {
  font-size: 15px;
}
.price-block {
  margin: 12px 0 4px;
}
.price {
  font-size: 40px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
.unit {
  margin-left: 6px;
  color: #888;
}
.price-tag {
  margin-left: 8px;
  vertical-align: super;
}
.change-block {
  display: flex;
  align-items: center;
  gap: 6px;
}
.change-text {
  font-size: 16px;
  font-weight: 600;
}
.change-note {
  font-size: 12px;
  color: #999;
}
.sell-block {
  margin-top: 8px;
  font-size: 14px;
  font-weight: 600;
}
.sell-note {
  margin-left: 6px;
  font-size: 12px;
  font-weight: 400;
  color: #999;
}
.meta-block {
  margin-top: 10px;
  color: #999;
  font-size: 12px;
}
.refresh-row {
  margin-top: 16px;
  display: flex;
  justify-content: center;
}
</style>
