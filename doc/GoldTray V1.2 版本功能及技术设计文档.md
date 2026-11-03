# GoldTray V1.2 版本功能及技术设计文档

**版本：** V1.2  
**日期：** 2026-11-02  
**目标平台：** macOS / Windows  
**技术架构：** 沿用现有 Tauri 2 + Vue 3 + TypeScript + Rust  
**版本目标：** 历史行情可视化、智能企稳识别、组合预警、风险提示

---

## 1. 版本背景

### 1.1 当前功能

GoldTray 已具备以下主要功能：

- 招商银行黄金活期参考报价查询
- macOS 菜单栏和 Windows 系统托盘行情展示
- 定时自动刷新
- 自定义价格上涨、下跌预警
- 多规则独立管理
- 系统通知
- 价格穿越判断
- 预警冷却和重复触发控制
- 通过预设价差估算客户卖出价

当前存在两个不足。

**问题一：缺少历史行情走势图。**

用户只能查看当前报价，无法直观了解近期涨跌趋势、历史高低点及价格变化。

**问题二：价格预警无法判断是否企稳。**

例如，客户买入参考价下跌至835元/克时，现有系统只能检测价格是否达到阈值，无法判断是否仍在快速下跌。

### 1.2 V1.2 建设目标

1. **历史行情可视化：** 在主页新增历史金价走势图，支持六个时间区间、价格悬停详情和区间统计。
2. **智能企稳预警：** 在原有价格预警基础上，增加下跌观察、初步企稳、反弹确认三级识别能力，并与走势图联动。

企稳判断只生成参考信号，不保证价格已经见底，不自动执行交易。

---

## 2. V1.2 功能范围

| 功能模块 | 需求 | 优先级 |
|---|---|---|
| 历史走势图 | 绿色面积图、时间区间切换 | P0 |
| 历史行情统计 | 最高、最低、平均、涨跌幅 | P0 |
| 历史接口适配 | 数据解析、校验、标准化 | P0 |
| 图表交互 | 悬停价格详情、加载及异常状态 | P0 |
| 智能企稳预警 | 到达观察价格后启动监测 | P0 |
| 初步企稳识别 | 连续有效交易日未明显创新低 | P0 |
| 反弹确认 | 突破近期价格区间高点 | P0 |
| 预警状态管理 | 状态持久化、去重及失效重置 | P0 |
| 企稳状态展示 | 首页状态及判断依据 | P1 |
| 图表预警联动 | 观察价、低点、确认点标记 | P1 |
| 历史缓存 | SQLite持久化及缓存更新 | P1 |

V1.2 不包含自动交易、银行账户登录、自动同步实际持仓、复杂量化预测及多银行支持。

---

## 3. 历史行情接口设计

### 3.1 最新价格接口

保留现有接口：

```http
GET https://api2.huilvsudi.com/app-api/mp/quote-detail/price-detail?bank=CMB&currency=GOLD_DEMAND
```

已验证返回格式：

```json
{
  "code": 0,
  "data": {
    "sellingPrice": "837.60",
    "change": 0.54,
    "bankName": "招商银行",
    "bankShortName": "招行",
    "currencyName": "黄金活期",
    "productType": "GOLD"
  },
  "msg": ""
}
```

`sellingPrice` 已通过人工核对与当时招商银行客户买入价一致，但应标识为第三方参考报价。

### 3.2 历史行情接口

```http
GET https://api2.huilvsudi.com/app-api/mp/quote-detail/chart-data
```

参数：

| 参数 | 值 |
|---|---|
| bank | CMB |
| currency | GOLD_DEMAND |
| priceType | SELL |
| period | 14d |

支持周期：

| 页面选项 | 接口参数 |
|---|---|
| 2天 | 48H |
| 14天 | 14d |
| 1个月 | 1M |
| 3个月 | 3M |
| 6个月 | 6M |
| 12个月 | 12M |

### 3.3 返回数据结构

```typescript
interface HistoryResponse {
  code: number;
  data: {
    categories: string[];
    series: Array<{
      name: string;
      time: string[];
      data: Array<number | null>;
    }>;
    stats: {
      startDate: string | null;
      totalDays: number | null;
      maxPrice: string | null;
      maxPriceTime: string | null;
      minPrice: string | null;
      minPriceTime: string | null;
      avgPrice: string | null;
      changePercent: string | null;
    };
  };
  msg: string;
}
```

目前确认包含最小值、最大值和平均值三类时段序列。

按 `series.name` 识别数据，禁止依赖数组固定顺序。

### 3.4 数据标准化

```typescript
interface HistoricalPricePoint {
  timestamp: string;
  minPrice: number | null;
  maxPrice: number | null;
  avgPrice: number | null;
  minPriceTime?: string;
  maxPriceTime?: string;
  quality: "VALID" | "CARRIED" | "MISSING" | "SUSPECT";
}
```

