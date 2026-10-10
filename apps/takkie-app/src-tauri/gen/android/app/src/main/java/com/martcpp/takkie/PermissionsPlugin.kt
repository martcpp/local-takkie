package com.martcpp.takkie

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

private const val MICROPHONE = "microphone"
private const val NOTIFICATIONS = "notifications"

// The runtime permissions the radio needs. Called from Rust (mobile.rs).
@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = MICROPHONE),
    Permission(strings = [Manifest.permission.POST_NOTIFICATIONS], alias = NOTIFICATIONS),
  ]
)
class PermissionsPlugin(private val activity: Activity) : Plugin(activity) {
  // Before Android 13 notifications need no permission, and asking fails.
  private val asksForNotifications = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU

  private fun current(): JSObject {
    val notifications =
      if (asksForNotifications) getPermissionState(NOTIFICATIONS) else PermissionState.GRANTED
    return JSObject()
      .put(MICROPHONE, (getPermissionState(MICROPHONE) ?: PermissionState.PROMPT).toString())
      .put(NOTIFICATIONS, (notifications ?: PermissionState.PROMPT).toString())
  }

  @Command
  fun state(invoke: Invoke) {
    invoke.resolve(current())
  }

  @Command
  fun ask(invoke: Invoke) {
    val wanted = listOfNotNull(MICROPHONE, NOTIFICATIONS.takeIf { asksForNotifications })
    val missing = wanted.filter { getPermissionState(it) != PermissionState.GRANTED }
    if (missing.isEmpty()) {
      invoke.resolve(current())
    } else {
      requestPermissionForAliases(missing.toTypedArray(), invoke, "asked")
    }
  }

  @PermissionCallback
  private fun asked(invoke: Invoke) {
    invoke.resolve(current())
  }

  // Where the user can turn a refused permission back on.
  @Command
  fun openSettings(invoke: Invoke) {
    val page = Intent(
      Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
      Uri.fromParts("package", activity.packageName, null)
    ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    activity.startActivity(page)
    invoke.resolve()
  }
}
