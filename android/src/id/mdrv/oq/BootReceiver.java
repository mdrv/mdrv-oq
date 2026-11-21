// Starts the quota service after boot. BOOT_COMPLETED is on Android's
// foreground-service background-start allowlist, so this is allowed even
// for the specialUse type.

package id.mdrv.oq;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        if (Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction())) {
            context.startForegroundService(
                    new Intent(context, QuotaService.class));
        }
    }
}
