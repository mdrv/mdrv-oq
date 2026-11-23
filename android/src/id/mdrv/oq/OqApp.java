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
    private final Handler main = new Handler(Looper.getMainLooper());
    private int hostRetriesLeft = 20;

    @Override
    public void onCreate() {
        super.onCreate();
        android.util.Log.i("OqApp", "app onCreate");
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
                        main.postDelayed(attempt[0], delay[0]);
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
                        maybeRegisterHostHandler(activity);
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
     * reflection. On some builds the create dispatch still sees a null
     * view, so a null schedules retries on the main thread instead of
     * giving up: the Rust call has no timeout, and an unregistered
     * handler would wedge the Reveal UI.
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
                if (hostRetriesLeft > 0) {
                    hostRetriesLeft--;
                    if (hostRetriesLeft == 19) {
                        android.util.Log.w("OqApp",
                                "host handler registration: the activity field is "
                                        + nshView + ", retrying");
                    }
                    main.postDelayed(
                            () -> maybeRegisterHostHandler(activity), 100);
                } else {
                    android.util.Log.w("OqApp",
                            "host handler registration: the view never appeared");
                }
                return;
            }
            ((NshView) nshView)
                    .handle(
                            "confirm-identity",
                            (data, reply) -> {
                                try {
                                    android.util.Log.i(
                                            "OqApp", "confirm-identity call received");
                                    confirmIdentity(
                                            getApplicationContext(),
                                            (NshHostReply) reply);
                                } catch (Throwable revealFailure) {
                                    // The Rust side has no timeout on the call:
                                    // every path must answer or the UI waits
                                    // forever.
                                    android.util.Log.w(
                                            "OqApp", "confirm-identity failed",
                                            revealFailure);
                                    reply.reject(
                                            "Reveal failed: " + revealFailure);
                                }
                            });
            hostHandlerRegistered = true;
            android.util.Log.i("OqApp", "confirm-identity handler registered");
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
        if (android.os.Build.VERSION.SDK_INT < 30) {
            reply.reject("Needs Android 11+");
            return;
        }
        BiometricManager biometrics = context.getSystemService(BiometricManager.class);
        if (biometrics == null) {
            reply.reject("No biometric service on this device");
            return;
        }
        int can = biometrics.canAuthenticate(
                BiometricManager.Authenticators.BIOMETRIC_WEAK
                        | BiometricManager.Authenticators.DEVICE_CREDENTIAL);
        android.util.Log.i("OqApp", "canAuthenticate: " + can);
        if (can != BiometricManager.BIOMETRIC_SUCCESS) {
            String reason;
            if (can == BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE
                    // with DEVICE_CREDENTIAL requested, no screen lock lands here
                    || can == BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED) {
                reason = "Set a screen lock, or enroll a fingerprint first";
            } else if (can == BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE) {
                reason = "The biometric hardware is busy, try again";
            } else {
                reason = "This device cannot confirm identity (" + can + ")";
            }
            reply.reject(reason);
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
        android.util.Log.i("OqApp", "authenticating");
        prompt.authenticate(
                new CancellationSignal(),
                context.getMainExecutor(),
                new BiometricPrompt.AuthenticationCallback() {
                    @Override
                    public void onAuthenticationSucceeded(
                            BiometricPrompt.AuthenticationResult result) {
                        android.util.Log.i("OqApp", "confirmed identity");
                        reply.resolve(null);
                    }

                    @Override
                    public void onAuthenticationError(int code, CharSequence errString) {
                        android.util.Log.w("OqApp", "prompt error " + code + ": " + errString);
                        reply.reject(errString.toString());
                    }
                });
    }
}
