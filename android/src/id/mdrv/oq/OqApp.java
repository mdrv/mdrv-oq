// Starts the quota foreground service on every process start: the app
// being opened, or the system restarting a dead service's process.
//
// `com.longbridge.nsh.AppActivity` is declared FINAL in the nsh runtime
// dex, so it cannot be subclassed to hook the activity lifecycle — the
// Application class is the remaining app-owned entry point.
//
// Android 12+ rejects startForegroundService while the app is not yet in
// the foreground state (Application.onCreate runs just before the first
// activity comes up), so early attempts are retried with backoff until
// the launcher activity has brought the app to the foreground.

package id.mdrv.oq;

import android.app.Application;
import android.content.Intent;
import android.os.Handler;
import android.os.Looper;

public class OqApp extends Application {
    @Override
    public void onCreate() {
        super.onCreate();
        final Handler handler = new Handler(Looper.getMainLooper());
        final long[] delay = {400};
        final Runnable[] attempt = new Runnable[1];
        attempt[0] = new Runnable() {
            @Override
            public void run() {
                try {
                    startForegroundService(
                            new Intent(OqApp.this, QuotaService.class));
                } catch (RuntimeException notForegroundYet) {
                    if (delay[0] <= 8_000) {
                        handler.postDelayed(attempt[0], delay[0]);
                        delay[0] *= 2;
                    }
                }
            }
        };
        attempt[0].run();
    }
}
