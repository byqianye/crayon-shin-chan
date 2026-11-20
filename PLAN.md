# 影视聚合桌面端 · 二开实施计划（融合版）

> 日期：2026-11-18
> 底座：`hgtuj5688/tvbox-desktop` @ `04274df`（MIT）
> 目标：Windows 10/11 x64 桌面软件；源可手动添加；展示时按作品去重

## 0. 决策摘要

- **路线**：Fork `tvbox-desktop` 二开，保留其 Electron + React + TS 三层架构。
- **不重写**：配置源解析、CMS/网页规则、聚合搜索并发池、ArtPlayer 播放、直播、外部播放器、177 项自检全部保留。
- **重点补强**：单条影视源手动添加、TVBox 多仓导入、内容级去重（名称+年份+类型）、源健康度调度、源审计、订阅墓碑。
- **架构升级**：把与 UI 无关的领域逻辑抽到 `src/core/`（纯 TS、无 Electron/React 依赖），便于单测和未来复用。
- **工期**：单人约 8–11 个工作日到可用版。

## 1. 底座评估（已读源码核实）

| 能力 | 现状 | 结论 |
|---|---|---|
| 架构 | 主进程网络/文件，preload `contextBridge`，渲染进程纯展示 | 保留 |
| 存储 | `data/` 下 JSON，便携、零原生依赖 | 保留，抽 repository 接口 |
| 配置源 | 支持 TVBox 方言 + 简洁方言，URL/本地文件 | 保留 |
| 影视源 | 只能管理配置源解析出的站点，**不能单独添加一个源** | 必补 |
| 多仓 | **不支持** TVBox 多仓清单 | 必补 |
| 聚合搜索 | 并发池 + 流式进度 + 可取消 | 保留 |
| 去重 | `mergeResults()` 只按 `normalizeName()` 分组，**不含年份/类型** | 必升级 |
| 归一化 | 已处理别名、季数、画质噪声、解说过滤 | 保留并扩展 |
| 播放 | ArtPlayer + hls.js、外部播放器探测、解析接口轮询 | 保留 |
| 直播 | m3u/txt、同名频道合并、线路体检 | 保留 |
| 自检 | `pnpm selftest` 177 项断言 | 扩展 |

## 2. 上游项目优点融合矩阵

> 原则：MIT 项目可复用代码；GPL/AGPL/CC-NC 项目**只借鉴产品思路与算法思想，不复制代码**。

| 上游 | 吸收的优点 | 落到本项目 |
|---|---|---|
| **tvbox-desktop**（MIT） | Electron 三层架构、便携 JSON 存储、双方言配置解析、并发流式搜索、保守归一化、ArtPlayer/hls.js、外部播放器、直播线路体检、selftest | 全量保留为底座 |
| **MisaTV**（GPL-3.0，只借鉴） | ① 聚合搜索「名称+年份」去重并标注来源；② 订阅维护：启动静默回源、墓碑（删掉的源不复活）、本地新增源保留、批量删除、体检后一键删失败源、站点字段编辑；③ 播放失败自动换线 | 去重键加年份/类型；新增 `tombstones.ts`；源列表批量操作；播放换线复用现有能力 |
| **xiugou-tv**（GPL-3.0，只借鉴） | ① TVBox 配置 JSON、多仓清单、CMS 资源站清单导入，多仓可指定具体仓库；② 站点自检（分类数量与可用性）；③ 相对地址还原；④ 历史/收藏备份恢复 | 新增多仓解析与导入向导；新增 `audit.ts`；补 `normalizeApi` 相对地址；备份恢复沿用现有 library/settings |
| **QuantumTV**（CC BY-NC-SA，只借鉴） | ① 源智能调度：统计响应时延/成功率，慢源失败源自动降权；② 核心逻辑与 UI 解耦；③ SQLite 本地存储；④ 多源聚合自动去重 | `health.ts` 打分调度；抽 `src/core/`；存储先 JSON、留 SQLite 接口；去重升级 |
| **FreeBox**（GPL-3.0，只借鉴） | 源审计：自动跑站点分类/搜索/详情，筛出不可用站点并给原因 | `audit.ts` 三步审计报告 + 一键停用/删除 |
| **zyfun / ZYPlayer**（AGPL-3.0，只借鉴） | 成熟源管理交互：筛选、批量、源详情编辑 | 源管理页信息架构参考 |
| **LibreTV / LunaTV / MoonTV**（Web，许可证逐个确认） | 自定义源 JSON 的用户习惯、多源并发搜索 | 后续增加「导入 Web 端源配置」兼容层，不复制代码 |

