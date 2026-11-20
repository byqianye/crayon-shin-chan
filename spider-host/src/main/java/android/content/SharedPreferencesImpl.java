package android.content;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/** 进程内的 SharedPreferences 实现；宿主退出后清空。 */
final class SharedPreferencesImpl implements SharedPreferences {
    private final Map<String, Object> values = new ConcurrentHashMap<>();
    private final List<OnSharedPreferenceChangeListener> listeners = new ArrayList<>();

    @Override
    public Map<String, ?> getAll() {
        return Collections.unmodifiableMap(new HashMap<>(values));
    }

    @Override
    public String getString(String key, String defValue) {
        Object value = values.get(key);
        return value instanceof String ? (String) value : defValue;
    }

    @Override
    @SuppressWarnings("unchecked")
    public Set<String> getStringSet(String key, Set<String> defValues) {
        Object value = values.get(key);
        return value instanceof Set ? new HashSet<>((Set<String>) value) : defValues;
    }

    @Override
    public int getInt(String key, int defValue) {
        Object value = values.get(key);
        return value instanceof Number ? ((Number) value).intValue() : defValue;
    }

    @Override
    public long getLong(String key, long defValue) {
        Object value = values.get(key);
        return value instanceof Number ? ((Number) value).longValue() : defValue;
    }

    @Override
    public float getFloat(String key, float defValue) {
        Object value = values.get(key);
        return value instanceof Number ? ((Number) value).floatValue() : defValue;
    }

    @Override
    public boolean getBoolean(String key, boolean defValue) {
        Object value = values.get(key);
        return value instanceof Boolean ? (Boolean) value : defValue;
    }

    @Override
    public boolean contains(String key) {
        return values.containsKey(key);
    }

    @Override
    public Editor edit() {
        return new EditorImpl();
    }

    @Override
    public void registerOnSharedPreferenceChangeListener(OnSharedPreferenceChangeListener listener) {
        if (listener != null) listeners.add(listener);
    }

    @Override
    public void unregisterOnSharedPreferenceChangeListener(OnSharedPreferenceChangeListener listener) {
        listeners.remove(listener);
    }

    private final class EditorImpl implements Editor {
        private final Map<String, Object> updates = new HashMap<>();
        private final Set<String> removals = new HashSet<>();
        private boolean clear;

        @Override
        public Editor putString(String key, String value) {
            updates.put(key, value);
            removals.remove(key);
            return this;
        }

        @Override
        public Editor putStringSet(String key, Set<String> value) {
            updates.put(key, value == null ? null : new HashSet<>(value));
            removals.remove(key);
            return this;
        }

        @Override
        public Editor putInt(String key, int value) {
            updates.put(key, value);
            removals.remove(key);
            return this;
        }

        @Override
        public Editor putLong(String key, long value) {
            updates.put(key, value);
            removals.remove(key);
            return this;
        }

        @Override
        public Editor putFloat(String key, float value) {
            updates.put(key, value);
            removals.remove(key);
            return this;
        }

        @Override
        public Editor putBoolean(String key, boolean value) {
            updates.put(key, value);
            removals.remove(key);
            return this;
        }

        @Override
        public Editor remove(String key) {
            removals.add(key);
            updates.remove(key);
            return this;
        }

        @Override
        public Editor clear() {
            clear = true;
            return this;
        }

        @Override
        public boolean commit() {
            apply();
            return true;
        }

        @Override
        public void apply() {
            if (clear) values.clear();
            for (String key : removals) values.remove(key);
            for (Map.Entry<String, Object> entry : updates.entrySet()) {
                if (entry.getValue() == null) values.remove(entry.getKey());
                else values.put(entry.getKey(), entry.getValue());
            }
            for (OnSharedPreferenceChangeListener listener : listeners) {
                listener.onSharedPreferenceChanged(SharedPreferencesImpl.this, null);
            }
        }
    }
}