要求：

- 使用 `categories` 作为主时间轴。
- 按索引关联各序列。
- 时间统一为 `Asia/Shanghai`。
- 验证价格数值、时间顺序及高低价关系。
- 保留原始数据。
- 识别长时间重复报价。
- 对缺失数据单独标记。
- 不把小时均价当作日线收盘价。

---

## 4. 主页历史走势图

### 4.1 页面布局

走势图放置于当前报价卡片下方、预警信息上方。

```text
┌──────────────────────────────────┐
│ GoldTray                         │
├──────────────────────────────────┤
│ 行情        预警        设置      │
├──────────────────────────────────┤
│ 招商银行 · 黄金活期               │
│                                  │
│ ¥837.60 /g                       │
│ 第三方客户买入参考价              │
├──────────────────────────────────┤
│ 2天 14天 1月 3月 6月 12月         │
│                                  │
│          历史价格走势图           │
│       绿色折线 + 渐变面积         │
│                                  │
├──────────────────────────────────┤
│ 最高  ¥901.18   最低  ¥832.20     │
│ 平均  ¥864.75   涨跌  -6.93%      │
├──────────────────────────────────┤
│ 智能企稳状态                     │
│ 观察中 · 尚未确认企稳              │
│ [查看判断依据]                   │
├──────────────────────────────────┤
│ 当前价格预警                     │
│ ↓835.00 第一档加仓               │
│ ↓800.00 第二档加仓               │
└──────────────────────────────────┘
```

价格仅为历史接口响应示例，不能硬编码成实时行情。

### 4.2 图表技术方案

使用 ECharts。

视觉效果参考汇率速递：

- 绿色折线。
- 浅绿色到透明的面积渐变。
- 默认不显示数据点。
- 自动计算纵轴范围。
- 简洁横轴时间标签。
- 鼠标悬停显示时段详情。
- 支持深色和浅色模式。
- 支持 Retina 和 Windows 高DPI屏幕。

默认使用时段平均值绘图。

### 4.3 时间范围切换

默认14天。

切换时：

1. 优先读取本地缓存。
2. 检查缓存有效期。
3. 必要时调用历史接口。
4. 更新图表及统计。
5. 保留用户选择的周期。

### 4.4 悬停详情

```text
2026-11-02 14:00

最低：832.20元
最高：839.80元
平均：836.50元
```

显示对应数据点的真实数值，不跨时段拼接。

### 4.5 区间统计

展示：

- 最高价
- 最低价
- 平均价
- 涨跌幅

优先使用接口返回的 `stats`，缺失时仅在统计口径明确的情况下本地计算。

### 4.6 异常处理

支持加载中、空数据、请求失败、缓存数据及数据过期状态。

历史行情异常不得影响现有实时价格提醒功能。

---

## 5. 智能企稳确认预警

### 5.1 新增预警类型

```typescript
type AlertType = "PRICE" | "STABILIZATION";
```

- `PRICE`：现有价格上涨、下跌预警。
- `STABILIZATION`：新增智能企稳预警。

两种规则独立管理，不能互相替代。

### 5.2 企稳状态机

```typescript
type StabilizationState =
  | "WAITING_FOR_DIP"
  | "OBSERVING"
  | "STABILIZING"
  | "CONFIRMED"
  | "INVALIDATED"
  | "INSUFFICIENT_DATA";
```

| 状态 | 含义 |
|---|---|
| WAITING_FOR_DIP | 等待价格进入观察区 |
| OBSERVING | 已触达，持续观察 |
| STABILIZING | 初步企稳 |
| CONFIRMED | 反弹确认 |
| INVALIDATED | 信号失效 |
| INSUFFICIENT_DATA | 数据不足 |

### 5.3 判断流程

```text
金价进入观察区
       │
       ▼
连续有效交易日未明显创新低？
       │
   ┌───┴───┐
   否      是
   │       │
继续观察   ▼
       下跌波动收窄？
           │
       ┌───┴───┐
       否      是
       │       │
    继续观察   ▼
          初步企稳
              │
              ▼
       突破近期高点？
              │
          ┌───┴───┐
          否      是
          │       │
       继续观察   ▼
           反弹确认
              │
              ▼
          发送通知
```

### 5.4 到价观察

例如：

```text
观察价：835.00元/克
当前价：834.80元/克
```

满足：

```typescript
currentBuyPrice <= observationPrice
```

进入 `OBSERVING`。

记录首次触达时间、触达价格、观察阶段最低价。

### 5.5 初步企稳条件

建议默认：

**条件A：连续2个有效交易日未明显创新低。**

允许配置低点容忍偏差，默认2元/克。

**条件B：下跌速度及波动明显收窄。**

根据有效历史行情判断，避免仅凭单个时段上涨确认止跌。

