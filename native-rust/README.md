# Rust 桌面实现

v0.1.7 将界面与业务迁移到 Rust / Dioxus，复用原版 CSS、Microsoft Edge WebView2、ArtPlayer/hls.js 与独立 Java Spider 宿主。第三方组件保留其原实现语言，原 src/ 用于功能、样式与数据契约对照。

## 模块

| 模块 | 职责 |
|---|---|
| storage / model | 原版 JSON、设置、收藏与观看历史；原子写入与坏文件保护 |
| config / sources | TVBox、多仓、图片包装与加密配置、手动站点、同步与忽略记录 |
| http / health | 连接复用、超时、取消、请求头与健康记录 |
| catalogue / cms / html_rules / category | 搜索、分类、详情、首页与缓存 |
| media / live / player / ai | 解析、媒体代理、直播、多线路、外部播放器与 AI 服务 |
| spider | jar/dex 缓存、Java JSON-RPC、并发、超时与宿主退出 |
| ui | Dioxus 界面、播放器桥接、抽帧和窗口生命周期 |

详情与分类缓存有容量和有效期限制，配置改变后失效。搜索使用有界并发及取消令牌；健康记录先进入内存再合并写盘。失败保持明确错误，不把超时或解析异常转成空结果。

## 加载优化（工作区实现，尚未发布）

首页原先按请求顺序收取并等待全部栏目，再一次性显示。当前实现改为有界并发、按完成顺序处理、逐栏目显示，最终保持配置顺序。单栏缓存与共享请求让切页返回和多个调用者复用已完成的内容；缺失源错误也随完整首页缓存保留。

分类目录逐站显示。点击已经出现的分类后，该站影片立即开始加载，其余站点的目录和影片后续补齐。目录成功缓存 10 分钟，有失败时缓存 1 分钟，翻页复用目录，配置 revision 改变后失效。分类内容逐叶返回，沿用最终站点顺序、交错合并、每站 30 条上限与最大页数；每次本页快照都与已完成的前几页合并，避免重复累加。

切换分类、配置变化或离开页面时取消旧加载，generation 检查继续防止旧结果覆盖新分类。最后一个等待者退出时释放共享请求，仍有等待者时保留合并。界面进度通道只保留最新快照，栏目使用稳定 key；海报保留懒加载并异步解码。并发上限和 Spider 请求超时保持原设置。

本机快慢源窗口验收已观察到：慢源仍在加载时，两个快源影片已显示并可进入分类；慢源成功后结果从 12 部补到 18 部，三页累计为 18、36、54 部，目录请求始终为 3 次。故意超时只影响对应栏目。此证据说明首批内容无需等待最慢源，不代表公网请求或整批完成时间均按固定比例加速。

## 构建与运行

在仓库根目录安装 pnpm 依赖，Rust 构建会嵌入既有媒体脚本、图标和样式。Windows 需要 Rust 与 C/C++ 工具链；GNU 使用 MinGW-w64（含 windres），MSVC 使用 Visual Studio C++ Build Tools 和 Windows SDK。WebView2 Runtime 是运行环境要求。

    cargo fmt --manifest-path native-rust/Cargo.toml -- --check
    cargo check --locked --manifest-path native-rust/Cargo.toml --all-targets
    cargo clippy --locked --manifest-path native-rust/Cargo.toml --all-targets -- -D warnings
    cargo test --locked --manifest-path native-rust/Cargo.toml --tests
    cargo test --locked --manifest-path native-rust/Cargo.toml --no-default-features
    pnpm build:rust

scripts/build.ps1 支持 -Profile、-Target、-Run 和 -DataDir，打印实际可执行文件路径，尊重 CARGO_TARGET_DIR。GNU 构建会从 Cargo 锁定的 WebView2 依赖中复制对应架构 WebView2Loader.dll 并核对 SHA256；MSVC 使用静态加载器。build.rs 为 exe 嵌入图标和版本资源。

启动时用 --data-dir 指向独立目录，也支持 CRAYON_DATA_DIR。默认数据位于程序同级 data/；验收不得直接使用已安装版数据。Spider 优先寻找程序同级 spider-host/ 或 resources/spider-host/，也可用 SPIDER_HOST_ROOT 指定。

