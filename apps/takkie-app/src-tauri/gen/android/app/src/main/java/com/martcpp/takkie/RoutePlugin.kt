package com.martcpp.takkie

import android.app.Activity
import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

// Where sound comes out. Android sends media to a connected headset by
// itself, and to the loudspeaker otherwise; this only reports which.
// Called from Rust (mobile.rs).
@TauriPlugin
class RoutePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun state(invoke: Invoke) {
    val audio = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    val outputs = audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).map { it.type }
    val route = when {
      outputs.any { it in BLUETOOTH } -> "bluetooth"
      outputs.any { it in WIRED } -> "wired"
      else -> "speaker"
    }
    invoke.resolve(JSObject().put("route", route))
  }

  companion object {
    // TYPE_BLE_HEADSET is 26; named only from Android 12.
    private val BLUETOOTH = setOf(
      AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
      AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
      26,
    )
    private val WIRED = setOf(
      AudioDeviceInfo.TYPE_WIRED_HEADSET,
      AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
      AudioDeviceInfo.TYPE_USB_HEADSET,
    )
  }
}
