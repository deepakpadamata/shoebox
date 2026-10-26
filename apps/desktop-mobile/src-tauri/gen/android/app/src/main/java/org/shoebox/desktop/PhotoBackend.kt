package org.shoebox.desktop

import android.Manifest
import android.content.ContentUris
import android.provider.DocumentsContract
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.database.Cursor
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.ext.SdkExtensions
import android.os.BatteryManager
import android.os.Build
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Everything the Rust side needs from Android that does not need an Activity: the camera roll
 * (MediaStore), device conditions and the Keystore-wrapped secrets. Used by the Tauri plugin while
 * the app is open and, through JNI, by the background worker (`BurstWorker`) when it is not.
 * Results are JSON strings so the native side stays simple.
 */
class PhotoBackend(private val context: Context) {
    private val filesUri = MediaStore.Files.getContentUri("external")

    private fun granted(p: String) =
        ContextCompat.checkSelfPermission(context, p) == PackageManager.PERMISSION_GRANTED

    /** `granted`, `limited` (the user picked some photos) or `denied`. */
    fun permission(): String = when {
        Build.VERSION.SDK_INT >= 33 ->
            if (granted(Manifest.permission.READ_MEDIA_IMAGES) || granted(Manifest.permission.READ_MEDIA_VIDEO)) "granted"
            else if (Build.VERSION.SDK_INT >= 34 && granted(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)) "limited"
            else "denied"
        else -> if (granted(Manifest.permission.READ_EXTERNAL_STORAGE)) "granted" else "denied"
    }

