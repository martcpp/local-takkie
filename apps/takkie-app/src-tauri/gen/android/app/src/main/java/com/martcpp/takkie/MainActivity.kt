package com.martcpp.takkie

import android.os.Bundle
import android.view.KeyEvent
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // The headset button talks while the app is on screen. Keys that aren't
  // ours go on as usual.
  override fun dispatchKeyEvent(event: KeyEvent): Boolean {
    return HeadsetPlugin.onKey(event) || super.dispatchKeyEvent(event)
  }

  // Back puts the app in the background, like Home. Finishing the activity
  // would end the process and the radio with it; the notification's Stop is
  // how the radio is turned off.
  @Suppress("OVERRIDE_DEPRECATION")
  override fun onBackPressed() {
    moveTaskToBack(true)
  }
}
