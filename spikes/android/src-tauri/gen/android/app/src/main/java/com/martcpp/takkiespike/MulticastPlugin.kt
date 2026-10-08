package com.martcpp.takkiespike

import android.app.Activity
import android.content.Context
import android.net.wifi.WifiManager
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class LockArgs {
  var on: Boolean = false
}

// Android's Wi-Fi driver may drop incoming multicast, mDNS included, unless
// an app holds this lock.
@TauriPlugin
class MulticastPlugin(private val activity: Activity) : Plugin(activity) {
  private val lock: WifiManager.MulticastLock by lazy {
    val wifi = activity.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    wifi.createMulticastLock("takkie-mdns").apply { setReferenceCounted(false) }
  }

  @Command
  fun setLock(invoke: Invoke) {
    val args = invoke.parseArgs(LockArgs::class.java)
    if (args.on) lock.acquire() else if (lock.isHeld) lock.release()
    invoke.resolve(JSObject().put("held", lock.isHeld))
  }
}
