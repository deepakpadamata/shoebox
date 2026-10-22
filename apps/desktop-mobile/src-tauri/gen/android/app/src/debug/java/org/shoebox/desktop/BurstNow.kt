package org.shoebox.desktop

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Debug trigger for one background burst (see the manifest). */
class BurstNow : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        BackgroundSync.runNow(context)
    }
}
