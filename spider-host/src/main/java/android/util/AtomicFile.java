package android.util;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;

/**
 * API 17 才加入 android.util.AtomicFile；Android 4.1 的 android.jar 里没有。
 * 一些 spider 用它保存配置/播放记录，这里按同名方法提供最小实现。
 */
public class AtomicFile {
    private final File baseName;
    private final File newName;
    private final File backupName;

    public AtomicFile(File baseName) {
        this.baseName = baseName;
        this.newName = new File(baseName.getPath() + ".new");
        this.backupName = new File(baseName.getPath() + ".bak");
    }

    public File getBaseFile() {
        return baseName;
    }

    public FileInputStream openRead() throws FileNotFoundException {
        if (backupName.exists()) {
            baseName.delete();
            backupName.renameTo(baseName);
        }
        return new FileInputStream(baseName);
    }

    public byte[] readFully() throws IOException {
        try (InputStream in = openRead(); ByteArrayOutputStream out = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[4096];
            int read;
            while ((read = in.read(buffer)) != -1) out.write(buffer, 0, read);
            return out.toByteArray();
        }
    }

    public FileOutputStream startWrite() throws IOException {
        if (baseName.exists()) {
            if (!baseName.renameTo(backupName)) {
                throw new IOException("Couldn't rename " + baseName + " to " + backupName);
            }
        }
        return new FileOutputStream(newName);
    }

    public void finishWrite(FileOutputStream stream) {
        if (stream != null) {
            try {
                stream.close();
            } catch (IOException ignored) {
                // 关闭失败不影响提交，下一次写入会覆盖 .new
            }
        }
        if (newName.renameTo(baseName)) backupName.delete();
    }

    public void failWrite(FileOutputStream stream) {
        if (stream != null) {
            try {
                stream.close();
            } catch (IOException ignored) {
                // 失败路径无需再抛
            }
        }
        newName.delete();
        if (backupName.exists()) backupName.renameTo(baseName);
    }

    public void delete() {
        baseName.delete();
        newName.delete();
        backupName.delete();
    }
}