**明确不吸收**：TVBox spider（jar/dex/py）、网盘取流、磁力 BT、弹幕、投屏——首版不做，避免把项目拖入深水区。

## 3. 目标架构

```
src/
├─ shared/                    类型与 IPC 契约（单一事实来源）
├─ core/                      ★新增：纯 TS 领域层，无 Electron/React 依赖
│  ├─ source/detect.ts        源类型探测（cms / tvbox / multirepo / m3u）
│  ├─ dedupe/normalize.ts     片名归一化、年份提取、季数提取
│  ├─ dedupe/merge.ts         搜索结果合并（纯函数）
│  └─ health/score.ts         源健康度打分与熔断判定
├─ main/
│  ├─ services/
│  │  ├─ sources.ts           配置源 CRUD + 同步
│  │  ├─ localSites.ts        ★新增：手动添加的单个影视源
│  │  ├─ multirepo.ts         ★新增：TVBox 多仓解析
│  │  ├─ sites.ts             站点汇总（配置源 + 本地源）、覆盖、排序
│  │  ├─ health.ts            ★新增：健康度持久化与查询
│  │  ├─ audit.ts             ★新增：站点三步审计
│  │  ├─ tombstones.ts        ★新增：删除墓碑
│  │  ├─ search.ts            聚合搜索（接健康度排序）
│  │  ├─ detail.ts            详情/线路
│  │  └─ …                    现有 cms/http/parse/player/live 等
│  └─ index.ts                IPC 注册
├─ preload/                   contextBridge
└─ renderer/                  React 页面与组件
```

数据流：

```
添加源 → detectSource() → 源记录（config | manual | multirepo）
       → parseConfig() → Site[]
搜索   → searchAll(并发池, 健康度加权) → SiteSearchResult[]
       → core/merge(归一化 + 年份 + 类型 + 相似度) → MergedVod[]（含 sources[]）
详情   → 选中 MergedVod → 各源详情 → 线路按源分组
播放   → 线路 URL → 主进程解析/代理 → ArtPlayer / 外部播放器
```

## 4. 核心设计

### 4.1 源模型与手动添加

新增「手动影视源」，与配置源并列：

```ts
// localSites.json
interface LocalSite extends Site {
  origin: 'manual'
  createdAt: number
  updatedAt: number
}
```

- `key` 统一前缀 `manual_`，避免与订阅站点冲突。
- 添加表单：名称、类型（自动探测可改）、API URL、请求头（UA/Referer）、超时、启用。
- 探测流程：请求 URL → 含 `sites[]` 判为 TVBox；含 `urls[]/repos[]` 判为多仓；返回 CMS JSON 判为 CMS；`#EXTM3U` 判为直播。
- 重复判断：规范化 URL（小写 host、去尾斜杠、去默认端口）比对，重复则提示合并/覆盖。
- 批量导入：多行 URL、剪贴板粘贴，逐条探测后预览再入库。
- `sites.ts::getSites()` 合并顺序：手动源 > 配置源；同 key 手动优先。

### 4.2 TVBox 多仓导入

支持三种常见形态，统一归一化为仓库列表：

```json
{ "urls": [{ "name": "仓库A", "url": "https://…/a.json" }] }
{ "repos": [{ "name": "仓库A", "url": "https://…/a.json" }] }
[ { "name": "仓库A", "url": "https://…/a.json" } ]
```

- 同步时逐个拉取子配置，任一仓库失败不影响其他仓库。
- `ConfigSource.kind` 增加 `multirepo`，记录子仓库数、成功/失败数。
- UI 支持展开查看子仓库，并可只启用其中某几个。

### 4.3 内容去重（核心）

分三层：

**① 源级去重**
- 配置源：规范化 URL 相同视为同一源。
- 站点：`site.key` 相同，先导入的优先；手动源覆盖配置源。

