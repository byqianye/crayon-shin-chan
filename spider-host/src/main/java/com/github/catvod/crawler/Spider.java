package com.github.catvod.crawler;

import android.content.Context;
import okhttp3.Dns;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * CatVod Spider 公开接口的干净实现。
 * 只定义宿主需要调用的方法签名，不包含任何第三方 spider 代码。
 */
public abstract class Spider {
    public void init(Context context, String extend) {
    }

    public String homeContent(boolean filter) {
        return "";
    }

    public String homeVideoContent() {
        return "";
    }

    public String categoryContent(String tid, String pg, boolean filter, HashMap<String, String> extend) {
        return "";
    }

    public String detailContent(List<String> ids) {
        return "";
    }

    public String searchContent(String key, boolean quick) {
        return "";
    }

    public String searchContent(String key, boolean quick, String pg) {
        return "";
    }

    public String playerContent(String flag, String id, List<String> vipFlags) {
        return "";
    }

    public String action(String action) {
        return "";
    }

    public Object[] proxyLocal(Map<String, String> params) {
        return null;
    }

    public void destroy() {
    }

    public static Dns safeDns() {
        return Dns.SYSTEM;
    }
}
