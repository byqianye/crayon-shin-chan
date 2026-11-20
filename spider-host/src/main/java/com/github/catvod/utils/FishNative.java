package com.github.catvod.utils;

import android.content.Context;

import java.io.File;
import java.nio.charset.StandardCharsets;

/**
 * FishGuard 的桌面侧轻量桥。
 *
 * 这里只覆盖“加载/守卫状态/明文 ext”这类不需要真实 ARM 运算的接口，
 * 让普通 Fish 站点先跑起来；需要 native 解密的 ext 仍应走 unidbg。
 * 类名与 spider jar 内的 FishNative 完全一致，靠父类加载器优先命中。
 */
public final class FishNative {
    private FishNative() {
    }

    public static void load(String path) {
    }

    public static void loadAndRegister(String path) {
    }

    public static void requireReadOnly(File file) {
    }

    public static int _guardConfig(String value) {
        return 0;
    }

    public static boolean _guardReady() {
        return true;
    }

    public static String _honHonKeyId() {
        return "";
    }

    public static boolean _honHonReady() {
        return false;
    }

    public static String _honHonRoute() {
        return "";
    }

    public static byte[] _honHonServerPublicKey() {
        return new byte[0];
    }

    public static byte[] _honHonSign(byte[] input) {
        return input;
    }

    public static int _kChk(String value) {
        return 0;
    }

    public static String _pan123ShareGetApi(String a, String b, String c, String d, int e, int f) {
        return "";
    }

    public static String _pan123UnlimitedUrl(String value) {
        return value;
    }

    public static byte[] _secAesCbcDec(String a, String b, byte[] input) {
        return input;
    }

    public static byte[] _secAesCbcEnc(String a, String b, byte[] input) {
        return input;
    }

    public static byte[] _secAesCbcEncJsonAccess(String a, String b, String c, String d, byte[] input) {
        return input;
    }

    public static String _secText(String value) {
        return value;
    }

    public static String _sv(int value) {
        return "";
    }

    public static byte[] aesDec(byte[] data, byte[] key, byte[] iv) {
        return data;
    }

    public static byte[] aesEnc(byte[] data, byte[] key, byte[] iv) {
        return data;
    }

    public static byte[] cKey(String value, String key) {
        return new byte[0];
    }

    public static String[] cfgUrls() {
        return new String[0];
    }

    public static void checkAlive() {
    }

    public static String defCfg() {
        return "{}";
    }

    public static byte[] extDe(String value) {
        return value == null ? new byte[0] : value.getBytes(StandardCharsets.UTF_8);
    }

    public static byte[] extEn(String value) {
        return value == null ? new byte[0] : value.getBytes(StandardCharsets.UTF_8);
    }

    public static int fishGoAbiVersion() {
        return 8;
    }

    public static long guardLease(int value, String key) {
        return 0L;
    }

    public static boolean guardUse(int value, long lease, String key) {
        return true;
    }

    public static boolean hostAllowed() {
        return true;
    }

    public static int hostApply(String value) {
        return 0;
    }

    public static void hostExit(long value) {
    }

    public static void hostInit(Context context) {
    }

    public static String hostStatus() {
        return "healthy";
    }

    public static void nExit() {
    }

    public static boolean nKilled() {
        return false;
    }

    public static String popupAck(long value) {
        return "";
    }

    public static long popupArm(String a, String b) {
        return 0L;
    }

    public static void popupCheck() {
    }

    public static boolean popupRequire(String a, String b, String c) {
        return false;
    }

    public static void popupShown(long value) {
    }

    public static void popupViolation(long value) {
    }

    public static String rPub() {
        return "";
    }

    public static void register() {
    }

    public static int spVer() {
        return 1;
    }

    public static String ssKey() {
        return "";
    }

    public static int startGo(int a, String b, String c, byte[] d) {
        return 0;
    }

    public static void stop() {
    }

    public static int svN() {
        return 1;
    }

    public static byte[] urlDe(String value) {
        return value == null ? new byte[0] : value.getBytes(StandardCharsets.UTF_8);
    }

    public static byte[] urlEn(String value) {
        return value == null ? new byte[0] : value.getBytes(StandardCharsets.UTF_8);
    }
}
