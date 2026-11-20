package android.util;

import java.nio.charset.StandardCharsets;

/**
 * android.util.Base64 的可运行替代。
 *
 * android.jar 里的实现是 "Stub!"，而很多 Spider 在 init() 里就会调用
 * encodeToString 生成签名/设备信息，所以这里必须给出真实实现。
 */
public final class Base64 {
    public static final int DEFAULT = 0;
    public static final int NO_PADDING = 1;
    public static final int NO_WRAP = 2;
    public static final int CRLF = 4;
    public static final int URL_SAFE = 8;

    private Base64() {
    }

    public static byte[] decode(String input, int flags) {
        if (input == null) return null;
        java.util.Base64.Decoder decoder = (flags & URL_SAFE) != 0
            ? java.util.Base64.getUrlDecoder()
            : java.util.Base64.getDecoder();
        try {
            return decoder.decode(input.replaceAll("\\s", ""));
        } catch (IllegalArgumentException error) {
            // Android 对非法输入会尽量解码，这里退回 MIME 解码后再试一次
            return java.util.Base64.getMimeDecoder().decode(input);
        }
    }

    public static byte[] decode(byte[] input, int flags) {
        return input == null ? null : decode(new String(input, StandardCharsets.ISO_8859_1), flags);
    }

    public static String encodeToString(byte[] input, int flags) {
        byte[] encoded = encode(input, flags);
        return encoded == null ? null : new String(encoded, StandardCharsets.ISO_8859_1);
    }

    public static byte[] encode(byte[] input, int flags) {
        if (input == null) return null;
        java.util.Base64.Encoder encoder = (flags & URL_SAFE) != 0
            ? java.util.Base64.getUrlEncoder()
            : java.util.Base64.getEncoder();
        if ((flags & NO_PADDING) != 0) encoder = encoder.withoutPadding();
        byte[] encoded = encoder.encode(input);
        if ((flags & CRLF) != 0) {
            String text = new String(encoded, StandardCharsets.ISO_8859_1)
                .replace("\n", "\r\n");
            return text.getBytes(StandardCharsets.ISO_8859_1);
        }
        return encoded;
    }
}
