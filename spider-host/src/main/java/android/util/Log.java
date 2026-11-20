package android.util;

/** android.util.Log 的最小实现；避免 android.jar 的 "Stub!" 异常。 */
public final class Log {
    private Log() {
    }

    public static int v(String tag, String msg) {
        return println(tag, msg);
    }

    public static int d(String tag, String msg) {
        return println(tag, msg);
    }

    public static int i(String tag, String msg) {
        return println(tag, msg);
    }

    public static int w(String tag, String msg) {
        return println(tag, msg);
    }

    public static int e(String tag, String msg) {
        return println(tag, msg);
    }

    public static int e(String tag, String msg, Throwable tr) {
        return println(tag, msg + (tr == null ? "" : ": " + tr));
    }

    public static int wtf(String tag, String msg) {
        return println(tag, msg);
    }

    private static int println(String tag, String msg) {
        System.err.println("[spider] " + tag + ": " + msg);
        return 0;
    }
}
