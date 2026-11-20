package android.app;

import android.content.Context;

public class Activity extends Context {
    public Activity() {
        super();
    }

    public void runOnUiThread(Runnable action) {
        action.run();
    }
}
