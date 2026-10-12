package com.martcpp.takkie

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.IBinder
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
    // If Android kills the app, the engine is gone too; a restarted service
    // would only show a notification for a radio that isn't running.
    return START_NOT_STICKY
  }

  override fun onDestroy() {
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
