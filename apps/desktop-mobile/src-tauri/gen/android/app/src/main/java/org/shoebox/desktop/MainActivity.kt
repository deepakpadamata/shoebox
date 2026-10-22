package org.shoebox.desktop

import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  @Volatile private var top = 0
  @Volatile private var bottom = 0

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
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