## Windows 发布包

    pnpm package:rust
    node scripts/release-check.mjs --rust --version 0.1.7 --write-checksums
    node scripts/release-notes.mjs --rust --version 0.1.7 --write

打包前必须备齐 spider-host 的 out、lib、runtime、dex-tools、unidbg-libs 与 unidbg-out；构建方法见根 README。NSIS 可通过 PATH、MAKENSIS_PATH 或现有 electron-builder 缓存定位，不自动下载来历不明的工具。

产物位于 release/rust/：安装器、免安装 ZIP、SHA256SUMS-rust.txt 和发布说明。包内包含 Rust exe、GNU 所需加载器、图标、完整 Spider 运行时、第三方许可、version.json 与逐文件 FILES.sha256，不包含 data/ 或 Electron。

安装器默认使用独立的当前用户目录 crayon-shin-chan-rust。安装与卸载保留 data/；旧 Electron 安装和数据可用于回退。首次迁移须退出两版并复制旧 data/，不要让两版同时写同一目录。

## 验证记录与边界（2026-11-25）

Rust 48 项契约测试在开启 desktop 和关闭默认功能两种配置下通过；check、严格 Clippy、fmt 与 release 构建通过。原 TypeScript 服务 26 项回归、两套类型检查、生产构建和 Electron 窗口 185 项断言也通过，后者属于原版对照证据。

真实 Rust 桌面使用隔离数据与本机 HTTP 服务完成：首次使用、大写 HTTP 地址导入、同步、搜索合并、详情、收藏与历史、分类三页、站点测试/审计、MP4 播放、切集、坏线报错和恢复、AI 资料总结与三帧画面总结、直播坏线自动切换、重启恢复、设置保存及退出保存。

修复并复验的桌面问题：播放器侧栏海报恢复 58px；影视源筛选栏正确排布；同步提示切页后自动消失；正常关闭时写入最后进度并让主进程和对应 WebView 子进程退出。请求超时 23 秒在重启后正确恢复。

NSIS 安装器已在包含中文与空格的隔离目录完成静默安装、426 个文件逐项 SHA256 校验、重复安装保留 data，以及卸载移除程序/注册信息并保留 data。ZIP 打包后独立检查版本、Spider 目录、逐文件清单和用户数据/Electron 排除。

NSIS 脚本明确使用 /INPUTCHARSET UTF8，避免旧编译器按本机代码页解读中文而生成乱码快捷方式。编译使用 /WX 拒绝警告；随包保留 NSIS 自带许可文本。

最终正式安装包完成 427 个文件的逐项 SHA256 校验；原版 8 个 JSON 文件与预设、Spider 缓存已迁移并核对哈希。中文卸载名称、桌面和开始菜单快捷方式均正确；正式窗口显示 v0.1.7、Rust / Dioxus / WebView2 和新数据目录，原配置与观看记录已恢复。旧 Electron 安装及数据保留作回退。

未完成全公网源矩阵、全页像素对照、实际外部播放器播放与 AI 取消按钮的桌面交互复验。取消、抽帧恢复与历史隔离有代码契约覆盖，但不将其当成全部 UI 交互认证。直播 MP4 的健康探测仍沿用原版仅识别 HLS 列表的行为，探测结果不能代替真实播放结果。

## 固定样本性能对照

scripts/benchmark.mjs 从原版真实 TypeScript 服务生成临时 Node 对照程序，与 Rust release 核心服务请求同一个本机 HTTP 桩。使用新数据目录，不读取用户设置或 API Key；结束后删除临时程序与数据。

迁移阶段样本：32 站、并发 8、每请求延迟 25ms、每站返回 100 条并取 30 条；运行 10 轮，剔除首轮后取 9 轮中位数。比较 960 条搜索结果与 1200 条去重结果，另有 5 组去重逐字段对照。

| 指标 | 原版 Node | Rust release 核心服务 |
|---|---:|---:|
| 搜索首条结果 | 41.32ms | 37.36ms |
| 搜索总耗时 | 155.11ms | 151.53ms |
| 去重：未知年份 | 6.79ms | 5.42ms |
| 去重：已知年份 | 4.56ms | 4.85ms |

这组历史样本仅代表固定本机服务场景，不代表所有场景均加速；已知年份去重略慢。发布阶段未重新测量整体启动、内存或真实源性能，也不承诺所有远程 Spider 持续可用。
