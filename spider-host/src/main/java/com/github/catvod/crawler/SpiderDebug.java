package com.github.catvod.crawler;

public final class SpiderDebug {
    private SpiderDebug() {
    }

    public static void log(String message) {
        System.err.println("[spider] " + message);
    }

    public static void log(String tag, String message) {
        System.err.println("[spider:" + tag + "] " + message);
    }

    public static void log(Throwable error) {
        error.printStackTrace(System.err);
    }

    public static void log(String message, Throwable error) {
        System.err.println("[spider] " + message);
        error.printStackTrace(System.err);
    }

    public static void log(String tag, String message, Throwable error) {
        System.err.println("[spider:" + tag + "] " + message);
        error.printStackTrace(System.err);
    }
}
