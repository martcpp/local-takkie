package com.martcpp.takkie

import android.app.Activity
import android.view.KeyEvent
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class HeadsetArgs {
  var on: Boolean = true
}

// Passes headset button presses to Rust (mobile.rs), which decides what
// they mean. Presses come from MainActivity while the app is on screen and
// from RadioService's media session otherwise.
@TauriPlugin
class HeadsetPlugin(private val activity: Activity) : Plugin(activity) {
  // Answers with the next press or release, however long that takes.
  @Command
  fun waitForButton(invoke: Invoke) {
    synchronized(Companion) {
      val next = queued.removeFirstOrNull()
      if (next != null) invoke.resolve(JSObject().put("pressed", next)) else waiting = invoke
    }
  }

  @Command
  fun setEnabled(invoke: Invoke) {
    enabled = invoke.parseArgs(HeadsetArgs::class.java).on
    invoke.resolve(JSObject().put("on", enabled))
  }

  companion object {
    // A wired earpiece's button, and what Bluetooth headsets send.
    private val BUTTONS = setOf(
      KeyEvent.KEYCODE_HEADSETHOOK,
      KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE,
      KeyEvent.KEYCODE_MEDIA_PLAY,
      KeyEvent.KEYCODE_MEDIA_PAUSE,
    )
    private val queued = ArrayDeque<Boolean>()
    private var waiting: Invoke? = null

    @Volatile
    var enabled = true
      private set

    // Returns true when the key was ours, so the caller doesn't pass it on
    // to whatever would otherwise play or pause.
    fun onKey(event: KeyEvent): Boolean {
      if (!enabled || event.keyCode !in BUTTONS) return false
      val pressed = when (event.action) {
        KeyEvent.ACTION_DOWN -> if (event.repeatCount == 0) true else return true
        KeyEvent.ACTION_UP -> false
        else -> return true
      }
      synchronized(this) {
        val asked = waiting
        waiting = null
        if (asked != null) asked.resolve(JSObject().put("pressed", pressed)) else queued.addLast(pressed)
      }
      return true
    }
  }
}
