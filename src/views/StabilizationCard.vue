<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import {
  NButton,
  NCard,
  NDescriptions,
  NDescriptionsItem,
  NModal,
  NTag,
  useMessage,
} from "naive-ui";
import * as api from "../api/history";
import {
  STABILIZATION_STATE_META,
  type StabilizationAlert,
  type StabilizationAssessment,
} from "../types/stabilization";

const alerts = ref<StabilizationAlert[]>([]);
const showModal = ref(false);
const assessing = ref(false);
const assessment = ref<StabilizationAssessment | null>(null);
const assessedName = ref("");
const message = useMessage();

const enabledAlerts = computed(() => alerts.value.filter((a) => a.enabled));

async function reload() {
  try {
    alerts.value = await api.listStabilizationAlerts();
  } catch (e) {
    console?.warn?.("企稳规则加载失败", e);
  }
}

async function openAssessment(alert: StabilizationAlert) {
  showModal.value = true;
  assessing.value = true;
  assessedName.value = alert.name;
  try {
    assessment.value = await api.assessStabilization(alert.id);
  } catch (e) {
    message.warning(String(e));
    showModal.value = false;
  } finally {
    assessing.value = false;
  }
}

function boolText(v: boolean): string {
  return v ? "满足" : "未满足";
}

onMounted(async () => {
  await reload();
  // 后台评估产生状态变更时刷新
  await listen("stabilization-changed", () => {
    void reload();
  });
});
</script>

<template>
  <n-card v-if="enabledAlerts.length > 0" size="small" title="智能企稳状态" style="margin-top: 12px">
    <div v-for="alert in enabledAlerts" :key="alert.id" class="stab-row">
      <div class="stab-main">
        <div class="stab-title">
          <n-tag
            size="small"
            :type="STABILIZATION_STATE_META[alert.state].type"
            :bordered="false"
          >
            {{ STABILIZATION_STATE_META[alert.state].text }}
          </n-tag>
          <span class="stab-name">{{ alert.name }}</span>
        </div>
        <div class="stab-sub">
          观察价 ≤¥{{ alert.observationPrice }}
          <template v-if="alert.observationLow">
            · 阶段最低 ¥{{ alert.observationLow }}
          </template>
        </div>
      </div>
      <n-button size="tiny" quaternary type="primary" @click="openAssessment(alert)">
        查看判断依据
      </n-button>
    </div>

    <n-modal
      v-model:show="showModal"
      preset="card"
      :title="`企稳分析详情 · ${assessedName}`"
      style="width: 340px"
    >
      <template v-if="assessment">
        <n-descriptions :column="1" size="small" label-placement="left" bordered>
          <n-descriptions-item label="观察价">
            ≤ ¥{{ assessment.observationPrice }}
          </n-descriptions-item>
          <n-descriptions-item label="阶段最低">
            {{ assessment.observationLow ? `¥${assessment.observationLow}` : "--" }}
          </n-descriptions-item>
          <n-descriptions-item label="有效观察交易日">
            {{ assessment.validDayCount }} 天
          </n-descriptions-item>
          <n-descriptions-item label="低点稳定">
            {{ boolText(assessment.lowStable) }}
          </n-descriptions-item>
          <n-descriptions-item label="波动收窄">
            {{ boolText(assessment.narrowing) }}
          </n-descriptions-item>
          <n-descriptions-item label="反弹突破">
            {{
              assessment.breakoutPrice
                ? `${boolText(assessment.breakoutMet)}（参考 ¥${assessment.breakoutPrice}）`
                : "--"
            }}
          </n-descriptions-item>
          <n-descriptions-item label="数据充分性">
            {{ assessment.dataSufficient ? "充足" : "不足（暂停确认）" }}
          </n-descriptions-item>
          <n-descriptions-item label="当前状态">
            {{ STABILIZATION_STATE_META[assessment.state as keyof typeof STABILIZATION_STATE_META]?.text ?? assessment.state }}
          </n-descriptions-item>
        </n-descriptions>
        <div class="assess-tip">
          企稳判断仅为参考信号，不保证价格已经见底，请结合银行 App 实际行情综合评估。
        </div>
      </template>
    </n-modal>
  </n-card>
</template>

<style scoped>
.stab-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 0;
}
.stab-row + .stab-row {
  border-top: 1px solid rgba(128, 128, 128, 0.15);
}
.stab-title {
  display: flex;
  align-items: center;
  gap: 8px;
}
.stab-name {
  font-weight: 600;
}
.stab-sub {
  margin-top: 4px;
  font-size: 12px;
  color: #999;
}
.assess-tip {
  margin-top: 10px;
  font-size: 12px;
  color: #999;
  line-height: 1.6;
}
</style>
