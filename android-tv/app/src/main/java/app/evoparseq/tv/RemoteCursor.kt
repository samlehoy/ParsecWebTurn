package app.evoparseq.tv

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorFilter
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PixelFormat
import android.graphics.drawable.Drawable
import android.view.View
import kotlin.math.roundToInt

/** A window-local UI decoration. Never touches the codec Surface or video pixels. */
internal class RemoteCursor(private val decor: View) : AutoCloseable {
    private val location = IntArray(2)
    private val arrow = Arrow()
    init {
        arrow.setVisible(false, false)
        decor.overlay.add(arrow)
    }
    fun update(packed: Long, left: Float, top: Float, width: Int, height: Int) {
        if ((packed and (1L shl 32)) == 0L || width <= 0 || height <= 0) {
            arrow.setVisible(false, false)
            return
        }
        // Explicit bounds cap the cursor at 2.5% of the shorter viewport edge.
        val w = minOf(20f * decor.resources.displayMetrics.density, minOf(width,height) / 40f)
            .roundToInt().coerceAtLeast(1)
        val h = (w * 1.4f).roundToInt().coerceAtLeast(1)
        decor.getLocationInWindow(location)
        val x = (left - location[0] + width * (packed and 65535L).toFloat() / 65535f)
            .roundToInt().coerceIn(0, (decor.width-w).coerceAtLeast(0))
        val y = (top - location[1] + height * ((packed ushr 16) and 65535L).toFloat() / 65535f)
            .roundToInt().coerceIn(0, (decor.height-h).coerceAtLeast(0))
        arrow.setBounds(x,y,x+w,y+h)
        arrow.setVisible(true, false)
        arrow.invalidateSelf()
    }
    override fun close() { decor.overlay.remove(arrow) }

    private class Arrow : Drawable() {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val path = Path().apply {
            moveTo(0f,0f);lineTo(0f,24f);lineTo(6f,18f);lineTo(12f,28f)
            lineTo(16f,26f);lineTo(10f,16f);lineTo(19f,16f);close()
        }
        override fun draw(canvas: Canvas) {
            if (!isVisible || bounds.isEmpty) return
            val saved = canvas.save()
            try {
                canvas.translate(bounds.left.toFloat(),bounds.top.toFloat())
                canvas.scale(bounds.width()/20f,bounds.height()/28f)
                paint.style=Paint.Style.FILL; paint.color=Color.WHITE
                canvas.drawPath(path,paint)
                paint.style=Paint.Style.STROKE; paint.color=Color.BLACK; paint.strokeWidth=1f
                canvas.drawPath(path,paint)
            } finally { canvas.restoreToCount(saved) }
        }
        override fun setAlpha(alpha: Int) { paint.alpha=alpha; invalidateSelf() }
        override fun setColorFilter(colorFilter: ColorFilter?) { paint.colorFilter=colorFilter; invalidateSelf() }
        @Suppress("DEPRECATION")
        override fun getOpacity(): Int = PixelFormat.TRANSLUCENT
    }
}