    /** New or changed photos and videos since `token` (a MediaStore generation; null = all). */
    fun scanJson(token: String?): String {
        val since = token?.toLongOrNull() ?: 0L
        val useGeneration = Build.VERSION.SDK_INT >= 30
        val genCol = if (useGeneration) MediaStore.MediaColumns.GENERATION_MODIFIED else MediaStore.MediaColumns.DATE_MODIFIED
        val projection = arrayOf(
            MediaStore.MediaColumns._ID, MediaStore.MediaColumns.MIME_TYPE, MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.DATE_MODIFIED, MediaStore.MediaColumns.DATA, genCol,
        )
        val selection = "(${MediaStore.Files.FileColumns.MEDIA_TYPE} = ? OR ${MediaStore.Files.FileColumns.MEDIA_TYPE} = ?) AND $genCol > ?"
        val args = arrayOf(
            MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE.toString(),
            MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO.toString(),
            since.toString(),
        )
        val assets = JSONArray()
        var newest = since
        context.contentResolver.query(filesUri, projection, selection, args, "$genCol ASC")?.use { c ->
            val id = c.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
            val mime = c.getColumnIndexOrThrow(MediaStore.MediaColumns.MIME_TYPE)
            val size = c.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
            val mod = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
            val data = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATA)
            val gen = c.getColumnIndexOrThrow(genCol)
            while (c.moveToNext()) {
                assets.put(assetJson(c, id, mime, size, mod, data))
                newest = maxOf(newest, c.getLong(gen))
            }
        }
        return JSONObject().put("assets", assets).put("removed", JSONArray()).put("token", newest.toString()).toString()
    }

    /** One asset as the Rust side reads it. `path` only when the file can be read where it is:
     *  the library references gallery files and never copies them (A-092). Read through the file
     *  path, a photo keeps its location when ACCESS_MEDIA_LOCATION is granted. */
    private fun assetJson(c: Cursor, id: Int, mime: Int, size: Int, mod: Int, data: Int): JSONObject {
        val o = JSONObject()
            .put("id", c.getLong(id).toString())
            .put("modified_ms", c.getLong(mod) * 1000)
            .put("size", c.getLong(size))
            .put("mime", c.getString(mime) ?: "application/octet-stream")
        val path = c.getString(data)
        if (path != null && File(path).canRead()) o.put("path", path)
        return o
    }

    /** The gallery asset `mediaId`, or null when MediaStore does not show it to this app. */
    private fun assetById(mediaId: Long): JSONObject? {
        val projection = arrayOf(
            MediaStore.MediaColumns._ID, MediaStore.MediaColumns.MIME_TYPE, MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.DATE_MODIFIED, MediaStore.MediaColumns.DATA,
        )
        context.contentResolver.query(filesUri, projection, "${MediaStore.MediaColumns._ID} = ?", arrayOf(mediaId.toString()), null)?.use { c ->
            if (!c.moveToFirst()) return null
            return assetJson(
                c,
                c.getColumnIndexOrThrow(MediaStore.MediaColumns._ID),
                c.getColumnIndexOrThrow(MediaStore.MediaColumns.MIME_TYPE),
                c.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE),
                c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED),
                c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATA),
            )
        }
        return null
    }

    private fun displayName(uri: Uri): String? = try {
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
    } catch (_: Exception) {
        null
    }

    /** The gallery (MediaStore) id behind a picked URI, if it is a gallery item on this phone. */
    private fun mediaIdOf(uri: Uri): Long? {
        val segments = uri.pathSegments
        if (uri.authority == MediaStore.AUTHORITY && segments.firstOrNull()?.startsWith("picker") == true) {
            // The photo picker: content://media/picker/<user>/<provider>/media/<id>. Items from
            // this phone (the local provider) carry their MediaStore id; cloud items do not.
            if (segments.none { it.endsWith(".photopicker") || it.endsWith(".media") }) return null
            return uri.lastPathSegment?.toLongOrNull()
        }
        if (Build.VERSION.SDK_INT >= 29 && DocumentsContract.isDocumentUri(context, uri)) {
            return try { MediaStore.getMediaUri(context, uri)?.let { ContentUris.parseId(it) } } catch (_: Exception) { null }
        }
        if (uri.authority == MediaStore.AUTHORITY) return try { ContentUris.parseId(uri) } catch (_: Exception) { null }
        return null
    }

    /** What a picked URI is to the library: a gallery photo read in place, a gallery photo this app
     *  may not read in place (not added: it would be a second copy), or a file from elsewhere to
     *  copy (the phone keeps no other copy of it). */
    fun resolvePicked(uri: Uri): JSONObject {
        val name = displayName(uri) ?: "a photo"
        val mediaId = mediaIdOf(uri)
        if (mediaId != null) {
            val asset = assetById(mediaId)
            // The same file, checked by name: the picker's ids are not a documented promise.
            val sameName = asset != null && asset.optString("path").substringAfterLast('/') == name
            if (asset != null && asset.has("path") && (sameName || !uri.toString().contains("picker"))) {
                return JSONObject().put("in_place", asset)
            }
            return JSONObject().put("unreadable", name)
        }
        return JSONObject().put("copy", uri.toString())
    }

    /** The system photo picker: any album, and no media permission needed. Falls back to the
     *  document picker on phones without it. At most [MediaStore.getPickImagesMaxLimit] per pick. */
    fun galleryIntent(): Intent =
        if (Build.VERSION.SDK_INT >= 33 ||
            (Build.VERSION.SDK_INT >= 30 && SdkExtensions.getExtensionVersion(Build.VERSION_CODES.R) >= 2)
        ) {
            Intent(MediaStore.ACTION_PICK_IMAGES).putExtra(MediaStore.EXTRA_PICK_IMAGES_MAX, MediaStore.getPickImagesMaxLimit())
        } else {
            filesIntent()
        }

    /** The document picker: Downloads, cloud drives, a USB stick... */
    fun filesIntent(): Intent =
        Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
            .putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("image/*", "video/*"))
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)

    /** The content URIs a picker returned (one, or several in the clip data). */
    fun pickedUris(data: Intent?): List<Uri> {
        val clip = data?.clipData ?: return listOfNotNull(data?.data)
        return (0 until clip.itemCount).map { clip.getItemAt(it).uri }
    }

    /** Copy a picked file from elsewhere (see [resolvePicked]) to the cache, keeping its name (the
     *  import looks at the extension), and return the path. The caller deletes it. */
    fun readPicked(uri: String): String {
        val u = Uri.parse(uri)
        val name = displayName(u) ?: "picked"
        val safe = name.replace(Regex("[^A-Za-z0-9._-]"), "_").takeLast(80)
        val dir = File(context.cacheDir, "picked").apply { mkdirs() }
        val out = File.createTempFile("p", "-$safe", dir)
        val input = context.contentResolver.openInputStream(u) ?: throw IllegalStateException("cannot open $uri")
        input.use { i -> out.outputStream().use { o -> i.copyTo(o) } }
        return out.absolutePath
    }

    /** Wi-Fi (an unmetered connection), charging and the battery level. */
    fun conditionsJson(): String {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val caps = cm.getNetworkCapabilities(cm.activeNetwork)
        val wifi = caps != null && (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET))
        val unmetered = caps != null && !cm.isActiveNetworkMetered
        val battery: Intent? = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, 100) ?: 100
        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val status = battery?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        val charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
        return JSONObject()
            .put("on_wifi", wifi || unmetered)
            .put("charging", charging)
            .put("battery_pct", if (scale > 0) level * 100 / scale else 100)
            .toString()
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
        File(File(context.filesDir, "keystore").apply { mkdirs() }, "$name.bin")

    /** The secret `name` as hex, or "" if there is none. Stored only as ciphertext (IV ++ AES-GCM). */
    fun secretLoad(name: String): String {
        val f = secretFile(name)
        if (!f.exists()) return ""
        val blob = f.readBytes()
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, wrappingKey(), GCMParameterSpec(128, blob.copyOfRange(0, 12)))
        return c.doFinal(blob, 12, blob.size - 12).joinToString("") { "%02x".format(it) }
    }

    fun secretStore(name: String, hex: String) {
        val plain = ByteArray(hex.length / 2) { hex.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, wrappingKey())
        secretFile(name).writeBytes(c.iv + c.doFinal(plain))
    }
}
