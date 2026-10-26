package org.shoebox.desktop

import android.Manifest
import android.app.Activity
import android.os.Build
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
    fun readOriginal(invoke: Invoke) = guarded(invoke, "cannot read asset") {
        JSObject().put("path", backend.readOriginal(invoke.getArgs().getString("id") ?: throw IllegalArgumentException("missing id")))
    }

    /** Let the user choose photos and videos: `source` "gallery" (the photo picker) or "files"
     *  (the document picker). Resolves with the chosen content URIs, none when cancelled. */
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
        val uris = JSArray()
        if (result.resultCode == Activity.RESULT_OK) backend.pickedUris(result.data).forEach { uris.put(it) }
        val out = JSObject()
        out.put("uris", uris)
        invoke.resolve(out)
    }

    @Command
    fun readPicked(invoke: Invoke) = guarded(invoke, "cannot read the chosen file") {
        JSObject().put("path", backend.readPicked(invoke.getArgs().getString("uri") ?: throw IllegalArgumentException("missing uri")))
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
