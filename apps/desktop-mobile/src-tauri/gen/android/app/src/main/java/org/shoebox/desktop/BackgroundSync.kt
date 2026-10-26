package org.shoebox.desktop

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * Background backup (M4-09): WorkManager wakes the app every ~15 minutes when the network is up and
 * the battery is not low; the worker runs a bounded `sync_burst` in the Rust core and, when photos
 * are still at risk and the budget ran out, posts the "finish backing up" notification (at most
 * once a day, decided by the Rust rules in `notify.rs`).
 */
object BackgroundSync {
    const val UNIQUE = "shoebox-background-sync"
    const val CHANNEL = "backup"
    const val SECURITY_CHANNEL = "security"
    private const val BUDGET_SECS = 120

    fun schedule(context: Context) {
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)
            .setRequiresBatteryNotLow(true)
            .build()
        val request = PeriodicWorkRequestBuilder<BurstWorker>(15, TimeUnit.MINUTES).setConstraints(constraints).build()
        WorkManager.getInstance(context).enqueueUniquePeriodicWork(UNIQUE, ExistingPeriodicWorkPolicy.KEEP, request)
    }

    /** Run one burst now (used by tests and diagnostics: `adb shell am broadcast -a ...BURST_NOW`). */
    fun runNow(context: Context) {
        WorkManager.getInstance(context).enqueue(OneTimeWorkRequestBuilder<BurstWorker>().build())
    }

    fun budgetSecs() = BUDGET_SECS
}

class BurstWorker(private val ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        // While the app is open its own core does this work; do not start a second one.
        if (ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) {
            Log.i("ShoeboxBurst", "skipped: the app is in the foreground")
            return Result.success()
        }
        val prefs = ctx.getSharedPreferences("burst", Context.MODE_PRIVATE)
        val last = prefs.getLong("last_notified_ms", 0L)
        val report = try {
            NativeBridge.runBurst(ctx.dataDir.absolutePath, BackgroundSync.budgetSecs(), PhotoBackend(ctx), last)
        } catch (e: Throwable) {
            Log.e("ShoeboxBurst", "burst failed", e)
            return Result.retry()
        }
        Log.i("ShoeboxBurst", report)
        prefs.edit().putString("last_report", report).apply()
        val json = JSONObject(report)
        val n = json.optJSONObject("notification")
        if (n != null) {
            notify(n.getString("title"), n.getString("body"))
            prefs.edit().putLong("last_notified_ms", System.currentTimeMillis()).apply()
        }
        // Security alerts (a new admin, a replaced recovery phrase): each once, on their own channel.
        val security = json.optJSONArray("security")
        if (security != null) {
            for (i in 0 until security.length()) {
                val s = security.getJSONObject(i)
                notify(s.getString("title"), s.getString("body"), security = true, id = 100 + i)
            }
        }
        return Result.success()
    }

    private fun notify(title: String, body: String, security: Boolean = false, id: Int = 1) {
        val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val channel = if (security) BackgroundSync.SECURITY_CHANNEL else BackgroundSync.CHANNEL
        nm.createNotificationChannel(
            if (security) NotificationChannel(channel, "Security", NotificationManager.IMPORTANCE_HIGH)
            else NotificationChannel(channel, "Backup", NotificationManager.IMPORTANCE_DEFAULT)
        )
        val open = PendingIntent.getActivity(
            ctx, 0, Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(ctx, channel)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle(title)
            .setContentText(body)
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        nm.notify(id, n)
    }
}