**② 内容级去重**

```ts
typeBucket = type_name 映射为：movie | tv | anime | variety | other
season     = 从标题提取「第N季 / Season N / S01」
dedupeKey  = `${typeBucket}|${normalizeName(title)}|${season}|${year}`
```

- 年份优先 `vod_year`，缺失时从标题 `(2024)` 提取。
- 标题完全相等且年份未知 → 合并。
- 相似度兜底：同 `typeBucket`、年份相同或都缺失、Levenshtein 相似度 ≥ 0.92 才合并。
- 保护规则：电影/剧集/动漫/综艺不跨类合并；季数不同不合并；同名不同年份不合并。
- 代表条目：沿用 `scoreCandidate()`（海报 > 备注 > 评分），并优先健康度高的源。
- 合并结果保留 `sources: Array<{ siteKey, siteName, vodId, year, remarks, health }>`。

**③ 线路级去重**
- 同一播放地址去重。
- 同一源内同名线路合并；线路名保留原始名称，展示时标注来源。

**展示形态**
- 搜索卡片角标「N 源」。
- 点开卡片展示来源列表；详情页线路按「源 → 线路」两级分组，默认选健康度最高的线路。
- 播放失败自动切下一条线路（复用现有能力）。

**测试用例（必须覆盖）**
- 庆余年 / 庆余年 第二季 / 庆余年（第二季）→ 合并；第一季与第二季分开。
- 射雕英雄传 1983 / 2008 / 2017 → 三张卡。
- 电影 / 电视剧同名 → 不合并。
- 解说、速看、混剪 → 过滤。

### 4.4 源健康度与调度

```ts
interface SiteHealth {
  siteKey: string
  successRate: number      // 最近 20 次滑动窗口
  avgMs: number
  consecutiveFails: number
  lastOkAt?: number
  lastFailAt?: number
  cooldownUntil?: number   // 熔断截止
}
```

- 每次搜索/详情/审计上报结果，落 `health.json`。
- 打分：`0.6 * successRate + 0.3 * (1 - min(avgMs, 3000) / 3000) + 0.1 * 新鲜度`。
- 连续失败 ≥ 3 → 冷却 10 分钟，搜索跳过并在 UI 标注「熔断中」。
- 冷启动默认 0.5，避免新源被永久压底。
- 搜索目标排序按分数降序，但用户手动设定的源顺序优先于健康度。

### 4.5 源审计

对单个站点依次执行：

1. 拉分类列表（CMS `ac=list` / 网页规则）
2. 搜索一个探针词（默认「测试」，可配置）
3. 取首条结果拉详情，解析线路与剧集数

输出：`{ ok, categories, searchHits, detailLines, episodes, ms, failStage, reason }`。

UI：源列表「审计」按钮；审计结果支持「一键停用失败源」「一键删除手动失败源」。

### 4.6 订阅墓碑

```ts
// tombstones.json
{
  "sourceUrls": ["https://…"],
  "siteKeys": ["csp_xxx"]
}
```

- 删除配置源/站点时写入墓碑。
- 同步订阅时过滤墓碑中的站点，避免「删了又回来」。
- 设置页提供「清空墓碑」入口并说明后果。

### 4.7 播放链路

- 保留 ArtPlayer + hls.js；主进程负责请求头、Referer、代理。
- 详情页线路按源分组，默认健康度最高；失败自动切下一条并提示。
- 收藏/历史主键从 `${siteKey}::${vodId}` 升级为 `${dedupeKey}::${siteKey}::${vodId}`，同时保留旧数据迁移。
- 外部播放器（PotPlayer/VLC/mpv/MPC-HC）继续可用。

## 5. 文件级改造清单

