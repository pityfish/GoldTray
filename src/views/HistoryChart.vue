<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useOsTheme } from "naive-ui";
import {
  NAlert,
  NButton,
  NCard,
  NSpin,
  NTag,
} from "naive-ui";
import * as echarts from "echarts/core";
import { LineChart } from "echarts/charts";
import {
  GridComponent,
  MarkLineComponent,
  TooltipComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import * as api from "../api/history";
import { HISTORY_PERIODS, type HistoryPeriodValue, type HistoryPayload } from "../types/history";

echarts.use([LineChart, GridComponent, TooltipComponent, MarkLineComponent, CanvasRenderer]);

const props = defineProps<{
  /** 企稳观察价水平线（元/克） */
  observationLines?: number[];
}>();

const osTheme = useOsTheme();
const isDark = computed(() => osTheme.value === "dark");

const period = ref<HistoryPeriodValue>("14d");
const loading = ref(false);
const error = ref<string | null>(null);
const payload = ref<HistoryPayload | null>(null);

const chartEl = ref<HTMLDivElement | null>(null);
let chart: echarts.ECharts | null = null;

/** 区间统计 */
const stats = computed(() => payload.value?.stats ?? null);

/** 缓存提示（来自本地缓存且已过期） */
const staleTag = computed(() =>
  payload.value?.stale ? "缓存数据（网络异常）" : payload.value?.fromCache ? "缓存数据" : null,
);

async function load(p: HistoryPeriodValue) {
  loading.value = true;
  error.value = null;
  try {
    payload.value = await api.getHistory(p);
    render();
  } catch (e) {
    error.value = String(e);
    payload.value = null;
    render();
  } finally {
    loading.value = false;
  }
}

function render() {
  if (!chartEl.value) return;
  const dark = isDark.value;
  const axisColor = dark ? "#8887" : "#999";
  const splitColor = dark ? "#ffffff14" : "#0000000d";
  const labelColor = dark ? "#ccc" : "#666";

  if (!chart) {
    chart = echarts.init(chartEl.value);
  }

  if (!payload.value || payload.value.points.length === 0) {
    chart.clear();
    return;
  }

  const pts = payload.value.points;
  const xData = pts.map((p) => p.timestamp);
  const avgData = pts.map((p) => p.avgPrice);

  const markLines = (props.observationLines ?? []).map((v) => ({
    yAxis: v,
    lineStyle: { color: "#e0791f", type: "dashed" as const, width: 1 },
    label: { formatter: `观察价 ¥${v.toFixed(2)}`, position: "insideEndTop" as const },
    symbol: "none",
  }));

  chart.setOption(
    {
      animation: false,
      grid: { left: 8, right: 8, top: 14, bottom: 4, containLabel: true },
      tooltip: {
        trigger: "axis",
        formatter: (params: unknown) => {
          const list = params as { dataIndex: number }[];
          const i = list?.[0]?.dataIndex ?? 0;
          const p = pts[i];
          if (!p) return "";
          const fmt = (v: number | null) => (v === null ? "--" : `${v.toFixed(2)} 元`);
          return [
            `<b>${p.timestamp}</b>`,
            `最低：${fmt(p.minPrice)}`,
            `最高：${fmt(p.maxPrice)}`,
            `平均：${fmt(p.avgPrice)}`,
          ].join("<br/>");
        },
      },
      xAxis: {
        type: "category",
        data: xData,
        boundaryGap: false,
        axisLine: { lineStyle: { color: axisColor } },
        axisLabel: { color: labelColor, fontSize: 10, hideOverlap: true },
        axisTick: { show: false },
      },
      yAxis: {
        type: "value",
        scale: true,
        splitLine: { lineStyle: { color: splitColor } },
        axisLabel: { color: labelColor, fontSize: 10 },
      },
      series: [
        {
          name: "平均价",
          type: "line",
          data: avgData,
          showSymbol: false,
          lineStyle: { color: "#2e9e6b", width: 1.5 },
          itemStyle: { color: "#2e9e6b" },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: "rgba(46, 158, 107, 0.28)" },
              { offset: 1, color: "rgba(46, 158, 107, 0.02)" },
            ]),
          },
          markLine: markLines.length > 0 ? { silent: true, data: markLines } : undefined,
        },
      ],
    },
    true,
  );
}

function onResize() {
  chart?.resize();
}

watch(period, (p) => void load(p));
watch(isDark, () => render());
watch(
  () => props.observationLines,
  () => render(),
  { deep: true },
);

onMounted(() => {
  void load(period.value);
  window.addEventListener("resize", onResize);
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", onResize);
  chart?.dispose();
  chart = null;
});

function statText(v: string | null): string {
  return v === null || v === "" ? "--" : `¥${v}`;
}
</script>

<template>
  <n-card size="small" title="历史走势" style="margin-top: 12px">
    <template #header-extra>
      <n-tag v-if="staleTag" size="small" type="warning" :bordered="false">
        {{ staleTag }}
      </n-tag>
    </template>

    <div class="period-row">
      <n-button
        v-for="opt in HISTORY_PERIODS"
        :key="opt.value"
        size="tiny"
        :type="period === opt.value ? 'primary' : 'default'"
        :secondary="period !== opt.value"
        :disabled="loading"
        @click="period = opt.value"
      >
        {{ opt.label }}
      </n-button>
    </div>

    <n-spin :show="loading" size="small">
      <div v-show="payload && payload.points.length > 0" ref="chartEl" class="chart" />

      <n-alert
        v-if="!loading && error"
        type="error"
        :bordered="false"
        size="small"
        style="margin-top: 8px"
      >
        历史行情获取失败：{{ error }}
        <n-button size="tiny" quaternary type="primary" @click="load(period)">
          重试
        </n-button>
      </n-alert>

      <n-alert
        v-if="!loading && !error && payload && payload.points.length === 0"
        type="info"
        :bordered="false"
        size="small"
        style="margin-top: 8px"
      >
        历史行情正在积累，暂无足够数据。
      </n-alert>
    </n-spin>

    <div v-if="stats" class="stats-grid">
      <div class="stat">
        <span class="stat-label">最高</span>
        <span class="stat-value price-up">{{ statText(stats.maxPrice) }}</span>
      </div>
      <div class="stat">
        <span class="stat-label">最低</span>
        <span class="stat-value price-down">{{ statText(stats.minPrice) }}</span>
      </div>
      <div class="stat">
        <span class="stat-label">平均</span>
        <span class="stat-value">{{ statText(stats.avgPrice) }}</span>
      </div>
      <div class="stat">
        <span class="stat-label">涨跌</span>
        <span
          class="stat-value"
          :class="Number(stats.changePercent) >= 0 ? 'price-up' : 'price-down'"
        >
          {{ stats.changePercent === null ? "--" : `${Number(stats.changePercent) >= 0 ? "+" : ""}${stats.changePercent}%` }}
        </span>
      </div>
    </div>
  </n-card>
</template>

<style scoped>
.period-row {
  display: flex;
  gap: 6px;
  margin-bottom: 8px;
  flex-wrap: wrap;
}
.chart {
  width: 100%;
  height: 200px;
}
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 4px;
  margin-top: 8px;
  padding: 0 4px;
}
.stat {
  display: flex;
  flex-direction: column;
  align-items: center;
}
.stat-label {
  font-size: 11px;
  color: #999;
}
.stat-value {
  font-size: 13px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
</style>
