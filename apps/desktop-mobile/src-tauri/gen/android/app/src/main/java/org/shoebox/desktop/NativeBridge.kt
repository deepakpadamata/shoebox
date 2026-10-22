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
}