两个条件同时成立，进入 `STABILIZING`。

数据不足时不得触发确认。

### 5.6 反弹确认条件

默认以近期2～3个有效交易日的最高价作为突破参考。

满足突破后，进一步确认：

- 连续两个有效小时窗口保持在突破价以上；或
- 完整有效交易日结束后仍保持在突破价以上。

具体以可靠的最新报价或时段末报价为判断依据，不得将时段最高价直接当成收盘价格。

### 5.7 企稳失效规则

满足以下情况时撤销当前确认：

- 再次明显创新低。
- 历史数据出现严重缺失。
- 行情源异常。
- 用户修改观察价格。
- 用户手动重置规则。

需要记录原状态及失效原因。

### 5.8 交易日处理

必须区分：

- 有效交易日
- 未完成交易日
- 周末及休市
- 长时间重复报价
- 数据缺失时段

不能把连续48小时重复报价判断为连续两个交易日企稳。

交易日规则应可配置。无法核实有效交易日时，状态应降级为数据不足或初步观察。

---

## 6. 智能企稳预警配置

新增独立规则表单。

```text
新增智能企稳预警

规则名称：
[ 第一轮加仓企稳确认 ]

监控产品：
[ 招商银行黄金活期 ]

观察价格：
[ 835.00 ] 元/克

观察交易日：
[ 2 ] 天

允许低点偏差：
[ 2.00 ] 元/克

突破参考：
[ 最近3个有效交易日高点 ]

确认方式：
[ 连续两个有效小时窗口 ]

通知阶段：
☑ 初步企稳
☑ 反弹确认

预警模式：
● 仅提醒一次
○ 重复提醒

[取消]           [保存]
```

高级配置包含：

- 观察交易日数
- 低点容忍偏差
- 波动收窄阈值
- 突破参考周期
- 突破缓冲幅度
- 确认持续时间
- 再次破位条件
- 冷却时间

### 6.1 预警示例

```text
第一轮加仓企稳确认

观察价格：≤835元
当前状态：观察中
观察阶段最低：832.20元
有效观察交易日：1天

[查看判断依据]
```

### 6.2 系统通知

初步企稳：

```text
GoldTray · 初步企稳

招商银行黄金活期

价格进入835元观察区后，
近期出现初步止跌信号。

建议继续观察，尚未完成反弹确认。
```

反弹确认：

```text
GoldTray · 反弹确认

招商银行黄金活期

当前行情满足设置的短期
反弹确认条件。

可人工评估是否分批加仓。
请核对招行App实际成交价格。
```

---

## 7. 走势图与预警联动

走势图增加以下标记：

- 观察价格水平线
- 首次进入观察区时间
- 观察阶段最低点
- 初步企稳点
- 反弹确认点
- 信号失效点

点击标记展示对应判断依据。

例如：

```text
企稳分析详情

观察价：835.00
阶段最低：832.20

有效观察交易日：2天
低点稳定：满足
波动收窄：满足
反弹突破：未满足

当前状态：初步企稳
```

历史信号不因失效而删除，应保留状态变更记录。

---

## 8. 数据存储设计

沿用现有SQLite数据库，采用迁移脚本新增表。

### 8.1 历史行情表

```sql
CREATE TABLE IF NOT EXISTS gold_history_bars (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    product_code TEXT NOT NULL,
    period TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    min_price TEXT,
    max_price TEXT,
    avg_price TEXT,
    quality TEXT NOT NULL DEFAULT 'VALID',
    fetched_at TEXT NOT NULL,
    UNIQUE(source, product_code, period, timestamp)
);

CREATE INDEX IF NOT EXISTS idx_gold_history_bars_time
ON gold_history_bars(product_code, timestamp);
```

### 8.2 智能企稳规则表

```sql
CREATE TABLE IF NOT EXISTS stabilization_alerts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    product_code TEXT NOT NULL,
    observation_price TEXT NOT NULL,
    stable_days INTEGER NOT NULL DEFAULT 2,
    low_tolerance TEXT NOT NULL DEFAULT '2.00',
    breakout_days INTEGER NOT NULL DEFAULT 3,
    breakout_confirm_hours INTEGER NOT NULL DEFAULT 2,
    enabled INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT 'WAITING_FOR_DIP',
    observation_started_at TEXT,
    observation_low TEXT,
    last_evaluated_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### 8.3 状态事件表

```sql
CREATE TABLE IF NOT EXISTS stabilization_events (
    id TEXT PRIMARY KEY,
    alert_id TEXT NOT NULL,
    previous_state TEXT,
    current_state TEXT NOT NULL,
    reference_price TEXT,
    reason TEXT,
    event_time TEXT NOT NULL
);
```

状态事件用于预警历史、图表标记、故障排查和通知去重。

---

## 9. 后端服务设计

保留现有Tauri Rust服务结构。

新增：

```text
src-tauri/src/
├── services/
│   ├── history_service.rs
│   ├── stabilization_engine.rs
│   └── scheduler.rs
├── providers/
│   └── huilvsudi.rs
├── models/
│   ├── historical_quote.rs
│   └── stabilization.rs
└── storage/
    ├── history_repository.rs
    └── stabilization_repository.rs
