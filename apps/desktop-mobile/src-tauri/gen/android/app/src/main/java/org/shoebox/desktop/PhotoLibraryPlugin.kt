package org.shoebox.desktop

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.Build
import android.provider.MediaStore
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The camera roll for the backup pipeline (M4-04): permission state, an incremental MediaStore
 * scan driven by MediaStore's generation counters, original bytes as a cache file, and the device
 * conditions (Wi-Fi, charging, battery) the sync rules look at. The Rust side is
 * `android_photos.rs`; the interface is the `PhotoSource` trait.
 */
@TauriPlugin(
    permissions = [
        Permission(
            strings = [
                Manifest.permission.READ_MEDIA_IMAGES,
                Manifest.permission.READ_MEDIA_VIDEO,
                Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED,
                Manifest.permission.ACCESS_MEDIA_LOCATION,
            ],
            alias = "media"
        ),
        Permission(strings = [Manifest.permission.READ_EXTERNAL_STORAGE], alias = "legacy"),
    ]
)
class PhotoLibraryPlugin(private val activity: Activity) : Plugin(activity) {
    private val filesUri = MediaStore.Files.getContentUri("external")

    private fun granted(p: String) =
        ContextCompat.checkSelfPermission(activity, p) == PackageManager.PERMISSION_GRANTED

    /** `granted`, `limited` (the user picked some photos) or `denied`. */
    private fun permissionState(): String = when {
        Build.VERSION.SDK_INT >= 33 ->
            if (granted(Manifest.permission.READ_MEDIA_IMAGES) || granted(Manifest.permission.READ_MEDIA_VIDEO)) "granted"
            else if (Build.VERSION.SDK_INT >= 34 && granted(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)) "limited"
            else "denied"
        else -> if (granted(Manifest.permission.READ_EXTERNAL_STORAGE)) "granted" else "denied"
    }

    @Command
    fun permission(invoke: Invoke) {
        invoke.resolve(JSObject().put("state", permissionState()))
    }

    @Command
    fun requestPermission(invoke: Invoke) {
        if (permissionState() == "granted") {
            invoke.resolve(JSObject().put("state", "granted"))
            return
        }
        requestPermissionForAlias(if (Build.VERSION.SDK_INT >= 33) "media" else "legacy", invoke, "permissionDone")
    }

    @PermissionCallback
    fun permissionDone(invoke: Invoke) {
        invoke.resolve(JSObject().put("state", permissionState()))
    }

    /** New or changed photos and videos since `token` (a MediaStore generation; absent = all). */
    @Command
    fun scan(invoke: Invoke) {
        try {
            val since = invoke.getArgs().getString("token")?.toLongOrNull() ?: 0L
            val resolver = activity.contentResolver
            val useGeneration = Build.VERSION.SDK_INT >= 30
            val genCol = if (useGeneration) MediaStore.MediaColumns.GENERATION_MODIFIED else MediaStore.MediaColumns.DATE_MODIFIED
            val projection = arrayOf(
                MediaStore.MediaColumns._ID, MediaStore.MediaColumns.MIME_TYPE, MediaStore.MediaColumns.SIZE,
                MediaStore.MediaColumns.DATE_MODIFIED, genCol,
            )
            val selection = "(${MediaStore.Files.FileColumns.MEDIA_TYPE} = ? OR ${MediaStore.Files.FileColumns.MEDIA_TYPE} = ?) AND $genCol > ?"
            val args = arrayOf(
                MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE.toString(),
                MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO.toString(),
                since.toString(),
            )
            val assets = JSArray()
            var newest = since
            resolver.query(filesUri, projection, selection, args, "$genCol ASC")?.use { c ->
                val id = c.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
                val mime = c.getColumnIndexOrThrow(MediaStore.MediaColumns.MIME_TYPE)
                val size = c.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
                val mod = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
                val gen = c.getColumnIndexOrThrow(genCol)
                while (c.moveToNext()) {
                    assets.put(
                        JSObject()
                            .put("id", c.getLong(id).toString())
                            .put("modified_ms", c.getLong(mod) * 1000)
                            .put("size", c.getLong(size))
                            .put("mime", c.getString(mime) ?: "application/octet-stream")
                    )
                    newest = maxOf(newest, c.getLong(gen))
                }
            }
            invoke.resolve(JSObject().put("assets", assets).put("removed", JSArray()).put("token", newest.toString()))
        } catch (e: Exception) {
            invoke.reject("could not read the camera roll: ${e.message}")
        }
    }

