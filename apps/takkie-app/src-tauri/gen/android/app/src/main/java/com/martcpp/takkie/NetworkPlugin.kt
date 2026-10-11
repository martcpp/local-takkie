package com.martcpp.takkie

import android.app.Activity
import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

// Whether the phone is on a network where other takkies can be: Wi-Fi or
// Ethernet, not mobile data. Called from Rust (mobile.rs).
@TauriPlugin
class NetworkPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun state(invoke: Invoke) {
    val manager = activity.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    // Every network, not just the active one: with Wi-Fi that has no
    // internet, Android keeps mobile data as the active network.
    @Suppress("DEPRECATION")
    val local = manager.allNetworks.any { network ->
      val caps = manager.getNetworkCapabilities(network)
      caps != null && (
        caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
          caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        )
    }
    invoke.resolve(JSObject().put("local", local))
  }
}
