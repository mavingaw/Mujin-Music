package com.parsonlabs.systemcolors

import android.os.Build
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/**
 * Android 12+ dynamic colour ("Material You"): the system palette generated
 * from the wallpaper. Mujin Music reads the seed tones and builds its own
 * Material 3 scheme from them, so the app follows the phone's colours.
 */
class ParsonSystemColorsModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("ParsonSystemColors")

    Function("getSystemColors") {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
        return@Function null
      }
      val context = appContext.reactContext ?: return@Function null
      mapOf(
        "accent1" to context.getColor(android.R.color.system_accent1_500),
        "accent2" to context.getColor(android.R.color.system_accent2_500),
        "accent3" to context.getColor(android.R.color.system_accent3_500),
        "neutral1" to context.getColor(android.R.color.system_neutral1_500),
        "neutral2" to context.getColor(android.R.color.system_neutral2_500)
      )
    }
  }
}
