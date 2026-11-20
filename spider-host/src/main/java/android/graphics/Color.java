package android.graphics;

public final class Color {
    public static final int BLACK = 0xff000000;
    public static final int WHITE = 0xffffffff;
    public static final int TRANSPARENT = 0;
    public static final int RED = 0xffff0000;
    public static final int GREEN = 0xff00ff00;
    public static final int BLUE = 0xff0000ff;

    private Color() {
    }

    public static int parseColor(String value) {
        if (value == null || value.isEmpty()) return BLACK;
        String text = value.trim();
        if (text.startsWith("#")) {
            String hex = text.substring(1);
            if (hex.length() == 6) hex = "ff" + hex;
            if (hex.length() == 8) {
                try {
                    return (int) Long.parseLong(hex, 16);
                } catch (NumberFormatException ignored) {
                    return BLACK;
                }
            }
        }
        return BLACK;
    }

    public static int rgb(int red, int green, int blue) {
        return 0xff000000 | ((red & 0xff) << 16) | ((green & 0xff) << 8) | (blue & 0xff);
    }

    public static int argb(int alpha, int red, int green, int blue) {
        return ((alpha & 0xff) << 24) | ((red & 0xff) << 16) | ((green & 0xff) << 8) | (blue & 0xff);
    }
}
