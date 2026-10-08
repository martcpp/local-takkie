package com.martcpp.takkiespike

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
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

// Without a foreground service Android cuts the app off from the mic, and
// may freeze it, soon after the screen goes off.
class TalkService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val manager = getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(
      NotificationChannel(CHANNEL, "Walkie-talkie", NotificationManager.IMPORTANCE_LOW)
    )
    val notification = NotificationCompat.Builder(this, CHANNEL)
      .setContentTitle("takkie is on")
      .setContentText("Listening on the local network")
      .setSmallIcon(android.R.drawable.ic_btn_speak_now)
      .setOngoing(true)
      .build()
    ServiceCompat.startForeground(
      this, 1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
    )
    return START_NOT_STICKY
  }

  companion object {
    const val CHANNEL = "talk"
  }
}

@InvokeArg
class ServiceArgs {
  var on: Boolean = false
}

@TauriPlugin
class ServicePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun setService(invoke: Invoke) {
    val args = invoke.parseArgs(ServiceArgs::class.java)
    val intent = Intent(activity, TalkService::class.java)
    // A microphone service can only start while the app is on screen.
    if (args.on) ContextCompat.startForegroundService(activity, intent)
    else activity.stopService(intent)
    invoke.resolve(JSObject().put("running", args.on))
  }
}
