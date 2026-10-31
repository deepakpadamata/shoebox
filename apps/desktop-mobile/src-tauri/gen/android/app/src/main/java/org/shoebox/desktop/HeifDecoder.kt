package org.shoebox.desktop

import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.graphics.ImageDecoder
import android.os.Build
import java.nio.ByteBuffer

/**
 * HEIC and AVIF through the system's decoder (I1-02). Rust calls [decode] over JNI (see
 * `heic_android.rs`) wherever the app reads a picture it has no decoder of its own for.
 */
object HeifDecoder {
    /**
     * The picture upright (the file's rotation applied) and in sRGB, as a software bitmap whose
     * pixels Rust reads in place and then [Bitmap.recycle]s; `null` when this Android has no
     * decoder for the format (HEIC needs Android 9, AVIF Android 12). A damaged file throws.
     *
     * An instance method on purpose: Rust calls it on the object it was handed, and `@JvmStatic`
     * would leave no such method.
     */
    fun decode(bytes: ByteArray): Bitmap? {
        val needed = if (isAvif(bytes)) Build.VERSION_CODES.S else Build.VERSION_CODES.P
        if (Build.VERSION.SDK_INT < needed) return null
        return ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) { decoder, _, _ ->
            // Software memory, so the pixels can be read; sRGB, so a Display P3 photo from an
            // iPhone is not read as if it were sRGB.
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            decoder.setTargetColorSpace(ColorSpace.get(ColorSpace.Named.SRGB))
        }
    }

    /** An AVIF's `ftyp` box names the brand `avif` or `avis`. */
    private fun isAvif(bytes: ByteArray): Boolean {
        if (bytes.size < 12) return false
        val brand = String(bytes, 8, 4, Charsets.ISO_8859_1)
        return brand == "avif" || brand == "avis"
    }
}
