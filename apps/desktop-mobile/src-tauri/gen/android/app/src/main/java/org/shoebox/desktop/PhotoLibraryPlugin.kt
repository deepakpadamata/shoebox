package org.shoebox.desktop

import android.Manifest
import android.app.Activity
import android.content.ClipData
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject
import java.io.File

/**
 * The Tauri face of [PhotoBackend] while the app is open (M4-04): the system permission prompt
 * needs an Activity, everything else is delegated. The Rust side is `android_photos.rs`.
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
    private val backend = PhotoBackend(activity.applicationContext)

    private fun json(s: String) = JSObject(s)

    private inline fun guarded(invoke: Invoke, what: String, block: () -> JSObject) {
        try {
            invoke.resolve(block())
        } catch (e: Exception) {
            invoke.reject("$what: ${e.message}")
        }
    }

    @Command
    fun permission(invoke: Invoke) = guarded(invoke, "permission") { JSObject().put("state", backend.permission()) }

    @Command
    fun requestPermission(invoke: Invoke) {
        if (backend.permission() == "granted") {
            invoke.resolve(JSObject().put("state", "granted"))
            return
        }
        requestPermissionForAlias(if (Build.VERSION.SDK_INT >= 33) "media" else "legacy", invoke, "permissionDone")
    }

    @PermissionCallback
    fun permissionDone(invoke: Invoke) {
        invoke.resolve(JSObject().put("state", backend.permission()))
    }

    @Command
    fun scan(invoke: Invoke) = guarded(invoke, "could not read the camera roll") { json(backend.scanJson(invoke.getArgs().getString("token"))) }

    @Command
    fun pickMedia(invoke: Invoke) {
        val files = invoke.getArgs().getString("source") == "files"
        try {
            startActivityForResult(invoke, if (files) backend.filesIntent() else backend.galleryIntent(), "picked")
        } catch (e: Exception) {
            invoke.reject("cannot open the picker: ${e.message}")
        }
    }

    @ActivityCallback
    fun picked(invoke: Invoke, result: ActivityResult) {
        val uris = if (result.resultCode == Activity.RESULT_OK) backend.pickedUris(result.data) else emptyList()
        // Looking each one up reads MediaStore: off the main thread.
        Thread {
            try {
                val items = JSArray()
                uris.forEach { items.put(backend.resolvePicked(it)) }
                val out = JSObject()
                out.put("items", items)
                invoke.resolve(out)
            } catch (e: Exception) {
                invoke.reject("cannot read what was chosen: ${e.message}")
            }
        }.start()
    }

    @Command
    fun readPicked(invoke: Invoke) = guarded(invoke, "cannot read the chosen file") {
        JSObject().put("path", backend.readPicked(invoke.getArgs().getString("uri") ?: throw IllegalArgumentException("missing uri")))
    }

    /** The system share sheet for files the app core wrote to its cache (`share.rs`). */
    @Command
    fun shareFiles(invoke: Invoke) = guarded(invoke, "cannot share") {
        val paths = invoke.getArgs().getJSONArray("paths")
        val uris = ArrayList<Uri>()
        for (i in 0 until paths.length()) {
            uris.add(FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", File(paths.getString(i))))
        }
        require(uris.isNotEmpty()) { "nothing to share" }
        val types = uris.map { MimeTypeMap.getSingleton().getMimeTypeFromExtension(MimeTypeMap.getFileExtensionFromUrl(it.toString())) ?: "*/*" }.toSet()
        val type = when {
            types.size == 1 -> types.first()
            types.all { it.startsWith("image/") } -> "image/*"
            types.all { it.startsWith("video/") } -> "video/*"
            else -> "*/*"
        }
        val send = if (uris.size == 1) {
            Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, uris[0])
        } else {
            Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
        }
        send.setType(type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        // The chooser passes the read grant on only for URIs in the clip data.
        send.clipData = ClipData.newRawUri(null, uris[0]).apply { uris.drop(1).forEach { addItem(ClipData.Item(it)) } }
        activity.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
        JSObject()
    }

    @Command
    fun conditions(invoke: Invoke) = guarded(invoke, "conditions") { json(backend.conditionsJson()) }

    @Command
    fun secretLoad(invoke: Invoke) = guarded(invoke, "cannot read the secret") {
        val hex = backend.secretLoad(invoke.getArgs().getString("name") ?: throw IllegalArgumentException("missing name"))
        if (hex.isEmpty()) JSObject() else JSObject().put("hex", hex)
    }

    @Command
    fun secretStore(invoke: Invoke) = guarded(invoke, "cannot store the secret") {
        backend.secretStore(
            invoke.getArgs().getString("name") ?: throw IllegalArgumentException("missing name"),
            invoke.getArgs().getString("hex") ?: throw IllegalArgumentException("missing value"),
        )
        JSObject()
    }
}
