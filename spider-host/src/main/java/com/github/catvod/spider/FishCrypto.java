package com.github.catvod.spider;

import java.util.concurrent.Callable;

/**
 * FishCrypto 的轻量桌面替身。
 *
 * FishCloud / Cloud 的核心方法只把 FishCrypto 当作“守卫闸门”：
 * beginHostEntry 返回非 null 就直接返回占位提示。这里让闸门永远放行，
 * 普通 Fish 站点的网络请求仍由 OkHttp 完成；需要真实 native 解密的站点
 * 以后再由 unidbg 桥补上。
 */
public class FishCrypto {
    public static volatile boolean a = true;
    public static volatile String b = "";
    public static volatile String[] c = new String[0];

    public static String beginHostEntry(Object owner, String action) {
        return null;
    }

    public static void endHostEntry(Throwable error) {
    }

    public static String a(String value) {
        return value;
    }

    public static String b(String value) {
        return value;
    }

    public static boolean d(long value) {
        return true;
    }

    public static Object f(Object value) {
        return value;
    }

    public static String g(String value) {
        return value;
    }

    public static String h(Throwable error) {
        return "";
    }

    public static boolean i(int value, String text) {
        return true;
    }

    public static int j(String value) {
        return 0;
    }

    public static boolean k() {
        return true;
    }

    public static boolean l() {
        return true;
    }

    public static int m(String value) {
        return 0;
    }

    public static boolean n() {
        return true;
    }

    public static String o() {
        return "";
    }

    public static void p() {
    }

    public static void q() {
    }

    public static void r() {
    }

    public static byte[] s(String a, String b, byte[] data) {
        return data;
    }

    public static byte[] t(String a, String b, String c, String d, byte[] data) {
        return data;
    }

    public static String u(String a, String b) {
        return a;
    }

    public static String v(String value) {
        return value;
    }

    public static String w(String a, String b) {
        return a;
    }

    public static String x(int value) {
        return "";
    }

    public static void y(int... values) {
    }

    public static void acceptShellHome(Object a, Object b, long c, String d, Callable<Boolean> callback) {
    }

    public static Object[] blockedProxy() {
        return null;
    }

    public static boolean businessReady() {
        return true;
    }

    public static void cancelHostHome(Object owner) {
    }

    public static boolean hostAllowed() {
        return true;
    }

    public static String hostEntry(Object owner, String action) {
        return null;
    }

    public static String invokeHost(Object owner, String action, long timeout, Callable<String> callback) {
        try {
            return callback.call();
        } catch (Exception error) {
            return "";
        }
    }
}
