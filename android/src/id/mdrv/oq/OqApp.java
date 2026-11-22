// Starts the quota foreground service on every process start: the app
// being opened, or the system restarting a dead service's process.
//
// `com.longbridge.nsh.AppActivity` is declared FINAL in the nsh runtime
// dex, so it cannot be subclassed to hook the activity lifecycle — the
// Application class is the remaining app-owned entry point, and its
// ActivityLifecycleCallbacks cover the rest: the first activity start
// brings the app to the foreground, where a foreground-service start is
// allowed, so the notification lands with the first frame instead of
// trailing the backoff (Android 12+ rejects FGS starts while the app is
// not yet in the foreground state — all Application.onCreate can do is
// attempt early and retry).
//
package id.mdrv.oq;

import android.app.Activity;
import android.app.Application;
import android.content.Intent;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;

public class OqApp extends Application {
    @Override
    public void onCreate() {
        super.onCreate();
        final Handler handler = new Handler(Looper.getMainLooper());
        final long[] delay = {50};
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
        // The first activity start makes the app foreground: the start
        // goes through on the spot, and the service's tick begins with a
        // fresh poll — so opening the app always refreshes. The other
        // lifecycle methods are overridden empty: the compile-time
        // android.jar predates the interface's default methods.
        registerActivityLifecycleCallbacks(
                new Application.ActivityLifecycleCallbacks() {
                    @Override
                    public void onActivityStarted(Activity activity) {
                        attempt[0].run();
                    }

                    @Override
                    public void onActivityCreated(Activity activity, Bundle savedInstanceState) {
                    }

                    @Override
                    public void onActivityResumed(Activity activity) {
                    }

                    @Override
                    public void onActivityPaused(Activity activity) {
                    }

                    @Override
                    public void onActivityStopped(Activity activity) {
                    }

                    @Override
                    public void onActivitySaveInstanceState(
                            Activity activity, Bundle outState) {
                    }

                    @Override
                    public void onActivityDestroyed(Activity activity) {
                    }
                });
        attempt[0].run();
    }
}