| 文件 | 改动 |
|---|---|
| `src/shared/types.ts` | 新增 `LocalSite`、`SiteHealth`、`AuditResult`、`MergedVod.sources`、`ConfigSource.kind='multirepo'` |
| `src/shared/ipc.ts` | 新增 `localSites:*`、`sites:audit`、`sites:remove-failed`、`health:get`、`tombstones:*` |
| `src/main/services/config.ts` | 导出 `normalizeSite`；补相对地址还原 |
| `src/main/services/sources.ts` | URL 规范化去重；多仓同步；墓碑过滤 |
| `src/main/services/localSites.ts` | **新增**：手动源 CRUD、持久化、与 sites 合并 |
| `src/main/services/multirepo.ts` | **新增**：三种多仓格式解析、子仓库同步 |
| `src/main/services/sites.ts` | 合并 local sites；排序按用户顺序 + 健康度；墓碑过滤 |
| `src/main/services/health.ts` | **新增**：健康度记录、打分、熔断 |
| `src/main/services/audit.ts` | **新增**：三步审计 |
| `src/main/services/tombstones.ts` | **新增**：墓碑读写 |
| `src/main/services/search.ts` | 搜索上报 health；目标按分数排序；缓存 |
| `src/main/services/detail.ts` | 详情上报 health；线路源标注 |
| `src/main/index.ts` | 注册新 IPC |
| `src/preload/index.ts` / `index.d.ts` | 暴露新 API |
| `src/core/dedupe/normalize.ts` | 从 `renderer/utils/normalize.ts` 迁入并扩展年份/季数/相似度 |
| `src/core/dedupe/merge.ts` | 从 `useApp.ts` 抽离 `mergeResults`，改为纯函数 |
| `src/core/source/detect.ts` | **新增**：源类型探测 |
| `src/core/health/score.ts` | **新增**：纯函数打分 |
| `src/renderer/src/pages/Sites.tsx` | 添加「添加影视源」「批量导入」「审计」「删失败源」入口 |
| `src/renderer/src/pages/Settings.tsx` | 多仓导入、自动同步开关、墓碑管理 |
| `src/renderer/src/pages/Search.tsx` | 来源角标、来源展开面板 |
| `src/renderer/src/pages/Detail.tsx` | 线路按源分组、默认健康源 |
| `src/renderer/src/store/useApp.ts` | 使用 core/merge；MergedVod 扩展 |
| `scripts/selftest.mjs` | 新增去重、源管理、审计断言 |

## 6. 里程碑与验收

| 阶段 | 内容 | 工期 | 验收 |
|---|---|---|---|
| M0 | 品牌化与基线：改应用名/appId/图标占位；`pnpm install` + `pnpm typecheck` + `pnpm build` 全绿 | 0.5–1 天 | 能打出安装包；typecheck/build 零错误 |
| M1 | 源管理：手动单源、批量导入、多仓、墓碑、审计、健康度 | 3–4 天 | 重复 URL 不新增；删除源同步后不复活；审计出报告；失败源可一键处理 |
| M2 | 去重引擎：core 抽离、年份+类型+季数、相似度、来源标注 | 2–3 天 | 4.3 的四组用例全过；搜索「庆余年」多源合并为 1 条 |
| M3 | 详情与播放：线路按源分组、失败换线、收藏/历史迁移 | 1–2 天 | 同一影片 ≥2 线路可播；换线正常；旧收藏不丢 |
| M4 | 打包验收：selftest 扩展、NSIS + portable、干净机器测试 | 1 天 | 自检全绿；无 Node 环境可运行；data/ 便携 |

**总计 8–11 个工作日。**

## 7. 测试策略

- 纯逻辑（归一化、去重、健康度打分、源探测）优先用现有 `scripts/selftest.mjs` 断言，不新增测试框架。
- 每个里程碑跑 `pnpm typecheck` + `pnpm build`；发版前跑 `pnpm selftest`。
- 真实源验收：至少 3 个公开 CMS 源 + 1 个 TVBox 订阅，覆盖搜索、详情、播放。
- 回归重点：分类浏览翻页顺序、直播频道合并、外部播放器、收藏历史。

## 8. 风险与边界

| 风险 | 应对 |
|---|---|
| 多仓格式不统一 | 兼容 `urls` / `repos` / 数组三种；子仓库失败隔离 |
| 去重误合并 | 保守策略：类型/季数/年份硬约束，相似度仅兜底；测试用例先行 |
| 源站失效 | 健康度 + 审计 + 一键停用/删除 |
| 健康度冷启动偏差 | 默认 0.5，用户手动排序优先 |
| Electron 体积/内存 | 首版接受；后续可抽 core 换 Tauri 壳 |
| 上游代码演进 | 保留 upstream remote，按需 cherry-pick；核心改动集中在 core 与新增 service |

