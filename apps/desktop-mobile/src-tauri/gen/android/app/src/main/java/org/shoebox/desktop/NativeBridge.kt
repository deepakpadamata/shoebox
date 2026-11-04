package org.shoebox.desktop

/** The headless entry into the Rust app core (see `android_bridge.rs`). */
object NativeBridge {
    init {
        System.loadLibrary("desktop_mobile")
    }

    /**
     * Back up new camera-roll photos and sync for at most `budgetSecs`, without any UI. Returns a
     * JSON report: `{"skipped": "..."}` if the app itself is running, otherwise the backup and
     * burst outcomes plus an optional `notification`. `lastNotifiedMs` feeds the once-a-day rule.
     */
    @JvmStatic
    external fun runBurst(dataDir: String, budgetSecs: Int, backend: PhotoBackend, lastNotifiedMs: Long): String

    /**
     * Index this phone's photos for search by meaning for at most `budgetSecs` (S1-11), fetching
     * the image model from a desktop or keeper the first time. Returns a JSON report.
     */
    @JvmStatic
    external fun runIndex(dataDir: String, budgetSecs: Int, backend: PhotoBackend): String

    /**
     * Hand Rust the decoder it reads HEIC and AVIF with (`HeifDecoder`): call it once per process,
     * before anything imports or shows a picture. Calling it again changes nothing.
     */
    @JvmStatic
    external fun registerImageDecoder(decoder: HeifDecoder)

    /**
     * Bytes `offset` to `offset + length` of a blob in the open app's store, checked against its
     * hash (`video.rs`); fewer at its end, `null` when it cannot be read.
     */
    @JvmStatic
    external fun readBlob(hash: String, offset: Long, length: Int): ByteArray?
}
