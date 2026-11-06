package org.shoebox.desktop

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.os.BatteryManager
import android.os.Build
import android.os.PowerManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
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
        // Search indexing (S1-11): only while charging, with the battery not low.
        val charging = Constraints.Builder()
            .setRequiresCharging(true)
            .setRequiresBatteryNotLow(true)
            .build()
        val index = PeriodicWorkRequestBuilder<IndexWorker>(30, TimeUnit.MINUTES).setConstraints(charging).build()
        WorkManager.getInstance(context).enqueueUniquePeriodicWork(INDEX_UNIQUE, ExistingPeriodicWorkPolicy.KEEP, index)
    }

    const val INDEX_UNIQUE = "shoebox-index"
    const val INDEX_BUDGET_SECS = 480

    /** Run one burst now (used by tests and diagnostics: `adb shell am broadcast -a ...BURST_NOW`). */
    fun runNow(context: Context) {
        WorkManager.getInstance(context).enqueue(OneTimeWorkRequestBuilder<BurstWorker>().build())
    }

    fun budgetSecs() = BUDGET_SECS

    /** Another indexing run right away (while charging), for a backlog one run could not finish. */
    fun indexAgain(context: Context) {
        val charging = Constraints.Builder().setRequiresCharging(true).setRequiresBatteryNotLow(true).build()
        WorkManager.getInstance(context).enqueueUniqueWork(
            INDEX_UNIQUE + "-again", ExistingWorkPolicy.KEEP,
            OneTimeWorkRequestBuilder<IndexWorker>().setConstraints(charging).build(),
        )
    }
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
            ModelAssets.install(ctx)
            NativeBridge.registerImageDecoder(HeifDecoder)
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

/**
 * Whether the phone is on a network the user does not pay for by the byte (Wi-Fi, or a mobile
 * plan marked unmetered). Search models (about 135 MB) are downloaded only then (S2-02).
 */
fun onUnmeteredNetwork(ctx: Context): Boolean {
    val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager ?: return false
    val caps = cm.getNetworkCapabilities(cm.activeNetwork ?: return false) ?: return false
    return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
        caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
}

/**
 * Indexes this phone's photos for search while it charges (S1-11, S2-04). A run goes on until the
 * phone has nothing left to do (pictures, text, faces, then grouping and shard compaction), the
 * budget is spent, the phone gets too hot or is unplugged. With a big backlog (more than
 * [LONG_BACKLOG] photos, as the last run left it) it is a foreground worker with a progress
 * notification and a budget of hours; with a small one, a short burst. Where Android will not
 * start a foreground service from the background, it falls back to short bursts that follow one
 * another while the phone charges.
 */
class IndexWorker(private val ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        if (ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) {
            return Result.success()
        }
        val dir = ctx.dataDir
        val before = IndexProgress.read(dir)
        val big = before != null && !before.done && before.left > LONG_BACKLOG
        var foreground = false
        if (big) {
            foreground = try {
                setForeground(foregroundInfo(before!!))
                true
            } catch (e: Exception) {
                Log.w("ShoeboxIndex", "no foreground service here: short bursts instead", e)
                false
            }
        }
        val budget = if (foreground) LONG_BUDGET_SECS else BackgroundSync.INDEX_BUDGET_SECS
        val stop = File(dir, "index-stop")
        val report = try {
            coroutineScope {
                val watch = launch(Dispatchers.Default) {
                    while (isActive) {
                        delay(10_000)
                        if (tooHot() || !charging()) {
                            runCatching { stop.writeText("stop") }
                        }
                        if (foreground) IndexProgress.read(dir)?.let { runCatching { setForeground(foregroundInfo(it)) } }
                    }
                }
                try {
                    withContext(Dispatchers.IO) {
                        ModelAssets.install(ctx)
                        NativeBridge.registerImageDecoder(HeifDecoder)
                        NativeBridge.runIndex(dir.absolutePath, budget, onUnmeteredNetwork(ctx), PhotoBackend(ctx))
                    }
                } finally {
                    watch.cancel()
                }
            }
        } catch (e: Throwable) {
            Log.e("ShoeboxIndex", "indexing failed", e)
            return Result.retry()
        }
        Log.i("ShoeboxIndex", report)
        // More to do and still fine to do it: carry on at once rather than in half an hour.
        val after = IndexProgress.read(dir)
        if (!report.contains("\"skipped\"") && after != null && !after.done && after.left > 0 && !isStopped && !tooHot() && charging()) {
            BackgroundSync.indexAgain(ctx)
        }
        return Result.success()
    }

    /** WorkManager cancels the worker (unplugged, constraints lost): the run is in native code and must be told. */
    override fun onStopped() {
        runCatching { File(ctx.dataDir, "index-stop").writeText("stop") }
    }

    private fun foregroundInfo(p: IndexProgress): ForegroundInfo {
        val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(NotificationChannel(INDEX_CHANNEL, "Indexing photos", NotificationManager.IMPORTANCE_LOW))
        val percent = if (p.total > 0) ((p.total - p.left) * 100L / p.total).toInt() else 0
        val open = PendingIntent.getActivity(
            ctx, 0, Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(ctx, INDEX_CHANNEL)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle("Indexing photos for search: $percent%")
            .setContentText("${p.left} left. Runs while the phone charges.")
            .setProgress(100, percent, p.total == 0)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(open)
            .build()
        return if (Build.VERSION.SDK_INT >= 29) {
            ForegroundInfo(INDEX_NOTIFICATION, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            ForegroundInfo(INDEX_NOTIFICATION, n)
        }
    }

    private fun tooHot(): Boolean {
        if (Build.VERSION.SDK_INT < 29) return false
        val pm = ctx.getSystemService(Context.POWER_SERVICE) as PowerManager
        return pm.currentThermalStatus >= PowerManager.THERMAL_STATUS_SEVERE
    }

    private fun charging(): Boolean {
        val i = ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) ?: return true
        return i.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0
    }

    companion object {
        const val LONG_BACKLOG = 500
        const val LONG_BUDGET_SECS = 3 * 60 * 60
        const val INDEX_CHANNEL = "indexing"
        const val INDEX_NOTIFICATION = 7
    }
}

/** What the last indexing run left, as the core writes it to `index-progress.json`. */
data class IndexProgress(val indexed: Int, val total: Int, val left: Int, val done: Boolean) {
    companion object {
        fun read(dir: File): IndexProgress? = try {
            val j = JSONObject(File(dir, "index-progress.json").readText())
            IndexProgress(j.getInt("indexed"), j.getInt("total"), j.getInt("left"), j.getBoolean("done"))
        } catch (e: Exception) {
            null
        }
    }
}
