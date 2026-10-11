package com.martcpp.takkie

import android.app.Activity
import android.view.WindowManager
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class KeepOnArgs {
  var on: Boolean = false
}

// Stops the screen sleeping while the app is in front. Called from Rust
// (mobile.rs).
@TauriPlugin
class ScreenPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun setKeepOn(invoke: Invoke) {
    val args = invoke.parseArgs(KeepOnArgs::class.java)
    // Window flags may only be changed on the main thread.
    activity.runOnUiThread {
      val flag = WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON
      if (args.on) activity.window.addFlags(flag) else activity.window.clearFlags(flag)
      invoke.resolve(JSObject().put("on", args.on))
    }
  }
}
