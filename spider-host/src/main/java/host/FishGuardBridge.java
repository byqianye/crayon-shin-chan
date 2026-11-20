package host;

import com.github.unidbg.AndroidEmulator;
import com.github.unidbg.arm.backend.Unicorn2Factory;
import com.github.unidbg.file.linux.AndroidFileIO;
import com.github.unidbg.linux.android.AndroidEmulatorBuilder;
import com.github.unidbg.linux.android.AndroidResolver;
import com.github.unidbg.linux.android.dvm.AbstractJni;
import com.github.unidbg.linux.android.dvm.BaseVM;
import com.github.unidbg.linux.android.dvm.DalvikModule;
import com.github.unidbg.linux.android.dvm.DvmClass;
import com.github.unidbg.linux.android.dvm.DvmObject;
import com.github.unidbg.linux.android.dvm.StringObject;
import com.github.unidbg.linux.android.dvm.VM;
import com.github.unidbg.linux.android.dvm.VarArg;
import com.github.unidbg.memory.Memory;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/**
 * 一次性 FishGuard 解包/解密桥：只在首次遇到 FishGuard 站点时启动，
 * 调完 extDe/urlDe 立即退出，不把 unidbg 内存留在常驻 Spider 宿主里。
 */
public final class FishGuardBridge extends AbstractJni {
    private File root;

    @Override
    public DvmObject<?> callStaticObjectMethod(BaseVM vm, DvmClass dvmClass, String signature, VarArg varArg) {
        if ("com/github/catvod/spider/Init->context()Landroid/app/Application;".equals(signature)) {
            return vm.resolveClass("android/app/Application").newObject(null);
        }
        return super.callStaticObjectMethod(vm, dvmClass, signature, varArg);
    }

    @Override
    public DvmObject<?> callObjectMethod(BaseVM vm, DvmObject<?> dvmObject, String signature, VarArg varArg) {
        if ("android/app/Application->getFilesDir()Ljava/io/File;".equals(signature)
                || "android/content/Context->getFilesDir()Ljava/io/File;".equals(signature)) {
            return vm.resolveClass("java/io/File").newObject(new File("/data/local/tmp/fish-root"));
        }
        if ("android/app/Application->getCacheDir()Ljava/io/File;".equals(signature)
                || "android/content/Context->getCacheDir()Ljava/io/File;".equals(signature)) {
            return vm.resolveClass("java/io/File").newObject(new File("/data/local/tmp/fish-cache"));
        }
        if ("java/io/File->getAbsolutePath()Ljava/lang/String;".equals(signature)
                || "java/io/File->getPath()Ljava/lang/String;".equals(signature)) {
            Object value = dvmObject.getValue();
            String path = value instanceof File ? ((File) value).getPath().replace('\\', '/') : "/tmp/fish";
            if (path.matches("^[A-Za-z]:.*")) path = path.substring(2);
            if (!path.startsWith("/")) path = "/" + path;
            return new StringObject(vm, path);
        }
        return super.callObjectMethod(vm, dvmObject, signature, varArg);
    }

    private String run(File jar, String method, String input) throws Exception {
        root = new File(System.getProperty("java.io.tmpdir"), "fishguard-rootfs");
        root.mkdirs();
        String soName = null;
        File so = null;
        try (ZipFile zip = new ZipFile(jar)) {
            for (String candidate : new String[]{"assets/FishGuard-v8.so", "assets/FishGuard-v7.so"}) {
                ZipEntry entry = zip.getEntry(candidate);
                if (entry == null) continue;
                soName = candidate;
                so = File.createTempFile("fishguard-", ".so");
                try (InputStream in = zip.getInputStream(entry); OutputStream out = Files.newOutputStream(so.toPath())) {
                    in.transferTo(out);
                }
                break;
            }
        }
        if (so == null) throw new IllegalStateException("jar 中没有 FishGuard-v7/v8 so");

        boolean is64 = is64Bit(so);
        AndroidEmulator emulator = (is64
                ? AndroidEmulatorBuilder.for64Bit()
                : AndroidEmulatorBuilder.for32Bit())
                .setProcessName("fishguard")
                .setRootDir(root)
                .addBackendFactory(new Unicorn2Factory(true))
                .build();
        Memory memory = emulator.getMemory();
        memory.setLibraryResolver(new AndroidResolver(23));
        VM vm = emulator.createDalvikVM(jar);
        vm.setVerbose(false);
        vm.setJni(this);
        DalvikModule module = vm.loadLibrary(so, false);
        module.callJNI_OnLoad(emulator);
        DvmClass fishNative = vm.resolveClass("com/github/catvod/utils/FishNative");
        fishNative.callStaticJniMethod(emulator, "register()V");
        DvmObject<?> result = fishNative.callStaticJniMethodObject(
                emulator,
                method + "(Ljava/lang/String;)[B",
                input
        );
        if (result == null || !(result.getValue() instanceof byte[])) {
            throw new IllegalStateException("native 方法返回空");
        }
        byte[] bytes = (byte[]) result.getValue();
        emulator.close();
        return new String(bytes, StandardCharsets.UTF_8);
    }

    private static boolean is64Bit(File so) throws Exception {
        byte[] header = new byte[5];
        try (InputStream in = Files.newInputStream(so.toPath())) {
            if (in.read(header) != header.length) throw new IllegalStateException("so 头部过短");
        }
        if (header[0] != 0x7f || header[1] != 'E' || header[2] != 'L' || header[3] != 'F') {
            throw new IllegalStateException("不是 ELF");
        }
        return header[4] == 2;
    }

    private static String jsonEscape(String value) {
        return value.replace("\\", "\\\\").replace("\"", "\\\"")
                .replace("\n", "\\n").replace("\r", "\\r");
    }

    public static void main(String[] args) {
        String jar = null;
        String method = "extDe";
        String input = "";
        for (int i = 0; i < args.length; i++) {
            if ("--jar".equals(args[i])) jar = args[++i];
            else if ("--method".equals(args[i])) method = args[++i];
            else if ("--input".equals(args[i])) input = args[++i];
            else if ("--input-file".equals(args[i])) {
                try {
                    input = new String(Files.readAllBytes(new File(args[++i]).toPath()), StandardCharsets.UTF_8).trim();
                } catch (Exception error) {
                    System.out.println("{\"ok\":false,\"error\":\"" + jsonEscape(error.toString()) + "\"}");
                    return;
                }
            }
        }
        try {
            if (jar == null) throw new IllegalArgumentException("missing --jar");
            String output = new FishGuardBridge().run(new File(jar), method, input);
            System.out.println("{\"ok\":true,\"text\":\"" + jsonEscape(output) + "\"}");
        } catch (Throwable error) {
            System.out.println("{\"ok\":false,\"error\":\"" + jsonEscape(error.toString()) + "\"}");
        }
    }
}
