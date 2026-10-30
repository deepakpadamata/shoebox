package org.shoebox.desktop

import android.content.Context
import java.io.File

/**
 * The search text model ships inside the APK (assets/models, put there by the release build) and
 * is copied once into the data folder, where the Rust side finds and checks it by hash (S1-08).
 * A build without the asset (development) simply has no search by meaning until it is there.
 */
object ModelAssets {
    @Volatile private var done = false

    fun install(ctx: Context) {
        if (done) return
        synchronized(this) {
            if (done) return
            val names = try { ctx.assets.list("models") ?: emptyArray() } catch (e: Exception) { emptyArray() }
            val dir = File(ctx.dataDir, "models").apply { mkdirs() }
            for (name in names) {
                val out = File(dir, name)
                val size = try {
                    ctx.assets.openFd("models/$name").use { it.length }
                } catch (e: Exception) {
                    -1L // compressed asset: size unknown, compare after copying
                }
                if (out.exists() && (size < 0 || out.length() == size)) continue
                val part = File(dir, "$name.part")
                ctx.assets.open("models/$name").use { input ->
                    part.outputStream().use { input.copyTo(it, 1 shl 20) }
                }
                part.renameTo(out)
            }
            done = true
        }
    }
}