**首版不做**：TVBox spider（jar/js/py）、网盘、磁力、弹幕、投屏、移动端。

## 9. 合规

- 软件不内置、不托管、不分发任何影视源或内容，只提供通用播放与管理工具。
- 内置推荐订阅仅含公开采集接口摘要，用户可删除；正式发行前复核其合法性与可用性。
- 保留上游 MIT LICENSE 与版权声明；GPL/AGPL/CC-NC 项目仅借鉴思路，不复制代码。

## 10. 下一步

1. 确认应用名称与品牌（影响 M0 的 appId、图标、窗口标题）。
2. 开始 M0：安装依赖、跑通 typecheck/build、改名换标。
3. 进入 M1：先做「手动单源 + 健康度」，再做「多仓 + 审计 + 墓碑」。

---

## 11. 实施进展（2026-11-18）

### 已完成

- **M0 基线**：`pnpm install`、`pnpm typecheck`、`pnpm build` 全部通过；Electron 44.4.5 二进制就绪。
- **M1 源管理**：
  - 手动单源添加 / 删除 / 批量导入（`src/main/services/localSites.ts`、`src/main/services/audit.ts`）
  - 多仓清单解析（`src/main/services/multirepo.ts`）
  - 源健康度与熔断（`src/main/services/health.ts`、`src/core/health/score.ts`）
  - 订阅墓碑（`src/main/services/tombstones.ts`）
  - 站点审计：分类 → 搜索 → 详情三步
  - 影视源页新增「添加影视源 / 批量导入 / 审计 / 忽略」；设置页新增忽略列表管理
- **M2 去重引擎**：
  - 归一化逻辑迁入 `src/core/dedupe/normalize.ts`，新增年份、季数、内容分类、相似度
  - 合并逻辑迁入 `src/core/dedupe/merge.ts`，按「类型 + 片名 + 季数 + 年份」去重
  - 网页规则站 `type_name` 为空时按通配处理，避免与 CMS 站拆成两条
  - 去重用例：庆余年分季、射雕三个年份、电影/电视剧同名、解说过滤、同站多版本、不同副标题，全部通过
- **回归**：`pnpm selftest` **177/177 通过**。

### 四个指定源的实测结果

> 测试时间：2026-11-18；请求 UA：`okhttp/3.12.0`（配置服务器按 UA 返回不同内容）。

| 源 | 包装格式 | 解密后站点 | 可直接点播 | 直播源 | 解析接口 | 结论 |
|---|---|---:|---:|---:|---:|---|
| `http://我不是.摸鱼儿.cc/` | 标准 TVBox JSON | 111（全部 `csp_*` jar） | 0 | 1 | 0 | 可导入；点播需 jar Spider；测试时源站偶发 `fetch failed` |
| `http://www.饭太硬.net/tv` | JPEG 尾部 Base64 + JSONC | 49（46 jar + 3 JS） | 0 | 5 | 0 | 可导入；直播可用；点播需 Spider |
| `http://肥猫.net` | `2423…2324` hex-tag-aes | 41（40 jar + 1 JS） | 0 | 1 | 4 | 自动解密后导入；解析/直播可用；点播需 Spider |
| `https://9877.kstore.space/sun.json` | `2423…2324` hex-tag-aes | 117（115 jar + 2 py） | 0 | 0 | 9 | 自动解密后导入；解析可用；点播需 Spider |

适配代码：

- `src/main/services/configPayload.ts`：okhttp UA、图片尾 Base64、hex-tag-aes 解密、JSONC 注释清理
- `src/core/source/detect.ts`：识别 `cms / tvbox / multirepo / m3u / html / encrypted / image`
- `src/main/services/config.ts`：`csp_*`（jar）、`.py`、`.js/drpy` 站点标记为不可直接用，避免误请求
- `ConfigSource.usableSiteCount`：配置源列表显示「站点 N（可用 M）」

### 关键限制

这四个源的点播能力全部依赖 TVBox Spider 运行时：

