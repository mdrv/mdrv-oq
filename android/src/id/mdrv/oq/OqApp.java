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
import android.app.KeyguardManager;
import android.content.Intent;
import android.hardware.biometrics.BiometricManager;
import android.hardware.biometrics.BiometricPrompt;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.os.Handler;
import android.os.Looper;
import com.longbridge.nsh.NshHostReply;
import com.longbridge.nsh.NshView;
import java.lang.reflect.Field;

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
                        maybeRegisterHostHandler(activity);
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

    /** Set once the host handler is registered (the first activity create). */
    private boolean hostHandlerRegistered;

    /**
     * Registers the "confirm-identity" host handler the Rust side calls
     * through `host_call` before revealing the stored API key. The
     * runtime's AppActivity keeps its NshView private, so reach it by
     * reflection; the first activity create is the earliest point where
     * the view exists.
     */
    private void maybeRegisterHostHandler(Activity activity) {
        if (hostHandlerRegistered
                || !"com.longbridge.nsh.AppActivity".equals(activity.getClass().getName())) {
            return;
        }
        try {
            Field view = activity.getClass().getDeclaredField("view");
            view.setAccessible(true);
            Object nshView = view.get(activity);
            if (!(nshView instanceof NshView)) {
                return;
            }
            ((NshView) nshView)
                    .handle(
                            "confirm-identity",
                            (data, reply) ->
                                    confirmIdentity(
                                            getApplicationContext(), (NshHostReply) reply));
            hostHandlerRegistered = true;
        } catch (ReflectiveOperationException | ClassCastException e) {
            android.util.Log.w("OqApp", "host handler registration failed", e);
        }
    }

    /**
     * The device-credential gate: resolve on a successful confirmation,
     * reject with a human-readable reason otherwise. BIOMETRIC_WEAK |
     * DEVICE_CREDENTIAL lets the fingerprint or the screen lock pass;
     * no negative button is set (the credential UI provides one).
     */
    private static void confirmIdentity(android.content.Context context, NshHostReply reply) {
        KeyguardManager keyguard = context.getSystemService(KeyguardManager.class);
        if (keyguard == null || !keyguard.isKeyguardSecure()) {
            reply.reject("Set a screen lock first");
            return;
        }
        if (android.os.Build.VERSION.SDK_INT < 30) {
            reply.reject("Needs Android 11+");
            return;
        }
        BiometricPrompt prompt =
                new BiometricPrompt.Builder(context)
                        .setTitle("Reveal the API key")
                        .setSubtitle("Confirm it's you")
                        .setAllowedAuthenticators(
                                BiometricManager.Authenticators.BIOMETRIC_WEAK
                                        | BiometricManager.Authenticators.DEVICE_CREDENTIAL)
                        .build();
        prompt.authenticate(
                new CancellationSignal(),
                context.getMainExecutor(),
                new BiometricPrompt.AuthenticationCallback() {
                    @Override
                    public void onAuthenticationSucceeded(
                            BiometricPrompt.AuthenticationResult result) {
                        reply.resolve(null);
                    }

                    @Override
                    public void onAuthenticationError(int code, CharSequence errString) {
                        reply.reject(errString.toString());
                    }
                });
    }
}
