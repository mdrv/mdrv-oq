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
        long interval = intervalSecs();
        // Repeated starts (app open, boot, adb) reschedule instead of
        // stacking poll loops, and pick up a changed interval.
        if (task != null) {
            task.cancel(false);
        }
        executor.submit(this::poll);
        task = executor.scheduleWithFixedDelay(
                this::poll, interval, interval, TimeUnit.SECONDS);
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

    private void poll() {
        if (stopped) {
            return;
        }
        try {
            NotificationManager nm = getSystemService(NotificationManager.class);
            lastPollMs = System.currentTimeMillis();
            nm.notify(NOTIFICATION_ID,
                    build(status() + "  \uD83D\uDD04 " + lastUpdateAge()));
        } catch (Exception ignored) {
            // A dead notification manager should not kill the loop.
        }
    }

    /// How long ago the last poll ran, coarse ("now", "2m", "3h", "1d").
    private String lastUpdateAge() {
        long seconds = Math.max(0,
                (System.currentTimeMillis() - lastPollMs) / 1_000);
        if (seconds < 60) {
            return "now";
        }
        long minutes = seconds / 60;
        if (minutes < 60) {
            return minutes + "m";
        }
        long hours = minutes / 60;
        if (hours < 24) {
            return hours + "h";
        }
        return (hours / 24) + "d";
    }

    /// One fetch, rendered like the desktop tray line.
    private String status() {
        String key = readSetting("api_key");
        if (key == null || key.trim().isEmpty()) {
            return "no API key \u2014 open the app to set one";
        }
        String body = fetch(key.trim());
        if (body == null) {
            return "fetch failed";
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
            if (percentage == null) {
                return "\u2026";
            }
            StringBuilder line = new StringBuilder("GLM ")
                    .append(Math.round(percentage)).append("%");
            if (resetMs != null && resetMs > 0
                    && !"0".equals(readSetting("show_reset"))) {
                line.append(" \u00b7 resets ").append(clock(resetMs));
            }
            return line.toString();
        } catch (Exception e) {
            return "\u2716 " + e.getMessage();
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
