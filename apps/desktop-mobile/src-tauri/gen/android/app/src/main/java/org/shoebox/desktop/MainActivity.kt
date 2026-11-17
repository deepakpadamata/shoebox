package org.shoebox.desktop

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import java.io.File

class MainActivity : TauriActivity() {
  @Volatile private var top = 0
  @Volatile private var bottom = 0

  // TauriActivity turns this off; the UI keeps a history entry for each open sheet and sub-page
  // (U1-02), so system Back has to reach the WebView's history first.
  override val handleBackNavigation: Boolean = true

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    NativeBridge.registerImageDecoder(HeifDecoder)
    BackgroundSync.schedule(applicationContext)
    Thread { ModelAssets.install(applicationContext) }.start()
  }

  override fun onResume() {
    super.onResume()
    askForNotificationsOnce()
  }

  // Android 13+ denies notifications until the app asks, so without this the background worker's
  // security alerts and backup notices were dropped (found on the S25, 2026-11-17). Asked once,
  // after this device belongs to a library (`app.json`), not over the first-run screens. Only an
  // answer counts: a dialog closed without one (the task brought back by a launch) asks again.
  private var asking = false

  private fun askForNotificationsOnce() {
    if (Build.VERSION.SDK_INT < 33 || asking) return
    if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
    if (!File(applicationInfo.dataDir, "app.json").exists()) return
    if (getSharedPreferences("permissions", MODE_PRIVATE).getBoolean(NOTIFICATIONS_ANSWERED, false)) return
    asking = true
    requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATIONS_REQUEST)
  }

  override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
    super.onRequestPermissionsResult(requestCode, permissions, grantResults)
    if (requestCode != NOTIFICATIONS_REQUEST) return
    asking = false
    if (grantResults.isNotEmpty()) {
      getSharedPreferences("permissions", MODE_PRIVATE).edit().putBoolean(NOTIFICATIONS_ANSWERED, true).apply()
    }
  }

  private companion object {
    const val NOTIFICATIONS_REQUEST = 4701
    const val NOTIFICATIONS_ANSWERED = "notifications_answered"
  }

  // The WebView draws edge to edge but does not report the system bars to CSS: hand the insets
  // (in CSS pixels) to the UI, which turns them into --safe-top / --safe-bottom (src/main.ts).
  override fun onWebViewCreate(webView: WebView) {
    val density = resources.displayMetrics.density
    ViewCompat.setOnApplyWindowInsetsListener(webView) { _, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      top = (bars.top / density).toInt()
      bottom = (bars.bottom / density).toInt()
      insets
    }
    webView.addJavascriptInterface(object {
      @JavascriptInterface fun top() = this@MainActivity.top
      @JavascriptInterface fun bottom() = this@MainActivity.bottom
    }, "ShoeboxInsets")
    webView.addJavascriptInterface(object {
      // Dark status-bar icons on a light theme (src/lib/theme.ts); light icons on a dark one.
      @JavascriptInterface fun setLightBars(light: Boolean) {
        runOnUiThread {
          val c = WindowCompat.getInsetsController(window, window.decorView)
          c.isAppearanceLightStatusBars = light
          c.isAppearanceLightNavigationBars = light
        }
      }
    }, "ShoeboxTheme")
    webView.addJavascriptInterface(object {
      // "Keep the screen on until backup finishes" (mockup 5e).
      @JavascriptInterface fun setKeepAwake(on: Boolean) {
        runOnUiThread {
          if (on) window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
          else window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
      }
    }, "ShoeboxHost")
  }
}
