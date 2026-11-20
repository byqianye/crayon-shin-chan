package host;

import android.app.Application;
import com.github.catvod.crawler.Spider;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.google.gson.JsonPrimitive;

import java.io.BufferedReader;
import java.io.File;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.io.PrintStream;
import java.lang.reflect.InvocationTargetException;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/**
 * 独立 Spider 宿主。协议：stdin/stdout 每行一个 JSON-RPC 请求/响应。
 *
 * 一个进程可以挂多个站点实例，所有站点共用一个 JVM，避免每个站起一个进程。
 * 首次使用某个站点时，主进程发 open（site/jar/class/ext），之后所有调用
 * 都带 site 字段路由到对应实例。
 */
public final class SpiderHost {
    private static final Gson GSON = new GsonBuilder().disableHtmlEscaping().create();
    private final PrintWriter out;
    private final Map<String, URLClassLoader> loaders = new ConcurrentHashMap<>();
    private final Map<String, Spider> spiders = new ConcurrentHashMap<>();
    private final Map<String, Object> locks = new ConcurrentHashMap<>();
    private final ExecutorService workers = Executors.newFixedThreadPool(4);

    public static void main(String[] args) {
        String jar = null;
        String className = null;
        String ext = "";
        for (int i = 0; i < args.length; i++) {
            switch (args[i]) {
                case "--jar":
                    jar = args[++i];
                    break;
                case "--class":
                    className = args[++i];
                    break;
                case "--ext":
                    ext = args[++i];
                    break;
                default:
                    break;
            }
        }
        try {
            // 协议通道固定用原始 stdout；spider 自己的 System.out 调试输出
            // 全部改道 stderr，避免把 JSON-RPC 行冲乱。
            PrintStream protocolOut = System.out;
            System.setOut(new PrintStream(System.err, true, StandardCharsets.UTF_8));
            SpiderHost host = new SpiderHost(protocolOut);
            if (jar != null && className != null) host.open("default", jar, jar, className, ext);
            host.run();
        } catch (Throwable error) {
            error.printStackTrace(System.err);
            System.exit(1);
        }
    }

    private SpiderHost(PrintStream protocolOut) {
        this.out = new PrintWriter(protocolOut, true, StandardCharsets.UTF_8);
    }

    private void run() throws Exception {
        JsonObject ready = new JsonObject();
        ready.addProperty("event", "ready");
        ready.addProperty("pid", ProcessHandle.current().pid());
        ready.addProperty("sites", spiders.size());
        out.println(GSON.toJson(ready));

        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        String line;
        while ((line = in.readLine()) != null) {
            line = line.trim();
            if (line.isEmpty()) continue;
            String request = line;
            workers.submit(() -> handle(request));
        }
        workers.shutdown();
        workers.awaitTermination(10, TimeUnit.SECONDS);
    }

    private void handle(String line) {
        JsonObject response = new JsonObject();
        JsonElement id = JsonNull.INSTANCE;
        try {
            JsonObject request = JsonParser.parseString(line).getAsJsonObject();
            id = request.has("id") ? request.get("id") : JsonNull.INSTANCE;
            response.add("id", id);
            String method = request.has("method") ? request.get("method").getAsString() : "";
            JsonObject params = request.has("params") && request.get("params").isJsonObject()
                    ? request.getAsJsonObject("params") : new JsonObject();
            response.addProperty("ok", true);
            response.add("data", dispatch(method, params));
        } catch (Throwable error) {
            Throwable cause = error instanceof InvocationTargetException && error.getCause() != null
                    ? error.getCause() : error;
            response.add("id", id);
            response.addProperty("ok", false);
            response.addProperty("error", cause.getClass().getSimpleName() + ": " + String.valueOf(cause.getMessage()));
            cause.printStackTrace(System.err);
        }
        synchronized (out) {
            out.println(GSON.toJson(response));
        }
    }

    private JsonElement dispatch(String method, JsonObject params) throws Exception {
        if ("open".equals(method)) {
            return open(
                    stringParam(params, "site", "default"),
                    stringParam(params, "jar", ""),
                    stringParam(params, "raw", ""),
                    stringParam(params, "class", ""),
                    stringParam(params, "ext", ""));
        }
        if ("ping".equals(method)) {
            return memoryInfo();
        }
        if ("shutdown".equals(method)) {
            System.exit(0);
            return JsonNull.INSTANCE;
        }

        String site = stringParam(params, "site", "default");
        Spider spider = spiders.get(site);
        if (spider == null) {
            throw new IllegalStateException("site not opened: " + site);
        }
        Object lock = locks.computeIfAbsent(site, key -> new Object());
        synchronized (lock) {
            switch (method) {
                case "home":
                    return parse(spider.homeContent(boolParam(params, "filter", true)));
                case "homeVideo":
                    return parse(spider.homeVideoContent());
                case "category":
                    return parse(spider.categoryContent(
                            stringParam(params, "tid", ""),
                            stringParam(params, "pg", "1"),
                            boolParam(params, "filter", true),
                            stringMap(params.getAsJsonObject("extend"))));
                case "detail":
                    return parse(spider.detailContent(stringList(params, "ids")));
                case "search":
                    return parse(spider.searchContent(
                            stringParam(params, "key", ""),
                            boolParam(params, "quick", false)));
                case "player":
                    return parse(spider.playerContent(
                            stringParam(params, "flag", ""),
                            stringParam(params, "id", ""),
                            stringList(params, "vipFlags")));
                case "action":
                    return parse(spider.action(stringParam(params, "action", "")));
                default:
                    throw new IllegalArgumentException("unknown method: " + method);
            }
        }
    }

