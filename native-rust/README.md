# Rust 桌面实现

v0.1.10 延续 Rust / Dioxus 桌面实现，复用原版 CSS、Microsoft Edge WebView2、ArtPlayer/hls.js 与独立 Java Spider 宿主。旧 `src/` 保留为 Electron 对照实现；Rust 发布包不包含 Electron。

本文说明 Windows 安装、数据迁移、开发构建及已验证边界。项目不提供或托管影视内容，请只配置合法、可信且获授权的来源。

## 安装与迁移

Rust 桌面发布提供当前用户安装版和免安装 ZIP。默认安装目录为 `%LOCALAPPDATA%\Programs\crayon-shin-chan-rust`；安装器允许改为当前用户可写目录。首次运行要求系统已安装 Microsoft Edge WebView2 Runtime。Windows 安装包随附 Spider 宿主、精简 JRE、转换工具及许可文件，普通使用者不必另装 Rust、Node.js 或 Java。程序未进行代码签名，Windows SmartScreen 可能显示发布者未知。

默认数据目录为程序可执行文件同级的 `data/`。安装包不携带用户配置、API Key、收藏或观看历史；安装升级与卸载都会保留 `data/`。免安装版需完整解压并保留 `spider-host/`、`licenses/` 等目录。GNU 构建生成的 `WebView2Loader.dll` 必须与程序 exe 同级。

从旧 Electron 版迁移时，先退出两个版本并备份旧程序目录的 `data/`，再把数据复制到 Rust 程序旁的 `data/`。Rust 版读取兼容的 JSON 设置、订阅、收藏、观看历史和忽略记录；如目标目录已有 Rust 数据，先单独备份并手动合并，避免用旧副本覆盖较新状态。不要让两个版本同时写同一数据目录。

开发或隔离验收可通过命令行 `--data-dir <目录>` 指定目录，也可设置 `CRAYON_DATA_DIR`；命令行参数优先。安装版 Spider 默认从 exe 同级查找 `spider-host/`，开发时可用 `SPIDER_HOST_ROOT` 指定宿主目录。

## 功能与模块

| 模块 | 职责 |
|---|---|
| storage / model | 原版 JSON、设置、收藏与观看历史；原子写入与坏文件保护 |
| config / sources | TVBox、多仓、图片包装与加密配置、手动站点、同步与忽略记录 |
| http / health | 连接复用、超时、取消、请求头与健康记录 |
| catalogue / cms / html_rules / category | 搜索、分类、详情、首页与缓存 |
| websites | 看看鸭与 Animoe 原生网站适配、分集与按需媒体解析 |
| media / media_probe / live | 解析、有界类型识别与诊断、媒体代理、直播和多线路 |
| player / mpv / playback / ai | 外部播放器、mpv IPC、观看进度、续播与 AI 服务 |
| spider | jar/dex 缓存、Java JSON-RPC、并发、超时与宿主退出 |
| ui | Dioxus 界面、播放器桥接、抽帧和窗口生命周期 |

详情与分类缓存有容量和有效期限制，配置改变后失效。搜索使用有界并发及取消令牌；健康记录先进入内存再合并写盘。失败保持明确错误，不把超时或解析异常转成空结果。

## 媒体诊断与播放联动（v0.1.9）

播放与直播页可按需检查地址。浅探测仅请求前 4KiB，限制总时长并支持取消；以内容特征优先，再看响应类型和地址后缀。报告显示识别依据、HTTP 状态、采样量和分类错误，不返回原始地址、请求头或响应正文。decodeVerified 始终为 false；探测成功只表示连通与识别，解码能力须看真实播放。

内置播放器保留 ArtPlayer / hls.js，用 mpegts.js 转封装 FLV / MPEG-TS，用 Shaka 加载 DASH；不做本地转码。所有 DASH 网络请求经过本机代理，利用最终响应地址处理清单重定向、BaseURL 和 SegmentTemplate，保留 Range。跨域跳转按新域重新取授权头。支持范围仍取决于 WebView2 的音视频编码与 MSE 能力；受保护内容授权不在本轮范围内。

