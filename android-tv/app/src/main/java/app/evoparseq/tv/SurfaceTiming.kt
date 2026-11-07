package app.evoparseq.tv

import android.os.Build
import android.util.Log
import android.view.Surface

/** OS hint only. No Surface replacement, frame timer, buffer queue or forced HDMI mode switch. */
internal class SurfaceTiming {
    private var appliedSurface: Surface? = null
    private var appliedRate = Float.NaN

    fun apply(surface: Surface, rate: Float) {
        if (Build.VERSION.SDK_INT < 30 || !surface.isValid) return
        if (appliedSurface === surface && appliedRate == rate) return
        appliedSurface = surface
        appliedRate = rate
        try {
            // The two-argument API only permits seamless changes. Rust sends zero
            // for dynamic/unknown cadence, clearing a previous fixed-source vote.
            surface.setFrameRate(rate, Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE)
            Log.i("EvoParseq", "Video Surface frame-rate hint: $rate Hz")
        } catch (error: IllegalArgumentException) {
            Log.w("EvoParseq", "Surface frame-rate hint rejected: ${error.javaClass.simpleName}")
        } catch (error: IllegalStateException) {
            Log.w("EvoParseq", "Surface frame-rate hint unavailable: ${error.javaClass.simpleName}")
        }
    }

    fun detached() { appliedSurface = null; appliedRate = Float.NaN }
}
