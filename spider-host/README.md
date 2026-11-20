# Spider Host

TVBox `csp_*` 站点的桌面宿主。它把 Android/Dalvik 的 spider jar 转成 JVM class，
在一个独立 JVM 里执行，Electron 主进程只通过 stdin/stdout 的 JSON-RPC 调用。

## 运行方式

- 一个进程承载多个站点，首次调用 `open(site, jar, class, ext)` 时加载对应类。
- `home / category / detail / search / player / action` 都带 `site` 字段路由。
- 空闲 5 分钟由主进程 `stopSpiderHost()` 关闭；进程退出时也会清理。
- JVM 默认 `-Xms16m -Xmx192m -XX:+UseSerialGC`，实测 RSS 约 65MB。

## 构建

需要 JDK 17（`JAVA_HOME`）：

```powershell
pnpm run spider:deps
pnpm run spider:build
pnpm run spider:runtime
```

`spider:build` 只编译宿主；`spider:runtime` 用 jlink 生成约 45MB 的精简 JRE。
运行 `spider:smoke` 可以用真实 spider 验证 search → detail → player（需要
`SPIDER_JAVA`、`SPIDER_JAR`）。

FishGuard 的加密 `ext` 由独立的 `unidbg-out/FishGuardBridge` 一次性解密：

```powershell
pnpm run spider:unidbg   # 编译 unidbg 桥
```

`unidbg-libs` 约 35MB；桥进程只在首次解密时启动，解出结果后写入
`data/spider-cache/*.ext.txt`，后续不再启动。

## 已验证范围

- 荐片 `csp_Jianpian`：搜索、详情 29 条线路、播放直链。
- 金牌 `csp_Jpys`：搜索、详情、播放直链。
- 摸鱼 `csp_FishCloud`：首页 7 分类 / 251 条、搜索 4 条（轻量兼容路径）。
- 摸鱼 `csp_FishHxq`：unidbg 桥成功把加密 ext 解成 API 地址并缓存。
- 普通 `classes.dex` / `spider.jar`；`assets/` 里的加固 so（FishGuard 等）暂未接入
  unidbg，遇到会明确报错，不会静默返回空列表。

## 协议

每行一个 JSON：

```json
{"id":1,"method":"search","params":{"site":"csp_Jianpian","key":"庆余年","quick":false}}
{"id":1,"ok":true,"data":{"list":[]}}
```

spider 自己的 `System.out` 调试输出已在宿主启动时改道 stderr，不会污染协议。

## 实现边界

宿主按 TVBox/CatVod 的公开 Spider 方法签名独立实现，只使用 Apache-2.0 的
dex2jar / OkHttp / Gson 与 OpenJDK 运行时；没有复制 MisaTV、修狗TV 等
GPL/AGPL 项目的代码。加固壳（FishGuard / DexNative）需要另行接 unidbg。

Android 兼容层包含 `SharedPreferences`、`AtomicFile`、`Base64`、`Log`、
`ContentResolver`、`Build` 等常用 API；这些不是 android.jar 的 Stub 版本。