选择真正的 mpv.exe / mpv.com 时启用 Windows 命名管道 IPC。进度、暂停和结束复用 Playback 的保存与续播规则；切外部前保存内置最终位置，返回内置前等待 mpv 最终快照与进程回收。页面离开、窗口退出与启动取消都绑定会话，旧事件不覆盖新集。直播不写入点播历史；其他外部播放器（含 mpv.net 包装程序）保留独立启动，不回传进度。

本轮按 [Awesome 媒体目录](https://github.com/sindresorhus/awesome#media) 与 [Awesome Broadcasting](https://github.com/ebu/awesome-broadcasting) 筛选后，暂不把额外媒体工具放进播放主链。后续若确有编码排障需求，可选 [FFprobe](https://ffmpeg.org/ffprobe.html) 输出轨道 JSON，或用 [MediaInfo](https://github.com/MediaArea/MediaInfoLib) 提供技术元数据，先选其一；[Bento4](https://github.com/axiomatic-systems/Bento4) 适合开发用的 MP4/DASH 结构检查和样本制作。元数据工具不代替浏览器解码验证。现有缓存已有有效期、容量和共享请求，本轮不替换。

## Spider 转换缓存恢复（2026-11-27）

dex2jar 在转换开始时就可能生成 22 字节的空 ZIP，不能以文件非空或进程成功退出判断缓存可用。Rust 加载器读取 ZIP 目录，检查目标 Spider 类条目；空包或缺少目标类的转换结果不会交给 Java 宿主。原始 DEX 缓存仍有效时直接本地重转，来源地址已失效也可恢复。

转换写入独占临时 JAR，通过校验后才改名为正式缓存；取消、超时及请求 future 被丢弃时回收转换进程并清理临时输出。回归测试使用临时编译的真实 Java Spider、真实 DEX 与合法但零类的 DEX，覆盖调用、本地恢复和空输出拒绝；不能只用宿主 ping 代替类加载验证。

本次缓存修复验收（2026-11-27）：desktop 测试 95 项通过，关闭默认功能时 94 项通过，两者均为 0 失败、1 项真实 mpv 测试默认忽略。fmt、all-targets check、严格 Clippy 和 release 桌面构建通过。另覆盖共享 JAR 已被 Java 打开时缺类请求不破坏原缓存，以及取消、超时、future/cleanup 被丢弃和 runtime 关闭后的进程、临时文件回收。

隔离 Rust / WebView2 窗口以安装版的 22 字节坏缓存启动，自动恢复出 1,822,085 字节、957 个条目的 JAR，包含 AppRJ.class，未遗留 partial.jar。AppRJ 搜索“庆余年”返回 2 部，详情返回 3 条线路、138 集；线路四实际播放至 34.89 秒，1280×720、875 帧且无视频错误。线路三媒体诊断返回 HTTP 404，不能把类加载修复等同于所有上游线路可播放；本次未验证全公网片源或扬声器输出。

经用户授权，同版本修复 EXE 已替换到原安装目录并重启，1,378 项安装清单哈希全部匹配，替换前后 9 个数据 JSON 哈希一致。实际安装窗口的 22 字节坏缓存自动恢复为 1,822,082 字节、957 个条目的 JAR，包含 AppRJ.class，未遗留 partial.jar；搜索和详情再次确认返回 2 部、3 条线路和 138 集。安装后复验未播放影片，历史与设置文件哈希保持一致；启动自动刷新产生的运行数据变化另计。旧 EXE 和原文件清单保留在同级带时间戳的回滚备份目录。

## 本轮验证（2026-11-27，v0.1.9）

本次提交范围内，desktop 配置与关闭默认功能时均有 83 项 Rust 测试通过；两者各有 1 项需显式提供真实 mpv 的测试默认忽略。mpv 专项另行使用真实程序运行 11 项契约，全部通过，实测保存 4 秒后续播至 4.04 秒，切集从 1 秒开始，所启动进程均回收。播放器 JavaScript 桥接 10 项契约、语法检查、fmt、严格 Clippy 与 debug 桌面构建通过。

真实 Rust / WebView2 窗口使用隔离数据和本机生成的 H.264/AAC 样本，MP4、HLS、FLV、MPEG-TS、DASH 均观察到实际解码出帧。无后缀 FLV 和经 302 跳转的无后缀 DASH 各解码 300 帧；DASH 从结尾跳回 3.99 秒后重新出帧，诊断正确显示 MPEG-DASH、HTTP 206 和实际采样量。内置切 mpv、进度写盘及返回内置续播完成实测；另一轮在 mpv 播放时关闭主窗口，最后 42 秒进度落盘，主进程、对应 mpv 与调试端口均退出，stderr 为空。mpv 验收使用 null 音视频输出，验证 IPC、时间与会话回收，未验证其可见窗口和扬声器输出。

未覆盖全公网片源、全部编码、DRM 授权和完整长时间直播。上述记录为开发验收；安装包另由 verify-package.ps1 核验版本、文件清单、Spider 运行时与第三方许可。共用 CARGO_TARGET_DIR 时，无桌面测试会覆盖 debug 同名可执行文件；启动 UI 验收前必须最后重建 desktop，避免误用无桌面产物。

## 首页与分类加载优化

首页原先按请求顺序收取并等待全部栏目，再一次性显示。当前实现改为有界并发、按完成顺序处理、逐栏目显示，最终保持配置顺序。单栏缓存与共享请求让切页返回和多个调用者复用已完成的内容；缺失源错误也随完整首页缓存保留。

分类目录逐站显示。点击已经出现的分类后，该站影片立即开始加载，其余站点的目录和影片后续补齐。目录成功缓存 10 分钟，有失败时缓存 1 分钟，翻页复用目录，配置 revision 改变后失效。分类内容逐叶返回，沿用最终站点顺序、交错合并、每站 30 条上限与最大页数；每次本页快照都与已完成的前几页合并，避免重复累加。

切换分类、配置变化或离开页面时取消旧加载，generation 检查继续防止旧结果覆盖新分类。最后一个等待者退出时释放共享请求，仍有等待者时保留合并。界面进度通道只保留最新快照，栏目使用稳定 key；海报保留懒加载并异步解码。并发上限和 Spider 请求超时保持原设置。

本机快慢源窗口验收已观察到：慢源仍在加载时，两个快源影片已显示并可进入分类；慢源成功后结果从 12 部补到 18 部，三页累计为 18、36、54 部，目录请求始终为 3 次。故意超时只影响对应栏目。此证据说明首批内容无需等待最慢源，不代表公网请求或整批完成时间均按固定比例加速。

## 网站源适配（开发记录，2026-11-28）

在设置的内置推荐订阅中添加“看看鸭与 Animoe”，或导入 samples/website-sources.json；两站以 type=4 根域 API 路由到 Rust 适配器，沿用 Catalogue 和 Media，无需 Spider。分类、详情、首页缓存沿用配置 revision 隔离；看看鸭另有有界的短期页面与前台检查会话缓存。站点请求支持取消与整体超时，播放按需解析并最多采样 4KiB 内容。

站点与播放页必须同源，外域分集不会进入通用嗅探。媒体和重定向按最终目的域计算请求头，站点 Cookie、Authorization 不授予外域 CDN；目的域配置的独立授权仍保留。网站适配器返回的媒体请求头是最终注册依据，避免公共播放器重新并入站点凭据。

离线网站与接入契约共 27 项，覆盖两种播放地址编码、前台检查复用、分类与搜索、详情缓存、窗口分页、取消与响应体超时、伪装媒体错误页及跨域凭据边界。公网检查默认忽略，需要显式运行；不得将 Animoe 的 blocked 输出计作真实播放通过。

2026-11-28 开发阶段曾完成 desktop 配置下选定的 lib、网站、Catalogue、Media、媒体探测与播放回归共 76 项，以及关闭默认功能的网站契约 27 项；该快照早于最后的缓存边界与重定向修复，不能代替发布验收。

发布前本机检查：全仓 Rust 源码与测试的 rustfmt、`cargo metadata --locked`、TypeScript 类型、图标、锁文件与 CI 约束检查通过；Electron 参考构建、自检 185/185、服务回归及 Spider 契约 10/10 通过。Spider 宿主由项目脚本和 JDK 17 生成。本机 Rust 契约编译在依赖阶段因缺少 MSVC `link.exe` 失败，测试未执行；最终 Rust 编译、Clippy、完整契约和安装包校验由 Windows Release 工作流执行，以对应提交的成功记录为准。

```powershell
cargo test --locked --manifest-path native-rust/Cargo.toml --no-default-features --test animoe_contract --test kankanya_contract --test website_sources_contract --test website_dispatch_contract
cargo test --locked --manifest-path native-rust/Cargo.toml --no-default-features --test website_live_contract kankanya_live_recent -- --ignored --nocapture
cargo test --locked --manifest-path native-rust/Cargo.toml --no-default-features --test website_live_contract animoe_live -- --ignored --nocapture
```

公网样本：看看鸭首页 24 部、4 类；搜索“庆余年”2 部，详情8条线路、273个分集条目，搜索0.23–0.55秒、详情0.28–0.33秒、缓存命中约2毫秒。最近更新作品第一线路为HLS，解析1.99秒、独立媒体复查0.92秒，内容签名通过；未验证播放器出画。庆余年前四条线路返回403/404，观察到源站API返回已过期约16分钟的媒体签名，不应将上游失效当成解码成功。Animoe实时被Cloudflare拦截，`enc` 非标准清单仍明确报不支持，未完成真实播放验收。分页沿真实链接最多跳16次，超限明确报错。

## 构建与运行

在仓库根目录安装 pnpm 依赖，Rust 构建会嵌入既有媒体脚本、图标和样式。Windows 需要 Rust 与 C/C++ 工具链；GNU 使用 MinGW-w64（含 windres），MSVC 使用 Visual Studio C++ Build Tools 和 Windows SDK。WebView2 Runtime 是运行环境要求。

```powershell
cargo fmt --manifest-path native-rust/Cargo.toml -- --check
cargo check --locked --manifest-path native-rust/Cargo.toml --all-targets
cargo clippy --locked --manifest-path native-rust/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path native-rust/Cargo.toml --tests
cargo test --locked --manifest-path native-rust/Cargo.toml --no-default-features
pnpm build:rust
```

scripts/build.ps1 支持 -Profile、-Target、-Run 和 -DataDir，打印实际可执行文件路径，尊重 CARGO_TARGET_DIR。GNU 构建会从 Cargo 锁定的 WebView2 依赖中复制对应架构 WebView2Loader.dll 并核对 SHA256；MSVC 使用静态加载器。build.rs 为 exe 嵌入图标和版本资源。

启动时用 --data-dir 指向独立目录，也支持 CRAYON_DATA_DIR。默认数据位于程序同级 data/；验收不得直接使用已安装版数据。Spider 优先寻找程序同级 spider-host/ 或 resources/spider-host/，也可用 SPIDER_HOST_ROOT 指定。

## Windows 发布包

```powershell
pnpm package:rust
node scripts/release-check.mjs --rust --version 0.1.10 --write-checksums
node scripts/release-notes.mjs --rust --version 0.1.10 --write
```

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

当时未完成全公网源矩阵、全页像素对照、实际外部播放器播放与 AI 取消按钮的桌面交互复验。取消、抽帧恢复与历史隔离有代码契约覆盖，但不将其当成全部 UI 交互认证。当时直播 MP4 健康探测仅识别 HLS；此限制已在上述未发布改动中修复，健康结果仍不能代替真实播放。

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
