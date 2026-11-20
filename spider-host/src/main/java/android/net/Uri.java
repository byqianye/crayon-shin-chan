package android.net;

import java.io.File;
import java.net.URI;
import java.net.URISyntaxException;
import java.util.Collections;
import java.util.List;

public final class Uri {
    private final String raw;

    private Uri(String raw) {
        this.raw = raw == null ? "" : raw;
    }

    public static Uri parse(String value) {
        return new Uri(value);
    }

    public static Uri fromFile(File file) {
        return new Uri(file.toURI().toString());
    }

    public String getScheme() {
        try {
            return new URI(raw).getScheme();
        } catch (URISyntaxException ignored) {
            return null;
        }
    }

    public String getHost() {
        try {
            return new URI(raw).getHost();
        } catch (URISyntaxException ignored) {
            return null;
        }
    }

    public String getPath() {
        try {
            return new URI(raw).getPath();
        } catch (URISyntaxException ignored) {
            return raw;
        }
    }

    public List<String> getPathSegments() {
        String path = getPath();
        if (path == null || path.isEmpty() || "/".equals(path)) return Collections.emptyList();
        return java.util.Arrays.asList(path.replaceFirst("^/", "").split("/"));
    }

    public String getQueryParameter(String key) {
        try {
            String query = new URI(raw).getRawQuery();
            if (query == null) return null;
            for (String pair : query.split("&")) {
                int eq = pair.indexOf('=');
                String name = eq >= 0 ? pair.substring(0, eq) : pair;
                if (name.equals(key)) {
                    return eq >= 0 ? java.net.URLDecoder.decode(pair.substring(eq + 1), "UTF-8") : "";
                }
            }
        } catch (Exception ignored) {
            // 返回 null
        }
        return null;
    }

    @Override
    public String toString() {
        return raw;
    }
}