- jar（`csp_*`）：需要 JVM + dex2jar + Android 兼容层（unidbg），参考 MisaTV / xiugou-tv 的实现。
- `.py`：需要嵌入式 CPython。
- `.js`（drpy）：需要 drpy Node 运行时。

当时桌面端只支持 CMS 直连与网页规则，所以这些源导入后**点播站点会如实标灰**。
该限制已在第 13 节的 Spider 宿主 PoC 与第 14 节的 0.1.2 发布中解除（py/js 仍除外）。

### 下一阶段选项

- **A（推荐先做）**：接一个 CMS 直连源聚合包，保证「打开就能搜能播」，同时保留这四个源的直播/解析能力。
- **B**：集成 Spider 运行时（jar + py + js），真正解锁这四个源；按 MisaTV 的路线估算 2–4 周。
- **C**：先保持现状，继续完善源管理 / 去重 / UI。

## 12. A 方案执行结果（2026-11-18）

- 内置 `public-cms` 的 5 个苹果CMS 源实测**全部在线**：非凡、量子、电影天堂、光速、素博。
- 真实搜索「庆余年」：**5/5 站点有响应，25 条命中**，都可以进入详情。
- 真实详情：36 集 × 2 条线路；`ffm3u8` 线路直接返回 HTTP 200 的 master playlist；`subyun` 分享页也能解析出 `index.m3u8`。
- 四个 TVBox 源（肥猫/饭太硬/摸鱼/潇洒）继续作为可选远程预设，仅导入直播/解析接口，点播站点如实标灰。
- 开发数据目录已清空测试源与测试统计，删除 `bundled-sources` 标志；下次启动会自动安装 `public-cms`，装完即可搜可播。

## 13. Spider 宿主 PoC（2026-11-18 追加）

第 11 节的「关键限制」已被本轮 PoC 部分推翻：`csp_*` 站点现在可以通过
`spider-host` 在桌面端运行，不再一律标灰。

- 架构：`spider-host` 是一个独立 JVM 进程，Electron 通过 stdin/stdout JSON-RPC
  调用；所有站点共用一个 JVM，空闲 5 分钟退出。
- 转换：spider 文件（含 PNG/JPEG 尾部伪装的 zip、`classes.dex`）下载后按 MD5
  缓存，用 dex2jar 转成 JVM class；第二次直接复用。
- 实测：荐片 `csp_Jianpian` 搜索 20 条、详情 29 条线路、播放直链 m3u8；
  金牌 `csp_Jpys` 搜索 4 条、详情/播放均通过；JVM RSS 约 65MB，热态搜索 218–330ms。
- FishGuard native `ext` 解密已通过一次性 unidbg ARM 桥验证：摸鱼 `FishHxq`
  的加密 ext 能解成 `http://www.y456y.com/api//2026/fishhxq.php` 并缓存；
  摸鱼 `FishCloud` 走轻量兼容路径，首页 7 分类 / 251 条、搜索 4 条。
- 当前未覆盖：FishGuard 的播放级 native 字段/解密尚未全部补齐、依赖 WebView
  的 spider、Python/JS spider；会明确报错而不是静默跳过。

### 2026-11-18 源站复核

- `https://666.666291.xyz` 页面标题为「再见，我们跑路了」，其
  `/api.php/provide/vod/` 返回 `closed`；`www.muoua.top` 连接失败。
- `FishHuban` / `Fishshayang` 的镜像首页可访问，但 `FishCloud.detailContent`
  仍只返回影片元数据，`vod_play_from` / `vod_play_url` 为空。
- `FishHxq` 的解密 ext 指向 `www.y456y.com`，接口直接返回 403。
- 因此当前摸鱼 FishCloud/FishHxq 的播放不是 native 解析缺失，而是源站已关闭、
  需要账号/签名或没有返回播放字段；需要换存活源后再继续播放链路。
- 新增入口：`spider-host/build.ps1`、`build-runtime.ps1`、`fetch-deps.ps1`、
  `fetch-tools.ps1`；`src/main/services/spider.ts`；`scripts/spider-service-smoke.mjs`。

## 14. 0.1.2 发布与 Spider 运行时收口（2026-11-18）

