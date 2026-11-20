package android.content;

import android.net.Uri;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/** ContentResolver 的最小实现；spider 主要用它读取本地 file:// 配置。 */
public class ContentResolver {
    private final Context context;

    public ContentResolver(Context context) {
        this.context = context;
    }

    public InputStream openInputStream(Uri uri) throws FileNotFoundException {
        String scheme = uri == null ? null : uri.getScheme();
        if (uri != null && ("file".equals(scheme) || scheme == null)) {
            return new FileInputStream(new File(uri.getPath()));
        }
        throw new FileNotFoundException("不支持的 content URI：" + uri);
    }

    public OutputStream openOutputStream(Uri uri) throws FileNotFoundException {
        String scheme = uri == null ? null : uri.getScheme();
        if (uri != null && ("file".equals(scheme) || scheme == null)) {
            return new FileOutputStream(new File(uri.getPath()));
        }
        throw new FileNotFoundException("不支持的 content URI：" + uri);
    }

    public String getType(Uri uri) {
        return null;
    }

    public Context getContext() {
        return context;
    }
}
