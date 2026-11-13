import { expect, test } from "@playwright/test";

const fetchedAt = new Date().toISOString();
const productCode = "GOLD";
const fixture = {
  get_latest_quote: {
    productCode, bankName: "测试银行", productName: "测试黄金",
    customerBuyPrice: "850.13", fetchedAt, status: "LIVE",
  },
  get_settings: { refresh_interval_secs: "30", hud_opacity: "0.85" },
  get_display_options: { hudEnabled: true, trayTitleEnabled: false },
  "plugin:autostart|is_enabled": false,
  "plugin:app|version": "1.3.1",
  get_history: {
    period: "14d", fetchedAt, fromCache: true, stale: false,
    points: [1, 2, 3].map((day) => ({
      timestamp: `2026-10-0${day}T09:00:00+08:00`,
      avgPrice: 850 + day, minPrice: 848 + day, maxPrice: 852 + day,
      quality: "VALID",
    })),
    stats: { avgPrice: "852.00", minPrice: "849.00", maxPrice: "855.00", changePercent: "0.24" },
  },
  list_alerts: [{
    id: "price", name: "价格夹具规则", productCode, direction: "ABOVE",
    priceType: "CUSTOMER_BUY", targetPrice: "860.00", triggerMode: "ONCE",
    cooldownMinutes: 10, enabled: true, armed: true, confirmHits: 0,
  }],
  list_alert_history: [{
    id: "trigger", alertId: "price", alertName: "价格夹具规则", direction: "ABOVE",
    targetPrice: "860.00", triggeredPrice: "861.00", triggeredAt: fetchedAt, isEstimated: false,
  }, {
    id: "offline", alertId: "price", alertName: "价格夹具规则", direction: "ABOVE",
    targetPrice: "860.00", triggeredPrice: "862.00", isEstimated: false,
    historicalFrom: "2026-11-12T09:00:00+08:00", triggeredAt: "2026-11-12T09:10:00+08:00",
  }],
  list_stabilization_alerts: [
    { id: "stabilization", name: "企稳夹具规则", state: "WAITING_FOR_DIP", observationPrice: "851.00" },
    { id: "unknown", name: "未知状态夹具规则", state: "FUTURE_STATE", observationPrice: "850.00" },
  ].map((rule) => ({
    productCode, stableDays: 3, lowTolerance: "2.00", breakoutDays: 5, breakoutConfirmHours: 2,
    triggerMode: "ONCE", cooldownMinutes: 10, enabled: true, ...rule,
  })),
  list_stabilization_events: [{
    id: "event", alertId: "stabilization", alertName: "企稳夹具规则",
    currentState: "OBSERVING", reason: "观察价已触达", eventTime: fetchedAt,
  }, {
    id: "event-2", alertId: "unknown", alertName: "未知状态夹具规则",
    currentState: "WAITING_FOR_DIP", reason: "等待触达", eventTime: fetchedAt,
  }],
};

test("有数据时主标签可切换，行情图可在隐藏期间调整尺寸后恢复", async ({ page }) => {
  const runtimeErrors: string[] = [];
  page.on("pageerror", (error) => runtimeErrors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error" && !message.text().includes("Failed to load resource: the server responded with a status of 404")) {
      runtimeErrors.push(message.text());
    }
  });
  await page.addInitScript((responses) => {
    let listenerId = 0;
    let stabilizationEvents = responses.list_stabilization_events;
    let alertHistory = responses.list_alert_history;
    Object.assign(window, {
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: () => ++listenerId,
        invoke: async (command: string) => {
          if (command === "plugin:event|listen") return ++listenerId;
          if (command === "plugin:event|unlisten") return;
          if (command === "list_stabilization_events") return stabilizationEvents;
          if (command === "list_alert_history") return alertHistory;
          if (command === "clear_alert_history") {
            alertHistory = [];
            return;
          }
          if (command === "clear_stabilization_events") {
            stabilizationEvents = [];
            return;
          }
          if (Object.hasOwn(responses, command)) return responses[command as keyof typeof responses];
          throw new Error(`未提供 Tauri 测试响应: ${command}`);
        },
      },
    });
  }, fixture);

  await page.goto("/");
  const dashboard = page.locator(".dashboard");
  const chart = dashboard.locator(".chart canvas");
  await expect(dashboard.getByText("¥ 850.13", { exact: true })).toBeVisible();
  await expect(dashboard.getByText("¥852.00", { exact: true })).toBeVisible();
  await expect(dashboard.getByText("等待触达", { exact: true })).toBeVisible();
  await expect(dashboard.getByText("未知状态", { exact: true })).toBeVisible();
  await expect(chart).toBeVisible();
  const originalWidth = await chart.evaluate((canvas) => canvas.clientWidth);

  const selectTab = (name: string) => page.locator(".app-header").getByText(name, { exact: true }).click();
  await selectTab("预警");
  await expect(chart).toBeHidden();
  await expect(page.getByText("价格夹具规则", { exact: true })).toBeVisible();
  await expect(page.getByText("触发价 ¥861.00 / 目标 ¥860.00")).toBeVisible();
  await expect(page.getByText("价格夹具规则（离线补记 · 参考价）")).toBeVisible();
  await expect(page.getByText("时段极值 ¥862.00 / 目标 ¥860.00")).toBeVisible();
  await expect(page.locator(".alert-manager .n-timeline").getByRole("button", { name: "删除" })).toHaveCount(0);
  await page.locator(".alert-manager .history-header").getByRole("button", { name: "清空" }).click();
  await page.getByRole("button", { name: "确认", exact: true }).click();
  await expect(page.getByText("触发价 ¥861.00 / 目标 ¥860.00")).toHaveCount(0);
  await page.getByText("智能企稳", { exact: true }).click();
  await expect(page.locator(".alert-manager").getByText("企稳夹具规则", { exact: true }).first()).toBeVisible();
  await expect(page.locator(".alert-manager").getByText("未知状态", { exact: true }).first()).toBeVisible();
  await expect(page.locator(".n-timeline").getByText(/企稳夹具规则 → 观察中/).first()).toBeVisible();
  await expect(page.locator(".stab-manager .n-timeline").getByRole("button", { name: "删除" })).toHaveCount(0);
  await page.locator(".history-header").last().getByRole("button", { name: "清空" }).click();
  await page.getByRole("button", { name: "确认", exact: true }).click();
  await expect(page.locator(".stab-manager .n-timeline")).toHaveCount(0);

  await selectTab("设置");
  await expect(page.getByText("显示设置", { exact: true })).toBeVisible();
  await expect(page.getByText("开发者 · Tyon Peng", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "检查更新" })).toHaveCount(0);
  await expect(page.locator(".settings-view").getByText("30 秒", { exact: true })).toBeVisible();
  await page.setViewportSize({ width: 500, height: 720 });
  await selectTab("行情");
  await expect(chart).toBeVisible();
  await expect.poll(() => chart.evaluate((canvas) => canvas.clientWidth)).toBeGreaterThan(originalWidth);
  await expect.poll(() => chart.evaluate((element) => {
    const canvas = element as HTMLCanvasElement;
    const pixels = canvas.getContext("2d")?.getImageData(0, 0, canvas.width, canvas.height).data;
    return pixels?.some((channel) => channel !== 0);
  })).toBe(true);
  expect(runtimeErrors).toEqual([]);
});