- 修复 0.1.1 安装包未包含 `spider-host` 的发布遗漏：`extraResources` 现在会打进
  `runtime / out / lib / dex-tools / unidbg-*`；安装包 214MB，解包后
  `resources/spider-host` 约 110.6MB。
- `csp_*` 且配置带顶层 `spider` 的站点不再标 `unsupported`；影视源页显示
  `Spider / 未启用 / 内置 JVM 运行`，默认关闭，可逐条开启或用「启用可见 Spider」
  批量开启。
- 新增 `spider:status` / `spider:probe` IPC：影视源页显示运行时状态，并可一键自检
  JVM 与宿主协议。
- 安装版 0.1.2 用真实用户数据实测（300 个站点）：识别 289 个 Spider 站点，
  仅 6 个 py/js 站点明确不支持；运行时自检 161ms 通过；肥猫「荐片」连通性
  6.2s 通过（7 个分类 / 首页 10 条）；启用后搜索「庆余年」返回 5 条，276ms。
- 回归：`pnpm typecheck`、`pnpm build`、`pnpm selftest` 177/177 通过。

## 15. 0.1.3 Android 兼容层（2026-11-19）

- 实测 71 个失败 Spider 后定位到宿主 Android API 缺口：`SharedPreferences`、
  `AtomicFile`、`Base64`、`Log`、`ContentResolver`、`Build.HARDWARE`。
- 已在 `spider-host/src/main/java/android/` 补齐可运行实现；重新编译宿主。
- 典型改善：云岫/凝光从 `NoSuchMethodError` 变为可完成搜索调用；独播影视、
  哔哩合集、哔哩视频、半日99 从失败变为连通；`MTV` 首页返回 128 个分类。
- 剩余硬边界：`DexNative` 加固类（风云、AI剧、UC、剧迷等）仍报
  `UnsatisfiedLinkError`；`Pan.init` 自递归站点（爱盼、百酷、搬运）仍栈溢出；
  其余多为源站超时、TLS 或返回非 JSON。
- 0.1.3 安装包（214,178,504 字节）已安装；安装版实测云集 4K 首页 114 条、
  看球 4 个分类、文才 4 个分类、独播影视 81 条、哔哩合集 23 个分类。

## 16. 发布闭环（2026-11-20）

目标是把「本机能打包」升级成「任何人推一个 tag 就能产出可校验的 Release」。

- `.github/workflows/ci.yml`：三个 Windows 任务（类型/构建/脚本与版本守卫、
  JDK 17 宿主编译、177 项端到端自检），失败时上传开发窗口日志。
- `.github/workflows/release.yml`：`v*` tag 或手动触发；用 JDK 17 生成精简 JRE，
  打包、校验 `latest.yml` 版本、生成 `SHA256SUMS.txt` 与发布说明，最后只创建
  **draft Release**。已存在的正式 Release 会被拒绝覆盖，资产不足 5 个直接失败。
- 新增守卫脚本：`release-check.mjs`（版本/CHANGELOG/产物一致性）、
  `release-notes.mjs`（从 CHANGELOG 生成发布说明）、`check-lockfile.mjs`、
  `check-ci.mjs`（28 项工作流结构约束）。
- 统一包管理器：`packageManager: pnpm@11.25.0`，删除过期且包名错误的
  `package-lock.json`；CI 使用 `pnpm install --frozen-lockfile`。
- 本地已验证：`actionlint` 对两个工作流报 0 问题；`pnpm@11.25.0` 在只读副本上
  `--frozen-lockfile` 通过；两个负向用例（版本 0.1.4、CHANGELOG 9.9.9）均以
  退出码 1 失败；typecheck、build、177/177 自检通过。
- 2026-11-20 实跑收口：CI run 44705758715（代码）与 44709072170（文档）三个 job
  全绿；Release run 44706599956 全绿；`v0.1.3` tag 已推送，draft Release 已生成，
  9 个资产齐全（完整版
  exe / blockmap / zip，精简版 exe / blockmap / zip，latest.yml，
  SHA256SUMS.txt / SHA256SUMS-lite.txt）。draft 尚未转正式，正式发布前仍需用户确认。

## 17. Spider 真实性矩阵（2026-11-20）

