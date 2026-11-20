package android.os;

public class Handler {
    public Handler() {
    }

    public Handler(Looper looper) {
    }

    public boolean post(Runnable action) {
        action.run();
        return true;
    }

    public boolean postDelayed(Runnable action, long delayMillis) {
        action.run();
        return true;
    }

    public void removeCallbacks(Runnable action) {
    }

    public void sendEmptyMessage(int what) {
    }
}
