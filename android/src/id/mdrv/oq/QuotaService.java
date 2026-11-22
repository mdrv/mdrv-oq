// The quota's foreground service: a persistent notification that polls
// the Z.AI quota every interval, reading the same settings store the
// Rust UI writes (`files/state/mdrv-oq/settings.db`, turso = plain
// SQLite). Deliberately framework-only: no androidx, no Kotlin.
//
// Started once (adb, or later from the UI); runs until stopped. The
// notification shows `GLM 42% · resets 03:16` like the desktop tray.

package id.mdrv.oq;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.database.Cursor;
import android.database.sqlite.SQLiteDatabase;
import android.os.IBinder;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;

public class QuotaService extends Service {
    private static final String CHANNEL_ID = "quota";
    private static final int NOTIFICATION_ID = 1;
    private static final String API_URL =
            "https://api.z.ai/api/monitor/usage/quota/limit";
    private static final long DEFAULT_INTERVAL_SECS = 300;

    private final ScheduledExecutorService executor =
            Executors.newSingleThreadScheduledExecutor();
    private volatile boolean stopped = false;
    private ScheduledFuture<?> task;
    private volatile long lastPollMs = 0L;
    // Raw values from the last fetch, mirrored to state.json for the UI.
    private volatile double lastPct = Double.NaN;
    private volatile long lastResetMs = 0L;
    private volatile String lastLevel = null;
    private volatile String lastError = null;

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager nm = getSystemService(NotificationManager.class);
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "Quota", NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("Coding plan quota");
        nm.createNotificationChannel(channel);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && "STOP".equals(intent.getAction())) {
            stopSelf();
            return START_NOT_STICKY;
        }
        Notification notification = build("starting\u2026");
        if (android.os.Build.VERSION.SDK_INT >= 34) {
            startForeground(
                    NOTIFICATION_ID, notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
        // Repeated starts (app open, boot, adb) restart the tick instead
        // of stacking loops — and each tick re-reads the interval, so a
        // changed interval applies within a second, no force-stop.
        if (task != null) {
            task.cancel(false);
        }
        task = executor.scheduleWithFixedDelay(
                this::tick, 0, 1, TimeUnit.SECONDS);
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        stopped = true;
        executor.shutdownNow();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    /// Poll interval from the shared settings store, clamped to >= 30s.
    private long intervalSecs() {
        String raw = readSetting("interval_secs");
        if (raw != null) {
            try {
                long parsed = Long.parseLong(raw.trim());
                if (parsed >= 30) {
                    return parsed;
                }
            } catch (NumberFormatException ignored) {
            }
        }
        return DEFAULT_INTERVAL_SECS;
    }

    /// The 1 s tick: fetches when the last poll is older than the
    /// interval (re-read every tick, so a changed interval applies
    /// within a tick) or when the UI's refresh marker is waiting.
    private void tick() {
        if (stopped) {
            return;
        }
        boolean poked = consumeRefreshMarker();
        boolean due = System.currentTimeMillis() - lastPollMs
                >= intervalSecs() * 1000L;
        if (due || poked) {
            poll();
        }
    }

    /// The UI asks for an out-of-band poll by creating
    /// `files/state/refresh`; consume it and say yes.
    private boolean consumeRefreshMarker() {
        java.io.File marker =
                new java.io.File(getFilesDir(), "state/refresh");
        boolean existed = marker.exists();
        marker.delete();
        return existed;
    }

    private void poll() {
        if (stopped) {
            return;
        }
        try {
            NotificationManager nm = getSystemService(NotificationManager.class);
            lastPollMs = System.currentTimeMillis();
            String text = status();
            writeStateFile();
            nm.notify(NOTIFICATION_ID, build(text));
        } catch (Exception ignored) {
            // A dead notification manager should not kill the loop.
        }
    }

    /// One fetch, rendered like the desktop tray line. Also stashes the
    /// raw values for `writeStateFile` (the UI mirrors them instead of
    /// polling on its own — one poller, one clock).
    private String status() {
        String key = readSetting("api_key");
        if (key == null || key.trim().isEmpty()) {
            return fail("no API key \u2014 open the app to set one");
        }
        String body = fetch(key.trim());
        if (body == null) {
            return fail("fetch failed");
        }
        try {
            JSONObject data = new JSONObject(body).getJSONObject("data");
            JSONArray limits = data.getJSONArray("limits");
            Double percentage = null;
            Long resetMs = null;
            for (int i = 0; i < limits.length(); i++) {
                JSONObject limit = limits.getJSONObject(i);
                if (!"TOKENS_LIMIT".equals(limit.optString("type"))) {
                    continue;
                }
                percentage = limit.optDouble("percentage");
                resetMs = limit.optLong("nextResetTime");
                break;
            }
            if (percentage == null || percentage.isNaN()) {
                return noData();
            }
            lastPct = percentage;
            lastResetMs = resetMs != null ? resetMs : 0L;
            lastLevel = data.optString("level", null);
            lastError = null;
            StringBuilder line = new StringBuilder("GLM ")
                    .append(Math.round(percentage)).append("%");
            if (lastResetMs > 0 && !"0".equals(readSetting("show_reset"))) {
                line.append(" \u00b7 \u267B\uFE0F ").append(clock(lastResetMs));
            }
            return line.toString();
        } catch (Exception e) {
            return fail("\u2716 " + e.getMessage());
        }
    }

    /// No TOKENS_LIMIT row yet: clear any sample, no error either.
    private String noData() {
        lastPct = Double.NaN;
        lastResetMs = 0L;
        lastLevel = null;
        lastError = null;
        return "\u2026";
    }

    /// Record an error as the latest state, clearing any sample.
    private String fail(String message) {
        lastPct = Double.NaN;
        lastResetMs = 0L;
        lastLevel = null;
        lastError = message;
        return message;
    }

    /// Mirror the snapshot for the Rust UI. Atomic-ish via rename.
    private void writeStateFile() {
        try {
            java.io.File dir = new java.io.File(getFilesDir(), "state");
            if (!dir.isDirectory() && !dir.mkdirs()) {
                return;
            }
            JSONObject o = new JSONObject();
            o.put("percentage", Double.isNaN(lastPct) ? null : lastPct);
            o.put("next_reset_ms", lastResetMs > 0 ? lastResetMs : null);
            o.put("level", lastLevel);
            o.put("error",
                    lastError == null || lastError.isEmpty() ? null : lastError);
            o.put("fetched_at_ms", lastPollMs);
            java.io.File tmp = new java.io.File(dir, "state.json.tmp");
            try (java.io.FileWriter w = new java.io.FileWriter(tmp)) {
                w.write(o.toString());
            }
            java.io.File out = new java.io.File(dir, "state.json");
            if (out.exists() && !out.delete()) {
                return;
            }
            tmp.renameTo(out);
        } catch (Exception ignored) {
        }
    }

    private String fetch(String key) {
        HttpURLConnection connection = null;
        try {
            connection = (HttpURLConnection) new URL(API_URL).openConnection();
            connection.setConnectTimeout(20_000);
            connection.setReadTimeout(20_000);
            connection.setRequestProperty("Authorization", key);
            int code = connection.getResponseCode();
            if (code != 200) {
                return null;
            }
            BufferedReader reader = new BufferedReader(new InputStreamReader(
                    connection.getInputStream(), StandardCharsets.UTF_8));
            StringBuilder body = new StringBuilder();
            String line;
            while ((line = reader.readLine()) != null) {
                body.append(line);
            }
            reader.close();
            return body.toString();
        } catch (Exception e) {
            return null;
        } finally {
            if (connection != null) {
                connection.disconnect();
            }
        }
    }

    /// One value from the shared turso store; null when unreadable/unset.
    private String readSetting(String key) {
        String path = new java.io.File(getFilesDir(),
                "state/mdrv-oq/settings.db").getAbsolutePath();
        SQLiteDatabase db = null;
        try {
            db = SQLiteDatabase.openDatabase(
                    path, null, SQLiteDatabase.OPEN_READONLY);
            Cursor cursor = db.rawQuery(
                    "SELECT value FROM settings WHERE key = ?",
                    new String[] {key});
            String value = null;
            if (cursor.moveToFirst()) {
                value = cursor.getString(0);
            }
            cursor.close();
            return value;
        } catch (Exception e) {
            return null;
        } finally {
            if (db != null) {
                db.close();
            }
        }
    }

    private String clock(long epochMs) {
        return new SimpleDateFormat("HH:mm:ss", Locale.US)
                .format(new Date(epochMs));
    }

    private Notification build(String text) {
        // Tapping the notification opens the app (the launcher activity).
        Notification.Builder builder = new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle("mdrv-oq")
                .setContentText(text)
                // The last-updated indicator: SystemUI renders a stopwatch
                // from the poll time itself, so it keeps ticking even when
                // MIUI/Doze throttles this process (a repost-based age
                // label lagged minutes behind on a Poco F7).
                .setWhen(Math.max(lastPollMs, 1L))
                .setUsesChronometer(true)
                .setOngoing(true)
                .setOnlyAlertOnce(true);
        Intent open = getPackageManager().getLaunchIntentForPackage(
                getPackageName());
        if (open != null) {
            open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
            builder.setContentIntent(PendingIntent.getActivity(
                    this, 0, open,
                    PendingIntent.FLAG_UPDATE_CURRENT
                            | PendingIntent.FLAG_IMMUTABLE));
        }
        return builder.build();
    }
}