- 新增 `scripts/spider-matrix.mjs`：每个用例独立 Node 进程 + 独立 JVM，逐类跑
  `home → category → search → detail → player`，输出 Markdown/JSON；单个 jar
  卡死或崩溃不会拖垮整批。
- 新增 `scripts/spider-contract.mjs`：合成 10 个 Spider 契约用例，覆盖缺类、
  类型不对、Android API 缺失、native 加固、超时、TLS、空播放字段、非法 JSON、
  缺方法实现和正常全链路；已在本机 10/10 通过，并接入 CI。
- 新增 `spider-tests/cases.json`：固定 15 个常见 `csp_*` 类（XYQ.jar）；新增
  `.github/workflows/spider-live.yml` 每周一定时跑真实矩阵并上传 artifact。
- 2026-11-20 本机首轮真实矩阵（关键词「电影」）：`usable=1`（Ikanbot）、
  `search_empty_category_ok=1`（New6v：分类 18 条、详情 1 线 1 集、播放返回
  解析页）、`empty_search=13`。源站波动大，这组数字只代表当次，不承诺当前可用性。

## 18. Spider 产品闭环（2026-11-20）

- `home.ts`：配置 `home.sections` 现在可以引用 Spider 站点；有 `typeId` 时走
  宿主 `category`，没有时走 `home`，结果转成统一 `Vod`。没有 CMS 源时，默认取
  前 8 个已启用 Spider 站点各给一屏，避免一次拉起几十个 JVM 站点。
- `category.ts`：分类树从 Spider `home.class` 归一化；分类浏览走宿主 `category`，
  同一站点的多个叶子 id 分别请求后交替合并。
- 影视源页：每个 Spider 显示缓存字节数和 raw jar 的 SHA256；支持单站/全部清理
  缓存、重新准备运行时；连通性测试失败增加缺类、Android API、native、超时、
  TLS、空播放字段等分类。
- 实测（隔离数据目录 + XYQ.jar + Ikanbot）：首页内容位 24 条、分类「电影/电影」
  24 条、缓存 1.30MB / SHA256 `ace8cb39…`、清理后 0 文件、重启宿主 107ms、
  缺类站点返回 `ClassNotFoundException` 且分类为 `missing_class`；177/177 自检通过。

## 19. Spider 运行时隔离与安全提示（2026-11-20）

- 首次启用单个 Spider 站点或批量启用时，确认框明确提示「运行配置来源的第三方
  jar，独立 JVM 不是安全沙箱」，并显示 jar 来源 host；影视源列表也显示来源与
  SHA256。
- `sanitizedEnv()` 在拉起 JVM 前过滤 token、secret、password、API key、
  cookie/session、常见云厂商/GitHub 凭据等环境变量，避免把宿主凭据暴露给 jar。
- JVM 的工作目录、`user.home` 与 `java.io.tmpdir` 都限制到
  `data/spider-work/`；不再以应用目录为 cwd。
- `SpiderHostClient` 连续 3 次调用超时会杀掉卡死的宿主，下一次调用由
  `ensureReady()` 自动重新拉起；影视源页提供「重新准备运行时」按钮。
- 实测：Ikanbot 首页 24 条、分类 24 条、缓存 1.30MB/SHA256、清理后 0 文件、
  重启 107ms、缺类分类 `missing_class`；`spider-work` 目录创建成功，177/177 自检通过。

## 20. 安装包瘦身（2026-11-20）

- 新增 `electron-builder.lite.yml` 与 `build:win:lite`：精简版不打包
  `spider-host`（内置 JRE 44.4MB、unidbg 33.3MB、dex2jar 19.7MB、依赖 13.2MB），
  只保留 Electron/前端产物，适合 CMS/网页规则/JSON 接口用户。
- 0.1.3 Release 实测：完整版 exe 212,870,248 字节、精简版 118,968,462 字节；
  完整 zip 261,977,971 字节、精简 zip 165,127,115 字节。
  完整版/精简版分别写 `SHA256SUMS.txt` / `SHA256SUMS-lite.txt`。
- 发布工作流会同时构建两版并上传到同一个 draft Release；精简版明确标注
  “不含 Spider 运行时”，不承诺自动补装。后续若要“按需下载运行时”，需要
  再做下载器/校验/权限流程。