    /** Copy the original (with GPS when allowed) to the cache and return its path. */
    @Command
    fun readOriginal(invoke: Invoke) {
        try {
            val id = invoke.getArgs().getString("id")?.toLongOrNull() ?: return invoke.reject("missing id")
            var uri = ContentUris.withAppendedId(filesUri, id)
            if (Build.VERSION.SDK_INT >= 29 && granted(Manifest.permission.ACCESS_MEDIA_LOCATION)) {
                try { uri = MediaStore.setRequireOriginal(uri) } catch (_: Exception) {}
            }
            val dir = File(activity.cacheDir, "backup").apply { mkdirs() }
            val out = File(dir, "asset-$id")
            val input = activity.contentResolver.openInputStream(uri) ?: return invoke.reject("cannot open asset $id")
            input.use { i -> out.outputStream().use { o -> i.copyTo(o) } }
            invoke.resolve(JSObject().put("path", out.absolutePath))
        } catch (e: Exception) {
            invoke.reject("cannot read asset: ${e.message}")
        }
    }

    // ---- device secrets (M4-02): wrapped by a non-exportable Android Keystore key ----

    private fun wrappingKey(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey("shoebox-wrap", null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder("shoebox-wrap", KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return gen.generateKey()
    }

    private fun secretFile(name: String) =
        File(File(activity.filesDir, "keystore").apply { mkdirs() }, "$name.bin")

    /** The secret called `name` as hex, or absent. Stored only as ciphertext (IV ++ AES-GCM). */
    @Command
    fun secretLoad(invoke: Invoke) {
        try {
            val name = invoke.getArgs().getString("name") ?: return invoke.reject("missing name")
            val f = secretFile(name)
            if (!f.exists()) return invoke.resolve(JSObject())
            val blob = f.readBytes()
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.DECRYPT_MODE, wrappingKey(), GCMParameterSpec(128, blob.copyOfRange(0, 12)))
            val plain = c.doFinal(blob, 12, blob.size - 12)
            invoke.resolve(JSObject().put("hex", plain.joinToString("") { "%02x".format(it) }))
        } catch (e: Exception) {
            invoke.reject("cannot read the secret: ${e.message}")
        }
    }

    @Command
    fun secretStore(invoke: Invoke) {
        try {
            val name = invoke.getArgs().getString("name") ?: return invoke.reject("missing name")
            val hex = invoke.getArgs().getString("hex") ?: return invoke.reject("missing value")
            val plain = ByteArray(hex.length / 2) { hex.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.ENCRYPT_MODE, wrappingKey())
            secretFile(name).writeBytes(c.iv + c.doFinal(plain))
            invoke.resolve(JSObject())
        } catch (e: Exception) {
            invoke.reject("cannot store the secret: ${e.message}")
        }
    }

    @Command
    fun conditions(invoke: Invoke) {
        val cm = activity.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val caps = cm.getNetworkCapabilities(cm.activeNetwork)
        // "Wi-Fi" means an unmetered connection (Wi-Fi or Ethernet), not a mobile data plan.
        val wifi = caps != null && (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET))
        val unmetered = caps != null && !cm.isActiveNetworkMetered
        val battery: Intent? = activity.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, 100) ?: 100
        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val status = battery?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
        invoke.resolve(
            JSObject()
                .put("on_wifi", wifi || unmetered)
                .put("charging", charging)
                .put("battery_pct", if (scale > 0) level * 100 / scale else 100)
        )
    }
}