```

### 9.1 HistoryService

负责：

- 获取历史行情
- 数据结构校验
- 标准化
- 缓存与持久化
- 提供走势图数据
- 提供企稳分析数据

### 9.2 StabilizationEngine

负责：

- 检测观察区触达
- 计算近期低点变化
- 判断波动收窄
- 判断反弹确认
- 状态机流转
- 保存事件
- 生成通知事件

### 9.3 Scheduler

推荐刷新间隔：

| 任务 | 默认频率 |
|---|---|
| 当前报价 | 60秒 |
| 48H历史行情 | 15分钟 |
| 14天历史行情 | 30分钟 |
| 1～12个月历史行情 | 60分钟 |
| 企稳评估 | 有效行情更新时触发 |

刷新频率应遵守第三方数据提供方的使用条款及限流要求。

不依赖前端窗口定时器，所有后台任务保持在Rust侧运行。

---

## 10. 性能与异常处理

### 10.1 历史缓存

- 不同时间周期独立缓存。
- 相同请求合并，避免重复调用。
- 缓存过期后后台刷新。
- 网络异常时显示旧数据及过期时间。
- 图表降采样只用于显示，不参与企稳算法。

### 10.2 价格精度

价格统一使用十进制类型计算，避免浮点累计误差。

展示保留2位小数，内部计算保留更高精度。

### 10.3 数据异常

以下情况暂停企稳确认：

- 行情源异常。
- 时序数据严重缺失。
- 无法确认有效交易日。
- 报价明显异常且未验证。
- 数据源长时间不更新。

### 10.4 跨平台适配

**macOS**

- 菜单栏继续常驻。
- 支持Retina显示。
- 图表适配深色模式。
- 系统原生通知。

**Windows**

- 继续使用系统托盘。
- 点击打开包含走势图的主窗口。
- 适配Windows 10/11及高DPI。
- 支持原生系统通知。

---

## 11. 测试与验收

### 11.1 走势图

- [ ] 六种时间区间正常切换。
- [ ] 统计信息对应选择周期。
- [ ] 鼠标悬停显示正确数据。
- [ ] 休市重复数据处理正确。
- [ ] 数据缺失时正确显示。
- [ ] 历史接口失败不影响当前报价。
- [ ] 深色模式显示正常。

### 11.2 企稳判断

- [ ] 到达观察价后进入观察状态。
- [ ] 连续下跌时不误报企稳。
- [ ] 连续有效交易日不创新低时能识别止跌。
- [ ] 波动收窄条件正确计算。
- [ ] 反弹突破确认需要持续满足条件。
- [ ] 盘中瞬时冲高不误报确认。
- [ ] 再次明显创新低后撤销确认。
- [ ] 休市期间不误判交易日数量。
- [ ] 数据不足时不输出确认信号。
- [ ] 同一事件不重复通知。
- [ ] 重启应用后状态恢复正常。

### 11.3 兼容性

在macOS和Windows分别测试：

- 托盘运行
- 走势图渲染
- 历史行情刷新
- 智能预警配置
- 系统通知
- 数据持久化
- 系统休眠恢复
- 安装包运行

---

## 12. 开发实施顺序

**阶段一：历史行情接口接入**

完成 `chart-data` 调用、结构解析、数据校验及缓存。

**阶段二：主页走势图开发**

完成六个周期切换、面积图、悬停详情、统计信息和主题适配。

**阶段三：智能企稳引擎**

完成有效交易日聚合、低点判断、波动收窄、反弹确认和状态机。

**阶段四：预警系统集成**

新增智能企稳规则配置、状态展示、系统通知及图表信号标记。

**阶段五：双平台测试与发布**

完成自动化测试、真机兼容性测试、数据库迁移及安装包发布。

---

## 13. V1.2 验收与交付

本版本完成后应交付：

1. macOS 和 Windows 安装包。
2. 完整源代码。
3. 历史行情接口适配实现。
4. 主页走势图。
5. 智能企稳引擎。
6. 智能预警配置及通知功能。
7. 数据库迁移脚本。
8. 核心算法测试。
9. 跨平台兼容性测试结果。
10. V1.2版本更新说明。

**GoldTray V1.2 最终定位：** 在现有黄金行情及到价预警能力基础上，新增历史走势分析和分阶段企稳判断，让用户不仅能够知道价格是否达到目标，还能了解价格触达之后是否出现初步止跌与反弹信号。所有信号仅用于辅助判断，不作为自动交易指令。
