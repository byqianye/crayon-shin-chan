package android.content;

import java.io.File;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * 最小 Android Context 替身：只实现 Spider 实际会调用的少数方法。
 * 保持类名与 Android 一致，spider jar 不需要重新编译。
 */
public class Context {
    private final File baseDir;
    private final Map<String, SharedPreferences> sharedPreferences = new ConcurrentHashMap<>();
    private final ContentResolver contentResolver = new ContentResolver(this);

    public Context() {
        this.baseDir = new File(System.getProperty("java.io.tmpdir"), "crayon-spider-host");
        //noinspection ResultOfMethodCallIgnored
        this.baseDir.mkdirs();
    }

    public Context getApplicationContext() {
        return this;
    }

    public File getFilesDir() {
        File dir = new File(baseDir, "files");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        return dir;
    }

    public File getCacheDir() {
        File dir = new File(baseDir, "cache");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        return dir;
    }

    public File getExternalFilesDir(String type) {
        File dir = new File(baseDir, type == null ? "external" : "external-" + type);
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        return dir;
    }

    public File getNoBackupFilesDir() {
        File dir = new File(baseDir, "no_backup");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        return dir;
    }

    public File getCodeCacheDir() {
        File dir = new File(baseDir, "code_cache");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        return dir;
    }

    public File getExternalCacheDir() {
        File dir = new File(baseDir, "external_cache");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        return dir;
    }

    public String getPackageName() {
        return "com.crayonshinchan.spiderhost";
    }

    public android.os.Looper getMainLooper() {
        return android.os.Looper.getMainLooper();
    }

    public SharedPreferences getSharedPreferences(String name, int mode) {
        return sharedPreferences.computeIfAbsent(name == null ? "default" : name,
            key -> new SharedPreferencesImpl());
    }

    public ContentResolver getContentResolver() {
        return contentResolver;
    }
}