    private JsonElement open(String site, String jar, String raw, String className, String ext) throws Exception {
        if (site.isEmpty()) throw new IllegalArgumentException("site is empty");
        if (jar.isEmpty() || className.isEmpty()) throw new IllegalArgumentException("jar/class is empty");
        Spider existing = spiders.get(site);
        if (existing != null) {
            JsonObject info = new JsonObject();
            info.addProperty("site", site);
            info.addProperty("class", existing.getClass().getName());
            info.addProperty("reused", true);
            return info;
        }

        File file = new File(jar).getCanonicalFile();
        File rawFile = raw.isEmpty() ? null : new File(raw).getCanonicalFile();
        String path = file.getPath() + (rawFile == null ? "" : "|" + rawFile.getPath());
        URLClassLoader loader = loaders.get(path);
        if (loader == null) {
            // raw zip 里有 dex2jar 不会带过来的 assets/（例如加固壳的 .so 和配置），
            // 挂进同一个 classloader，spider 的 getResourceAsStream("assets/...") 才找得到。
            URL[] urls = rawFile == null
                    ? new URL[]{file.toURI().toURL()}
                    : new URL[]{file.toURI().toURL(), rawFile.toURI().toURL()};
            URLClassLoader created = new URLClassLoader(urls, SpiderHost.class.getClassLoader());
            loader = loaders.putIfAbsent(path, created);
            if (loader == null) loader = created;
        }
        Thread.currentThread().setContextClassLoader(loader);
        Class<?> clazz = Class.forName(className, true, loader);
        Object instance = clazz.getDeclaredConstructor().newInstance();
        if (!(instance instanceof Spider)) {
            throw new IllegalStateException(className + " is not a Spider subclass");
        }
        Spider spider = (Spider) instance;
        spider.init(new Application(), ext);
        spiders.put(site, spider);
        locks.put(site, new Object());

        JsonObject info = new JsonObject();
        info.addProperty("site", site);
        info.addProperty("class", className);
        info.addProperty("reused", false);
        return info;
    }

    private static JsonElement parse(String value) {
        if (value == null || value.isEmpty()) return JsonNull.INSTANCE;
        try {
            return JsonParser.parseString(value);
        } catch (Exception ignored) {
            return new JsonPrimitive(value);
        }
    }

    private static String stringParam(JsonObject params, String key, String fallback) {
        return params.has(key) && !params.get(key).isJsonNull() ? params.get(key).getAsString() : fallback;
    }

    private static boolean boolParam(JsonObject params, String key, boolean fallback) {
        return params.has(key) && !params.get(key).isJsonNull() ? params.get(key).getAsBoolean() : fallback;
    }

    private static List<String> stringList(JsonObject params, String key) {
        List<String> result = new ArrayList<>();
        if (!params.has(key) || !params.get(key).isJsonArray()) return result;
        for (JsonElement item : params.getAsJsonArray(key)) {
            result.add(item.getAsString());
        }
        return result;
    }

    private static HashMap<String, String> stringMap(JsonObject object) {
        HashMap<String, String> result = new HashMap<>();
        if (object == null) return result;
        for (Map.Entry<String, JsonElement> entry : object.entrySet()) {
            result.put(entry.getKey(), entry.getValue().isJsonNull() ? "" : entry.getValue().getAsString());
        }
        return result;
    }

    private JsonElement memoryInfo() {
        Runtime runtime = Runtime.getRuntime();
        long usedMb = (runtime.totalMemory() - runtime.freeMemory()) / 1024 / 1024;
        long totalMb = runtime.totalMemory() / 1024 / 1024;
        long maxMb = runtime.maxMemory() / 1024 / 1024;
        JsonObject info = new JsonObject();
        info.addProperty("usedMb", usedMb);
        info.addProperty("totalMb", totalMb);
        info.addProperty("maxMb", maxMb);
        info.addProperty("sites", spiders.size());
        return info;
    }
}
