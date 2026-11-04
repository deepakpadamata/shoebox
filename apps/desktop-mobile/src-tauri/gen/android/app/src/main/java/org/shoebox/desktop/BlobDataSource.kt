package org.shoebox.desktop

import android.media.MediaDataSource
import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.PlaybackException
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSourceException
import androidx.media3.datasource.DataSpec
import java.io.IOException

/** How much one read from the app core asks for. */
private const val PIECE = 1 shl 20

private fun piece(blob: String, position: Long, length: Int): ByteArray =
    NativeBridge.readBlob(blob, position, length) ?: throw IOException("cannot read the video")

/**
 * A video held in Shoebox's blob store, for the player (R1-16): every read goes to the app core
 * (`video.rs`), which checks the bytes against the blob's hash. Nothing is copied to a file and no
 * port is opened.
 */
@UnstableApi
class BlobDataSource(private val blob: String, private val size: Long) : BaseDataSource(false) {
    private var uri: Uri? = null
    private var position = 0L
    private var remaining = 0L
    private var buffer = ByteArray(0)
    private var bufferStart = 0L

    override fun open(dataSpec: DataSpec): Long {
        transferInitializing(dataSpec)
        if (dataSpec.position > size) throw DataSourceException(PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE)
        uri = dataSpec.uri
        position = dataSpec.position
        remaining = if (dataSpec.length == C.LENGTH_UNSET.toLong()) size - position else minOf(dataSpec.length, size - position)
        transferStarted(dataSpec)
        return remaining
    }

    override fun read(target: ByteArray, offset: Int, length: Int): Int {
        if (length == 0) return 0
        if (remaining == 0L) return C.RESULT_END_OF_INPUT
        if (position < bufferStart || position >= bufferStart + buffer.size) {
            buffer = piece(blob, position, minOf(PIECE.toLong(), size - position).toInt())
            bufferStart = position
            if (buffer.isEmpty()) return C.RESULT_END_OF_INPUT
        }
        val from = (position - bufferStart).toInt()
        val n = minOf(length.toLong(), remaining, (buffer.size - from).toLong()).toInt()
        System.arraycopy(buffer, from, target, offset, n)
        position += n
        remaining -= n
        bytesTransferred(n)
        return n
    }

    override fun getUri(): Uri? = uri

    override fun close() {
        if (uri != null) {
            uri = null
            transferEnded()
        }
    }
}

/** The same reads for the system's frame grabber (poster frames of videos not in the gallery). */
class BlobMediaDataSource(private val blob: String, private val total: Long) : MediaDataSource() {
    override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
        if (position >= total) return -1
        val bytes = piece(blob, position, minOf(size.toLong(), total - position).toInt())
        System.arraycopy(bytes, 0, buffer, offset, bytes.size)
        return bytes.size
    }

    override fun getSize(): Long = total

    override fun close() {}
}
