# 蜡笔小新

<img src="build/icon.png" alt="蜡笔小新应用图标" width="128" height="128" />

Windows 影视聚合客户端，支持 TVBox 配置订阅。v0.1.7 起，桌面界面与业务由 **Rust / Dioxus** 实现，保留原版样式与功能流程，通过 WebView2、ArtPlayer/hls.js 和 Java Spider 运行第三方组件。

项目源自 [hgtuj5688/tvbox-desktop](https://github.com/hgtuj5688/tvbox-desktop)（MIT）。名称仅作项目代号，与《蜡笔小新》版权方无关。软件不提供、不托管任何影视内容，请自行使用合法、可信的配置源。

| 项目 | 当前版本 |
|---|---|
| 版本 | **0.1.9 · Rust 桌面版**，正式下载以 [Release](https://github.com/byqianye/crayon-shin-chan/releases/tag/v0.1.9) 为准 |
| 平台 | Windows x64，需要 Microsoft Edge WebView2 Runtime |
| 界面与业务 | Rust、Dioxus；复用原版 CSS |
| 媒体与 Spider | ArtPlayer、hls.js、mpegts.js、Shaka；随包提供 Java 宿主、JRE、dex2jar 与 unidbg 桥 |
| 数据 | 默认保存在程序同级的 data/，兼容原版 JSON 数据格式 |
| 发布包 | Rust 安装包、免安装 ZIP、SHA256 校验清单；不包含 Electron |

![原版界面参考](shots/home.png)

上图是原版历史截图，展示样式与功能布局，不代表当前源站可用性。迁移说明与实测边界见 [Rust 文档](native-rust/README.md)，完整历史见 [CHANGELOG](CHANGELOG.md)，配置规范见 [功能框架](功能框架.txt)。

v0.1.9 加入媒体诊断、mpv 进度与续播联动、FLV/TS/DASH 播放适配。继续保留 v0.1.8 的首页与分类逐步加载；实现说明与本机验收边界见 [Rust 文档](native-rust/README.md)。

## 下载安装

从 [Releases](https://github.com/byqianye/crayon-shin-chan/releases) 下载同一版本的文件：

| 文件 | 用途 |
|---|---|
| CrayonShinChan-0.1.9-rust-x64.exe | NSIS 安装版，创建桌面和开始菜单快捷方式，提供卸载入口 |
| CrayonShinChan-0.1.9-rust-x64.zip | 免安装版，解压后运行 crayon-rust.exe |
| SHA256SUMS-rust.txt | 安装包与 ZIP 的 SHA256 校验值 |

默认安装目录为 %LOCALAPPDATA%\Programs\crayon-shin-chan-rust。安装向导允许更改位置；请选择当前用户可写的目录。运行时不需要 Rust、Node.js 或另外安装 Java；Microsoft Edge WebView2 Runtime 由系统提供，缺失时请从 [Microsoft 官网](https://developer.microsoft.com/microsoft-edge/webview2/) 安装。

GNU 构建必须让 WebView2Loader.dll 与 crayon-rust.exe 保持同级。免安装包请完整解压，保留 spider-host/、licenses/ 等目录。安装包中的 version.json 标识实际版本与运行环境，FILES.sha256 可核对解包后的文件。

程序尚未进行代码签名，Windows 可能显示发布者未知提示。v0.1.9 只发布 Rust 完整版；旧 Electron 完整版和精简版仍可从对应历史 Release 下载。

## 从 Electron 版迁移

1. 退出旧版和 Rust 版，备份旧程序目录中的 data/。
2. 安装 Rust 版到新目录，或将免安装包解压到新目录。
3. 把旧 data/ 复制到新程序旁。配置源、站点覆盖、设置、收藏、观看历史与忽略记录使用兼容的 JSON 格式；预设本地文件也应一并复制。已有 Rust 数据时先备份，避免覆盖较新的记录。
4. 启动 crayon-rust.exe，核对配置源与观看记录。桌面快捷方式应指向该程序。

安装器升级时保留 data/，卸载也保留 data/，便于重新安装。旧 Electron 安装器仍沿用其历史卸载行为；需要回退时，先备份数据，再运行旧版。两版请避免同时读写同一份数据。

开发和验收可通过 --data-dir 指定独立目录，也支持 CRAYON_DATA_DIR。数据目录可在「设置 → 关于」查看。

## 主要功能与本版修复

- 配置订阅、本地 JSON、手动站点、多仓导入、同步、启停、筛选、测试、审计和忽略记录。
- CMS XML/JSON、网页规则、HTTP JSON 与 Java Spider；订阅中相对地址、请求头规则、解析接口和首页内容位。
- 流式聚合搜索、有界并发、请求取消、影片合并、解说过滤、分类浏览与分页。
- 详情、选集、多线路、网页播放地址提取、解析接口排行、内嵌播放与外部播放器检测。
- 直播 M3U/TXT、多线路合并与失败换线；收藏、观看历史、首页继续观看。
- AI 资料总结与画面总结。用户点击后才调用设置中的服务；画面总结发送抽取帧，不包含音轨。API Key 只保存在本机设置中。
- 修复续播、暂停和离开页面时的最终进度保存、同地址多集切换，以及抽帧期间的历史隔离。
- 修复全局请求超时继承、大写 HTTP/HTTPS 地址、播放器侧栏海报尺寸、站点筛选栏和跨页提示定时清除。
- 关闭窗口前保存播放进度，保存结束后退出进程。

首次启动会添加内置公开采集订阅和肥猫订阅并尝试同步。同步失败保留上次缓存；删除或停用后不自动恢复。第三方 Spider 默认关闭，需确认来源后在「影视源」页启用。包中不包含用户的设置、API Key、收藏或观看记录。

本版包含按需媒体诊断、mpv 进度与续播联动、FLV/TS/DASH 适配。媒体探测成功表示识别和连通性，实际播放仍取决于编码与 WebView2 能力；能力与验收边界见 [Rust 文档](native-rust/README.md#媒体诊断与播放联动v019)。

## Spider 与支持边界

带 spider 文件的 csp_* jar/dex 站点通过随包 Java 宿主运行。宿主支持首页、分类、搜索、详情、播放、取消与重启，影视源页提供运行时检查、缓存清理和错误分类。

Spider jar 是第三方可执行代码。JVM 会过滤敏感环境变量并使用独立工作目录，但独立 JVM 不是安全沙箱，只应启用可信来源。

- Python/JavaScript Spider、依赖浏览器 WebView 的实现仍可能不支持。
- DexNative 加固、Pan.init 递归、源站超时、TLS、空播放字段或媒体断连可能导致站点失败。宿主支持不代表所有站点可播放。
- 解析接口支持 HTTP 解析（type=1/2）；type=0 浏览器嗅探未实现。
- 外部播放器检测只检查常用路径、PATH 与注册表；自定义安装位置可能需要手填。
- 真实桌面验收覆盖本机 HTTP 桩的导入、搜索、详情、分类、播放、切集切线、直播换线、AI 资料/画面总结、收藏历史、重启与关闭。未完成全公网源矩阵、全页像素对照或全部外部播放器联调；AI 取消按钮尚未完成桌面交互复验。
- Rust 迁移不等于整个应用更快。固定样本的服务性能对照见 [Rust 文档](native-rust/README.md)；没有整体启动、内存或真实源性能结论。

## 开发与构建

需要 PowerShell 7、pnpm 11.25.0、Rust 工具链和 Windows C/C++ 链接工具。MSVC 需 Visual Studio C++ Build Tools / Windows SDK；GNU 需 MinGW-w64（含 windres）。构建 Java 宿主需 JDK 17，制作安装包需 NSIS，可通过 MAKENSIS_PATH 指定 makensis.exe。

首次准备：

    pnpm install --frozen-lockfile
    pnpm spider:deps
    pnpm spider:build
    pnpm spider:unidbg
    pnpm spider:runtime

| 命令 | 用途 |
|---|---|
| pnpm dev | 编译并启动 Rust 桌面版 |
| pnpm build:rust | 编译 Rust release，GNU 版同时复制 WebView2 加载器 |
| pnpm build:win / pnpm package:rust | 生成 Rust 完整版安装包、ZIP、许可与 SHA256 清单 |
| cargo test --locked --manifest-path native-rust/Cargo.toml --tests | Rust 契约回归，包含真实 Java 宿主 |
| node scripts/player-bridge-contract.mjs | HLS、FLV/TS、DASH 适配与资源释放契约 |
| cargo test --locked --manifest-path native-rust/Cargo.toml --no-default-features | 无桌面功能的核心服务回归 |
| cargo clippy --locked --manifest-path native-rust/Cargo.toml --all-targets -- -D warnings | 严格 Rust 静态检查 |
| node scripts/service-regression.mjs | 原 TypeScript 服务对照回归 |
| pnpm typecheck / pnpm build | 原 Electron 实现的类型检查与生产构建 |
| node scripts/ci-selftest.mjs | 原 Electron 窗口对照自检，使用隔离数据 |
| pnpm dev:electron | 启动原 Electron 开发版 |
| pnpm build:win:electron / pnpm build:win:lite | 旧 Electron 打包命令，非 v0.1.9 发布入口 |
| pnpm icon:check | 检查应用图标同步 |

Rust 包生成在 release/rust/。原 src/ 保留作功能、样式与数据契约对照；原版测试通过不等于 Rust 窗口测试通过。构建输出与依赖缓存不提交到 Git。

## CI 与发布

CI 保留原 TypeScript / Electron 对照检查，并增加 Rust fmt、check、严格 Clippy、两种功能配置的测试与 release 构建。发布工作流检查版本、图标、服务回归、Spider 契约、Electron 对照窗口及 Rust 实现，然后仅上传 Rust 包和对应 SHA256 清单。

v* 标签触发的自动流程只创建草稿，拒绝覆盖已有正式 Release；确认构建和安装验收后再发布。安装包和 ZIP 是 Release 资产，不提交进 Git。

## 目录与许可

| 路径 | 内容 |
|---|---|
| native-rust/src/ | Rust 业务、Dioxus 界面与媒体桥接 |
| native-rust/tests/ | 配置、存储、搜索、媒体、播放、直播、AI、Spider 契约 |
| native-rust/scripts/ | 构建、NSIS 打包和服务性能对照 |
| src/ | 原 Electron / React / TypeScript 实现与共用样式 |
| spider-host/ | Java Spider 宿主及运行时构建脚本 |
| scripts/ | 原版对照回归、CI 与发布校验 |

项目使用 [MIT](LICENSE)。发布包包含第三方声明与许可文件；Java 运行时许可在 spider-host/runtime/legal/，其余组件见 THIRD-PARTY-NOTICES.txt 和 licenses/。用户自行添加的第三方配置与 Spider 不属于本项目发布资产。
