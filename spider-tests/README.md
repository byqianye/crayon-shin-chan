# Spider 真实性矩阵

这里回答的不是「搜索能不能返回」，而是「一个 `csp_*` 类从 home 到 player 到底哪一段真的可用」。

## 两层测试

| 层 | 入口 | 触发 | 是否阻断 |
| --- | --- | --- | --- |
| 契约矩阵 | `node scripts/spider-contract.mjs` | 每次 CI 的 `Spider 宿主编译` 任务 | 是 |
| 真实源矩阵 | `node scripts/spider-matrix.mjs --cases spider-tests/cases.json` | 每周定时任务 / 手动 | 否，只产出报告 |

契约矩阵用合成的 Spider 覆盖错误分类：缺类、不是 `Spider` 子类、Android API 缺失、native 加固、超时、TLS 证书失败、空播放字段、非法 JSON、缺方法实现和正常全链路。它不接触外网，分类必须稳定。

真实源矩阵固定 `spider-tests/cases.json` 里的 15 个常见 `csp_*` 类，每个类独立进程执行 `home → search → detail → player`。源站波动只会改变报告，不会阻塞每次提交。

## 分类口径

| 分类 | 含义 |
| --- | --- |
| `usable` | home / search / detail / player 全链路成功 |
| `category_only` | 搜索为空，但分类页能取到内容且详情/播放可用 |
| `search_empty_category_ok` | 搜索为空，分类页有内容，但详情或播放仍失败 |
| `missing_class` | jar 里没有配置指定的类 |
| `wrong_type` | 类存在但不是 `Spider` 子类 |
| `android_api` | `NoSuchMethodError` / `Stub!` 等 Android 兼容层缺失 |
| `native` | `UnsatisfiedLinkError` / `loadLibrary`，通常是 native 加固 |
| `timeout` | 单个动作超过用例超时 |
| `tls` | `SSLHandshakeException` / 证书 / TLS 失败 |
| `empty_play_fields` | 详情没有 `vod_play_from` / `vod_play_url` 或没有剧集 |
| `empty_search` | 搜索和分类都返回空列表 |
| `invalid_json` | Spider 返回的不是 JSON 对象 |
| `empty_player` | 有线路但 `player` 没返回播放地址 |
| `home_error` | 只有首页失败，搜索/详情/播放仍可用 |
| `search_error` / `detail_error` | 对应动作异常 |

## 本地运行

先准备 JDK 17、宿主依赖和宿主 class：

```powershell
$env:JAVA_HOME = "C:\path\to\jdk-17"
pnpm spider:deps
pnpm spider:build
node scripts/spider-contract.mjs
```

真实源矩阵（会访问外网，耗时取决于源站）：

```powershell
node scripts/spider-matrix.mjs --cases spider-tests/cases.json --out "$env:TEMP\spider-reports"
```

只想跑某几个类：

```powershell
node scripts/spider-matrix.mjs --only mp4mov,jianpian --keyword 庆余年
```

## 报告

每次运行生成 `spider-matrix.json` 和 `spider-matrix.md`。CI 把真实源报告作为 `spider-matrix` artifact 保存 14 天，不写进 Git。报告里的耗时、错误和可用性都来自当次真实执行，不要拿历史数字承诺当前可用性。
