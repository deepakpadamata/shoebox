package org.shoebox.desktop

import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import androidx.media3.ui.PlayerView
import java.io.File

/**
 * A video in Android's own player over the app (R1-16): a gallery video from its file, where it
 * is; any other from the blob store through [BlobDataSource]. Back closes it; rotation, audio
 * focus and headphones being unplugged are the player's.
 */
@UnstableApi
class PlayerActivity : AppCompatActivity() {
    private var player: ExoPlayer? = null

    companion object {
        fun intent(context: Context, path: String?, blob: String, size: Long): Intent =
            Intent(context, PlayerActivity::class.java)
                .putExtra("path", path)
                .putExtra("blob", blob)
                .putExtra("size", size)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val view = PlayerView(this).apply {
            setBackgroundColor(Color.BLACK)
            keepScreenOn = true
            setShowNextButton(false)
            setShowPreviousButton(false)
        }
        setContentView(view)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, view).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }

        val path = intent.getStringExtra("path")
        val blob = intent.getStringExtra("blob") ?: return finish()
        val size = intent.getLongExtra("size", 0)
        val audio = AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build()
        val p = ExoPlayer.Builder(this)
            .setAudioAttributes(audio, true)
            .setHandleAudioBecomingNoisy(true)
            .build()
        if (path != null) {
            p.setMediaItem(MediaItem.fromUri(Uri.fromFile(File(path))))
        } else {
            val reads = DataSource.Factory { BlobDataSource(blob, size) }
            p.setMediaSource(ProgressiveMediaSource.Factory(reads).createMediaSource(MediaItem.fromUri("shoebox-blob:$blob")))
        }
        p.addListener(object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                Toast.makeText(this@PlayerActivity, "This video cannot be played here", Toast.LENGTH_LONG).show()
                finish()
            }
        })
        view.player = p
        p.playWhenReady = true
        p.prepare()
        player = p
    }

    override fun onStop() {
        super.onStop()
        player?.pause()
    }

    override fun onDestroy() {
        player?.release()
        player = null
        super.onDestroy()
    }
}
