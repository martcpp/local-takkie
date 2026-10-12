package com.martcpp.takkie

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.view.KeyEvent
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

private const val CHANNEL = "radio"
private const val NOTIFICATION = 1
private const val ACTION_STOP = "com.martcpp.takkie.STOP"

// Without a foreground service Android mutes the microphone a few seconds
// after the screen goes off, stops playback about a minute later, and
// suspends the multicast lock, all without an error.
class RadioService : Service() {
  private var lock: WifiManager.MulticastLock? = null
  private var session: MediaSession? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      ServicePlugin.stopRequested()
      stopSelf()
      return START_NOT_STICKY
    }
    getSystemService(NotificationManager::class.java).createNotificationChannel(
      NotificationChannel(CHANNEL, getString(R.string.radio_channel), NotificationManager.IMPORTANCE_LOW)
    )
    val immutable = PendingIntent.FLAG_IMMUTABLE
    val open = PendingIntent.getActivity(
      this, 0, packageManager.getLaunchIntentForPackage(packageName), immutable
    )
    val stop = PendingIntent.getService(
      this, 0, Intent(this, RadioService::class.java).setAction(ACTION_STOP), immutable
    )
    val notification = NotificationCompat.Builder(this, CHANNEL)
      .setContentTitle(getString(R.string.radio_on_title))
      .setContentText(getString(R.string.radio_on_text))
      .setSmallIcon(R.drawable.ic_stat_radio)
      .setContentIntent(open)
      .setOngoing(true)
      .addAction(0, getString(R.string.radio_stop), stop)
      .build()
    ServiceCompat.startForeground(
      this, NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
    )
    if (lock == null) {
      val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
      lock = wifi.createMulticastLock("takkie-service").apply {
        setReferenceCounted(false)
        acquire()
      }
    }
    if (session == null) {
      session = headsetSession()
    }
    // If Android kills the app, the engine is gone too; a restarted service
    // would only show a notification for a radio that isn't running.
    return START_NOT_STICKY
  }

  // Swiped away from the recent apps: turn the radio off, so others see us
  // leave, and don't keep a notification for an app that is gone.
  override fun onTaskRemoved(rootIntent: Intent?) {
    ServicePlugin.stopRequested()
    stopSelf()
    super.onTaskRemoved(rootIntent)
  }

  // With the app in the background or the screen off, headset buttons only
  // reach an app through an active media session.
  private fun headsetSession(): MediaSession {
    val buttons = PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE
    return MediaSession(this, "takkie").apply {
      setCallback(object : MediaSession.Callback() {
        override fun onMediaButtonEvent(intent: Intent): Boolean {
          val event = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(Intent.EXTRA_KEY_EVENT, KeyEvent::class.java)
          } else {
            @Suppress("DEPRECATION")
            intent.getParcelableExtra(Intent.EXTRA_KEY_EVENT)
          }
          return event != null && HeadsetPlugin.onKey(event)
        }
      })
      setPlaybackState(
        PlaybackState.Builder()
          .setState(PlaybackState.STATE_PLAYING, PlaybackState.PLAYBACK_POSITION_UNKNOWN, 1f)
          .setActions(buttons)
          .build()
      )
      isActive = true
    }
  }

  override fun onDestroy() {
    session?.release()
    session = null
    lock?.takeIf { it.isHeld }?.release()
    lock = null
    super.onDestroy()
  }
}

@InvokeArg
class ServiceArgs {
  var on: Boolean = false
}

// Starts and stops RadioService, and tells Rust (mobile.rs) when the user
// taps Stop on the notification.
@TauriPlugin
class ServicePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun setService(invoke: Invoke) {
    val args = invoke.parseArgs(ServiceArgs::class.java)
    val intent = Intent(activity, RadioService::class.java)
    try {
      if (args.on) {
        stopWanted = false
        // A microphone service may only start while the app is on screen.
        ContextCompat.startForegroundService(activity, intent)
      } else {
        activity.stopService(intent)
      }
      invoke.resolve(JSObject().put("running", args.on))
    } catch (error: Exception) {
      invoke.reject(error.message ?: "the service couldn't be changed")
    }
  }

  // Answers only once the user taps Stop, however long that takes.
  @Command
  fun waitForStop(invoke: Invoke) {
    synchronized(Companion) {
      if (stopWanted) {
        stopWanted = false
        invoke.resolve()
      } else {
        waiting = invoke
      }
    }
  }

  companion object {
    private var waiting: Invoke? = null
    private var stopWanted = false

    fun stopRequested() {
      synchronized(this) {
        val asked = waiting
        waiting = null
        if (asked != null) asked.resolve() else stopWanted = true
      }
    }
  }
}
