package android.os;

public final class SystemClock {
    private SystemClock() {
    }

    public static long elapsedRealtime() {
        return System.nanoTime() / 1_000_000;
    }

    public static long uptimeMillis() {
        return System.nanoTime() / 1_000_000;
    }

    public static void sleep(long millis) {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException ignored) {
            Thread.currentThread().interrupt();
        }
    }
}
