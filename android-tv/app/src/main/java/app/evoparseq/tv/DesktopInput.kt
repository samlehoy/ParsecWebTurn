package app.evoparseq.tv

import android.app.Activity
import android.os.Handler
import android.os.Looper
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.ViewConfiguration
import androidx.compose.runtime.*
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Thin OS adapter. All mouse gestures, key layout/wire rules and held state are Rust. */
internal class DesktopInput(private val activity: Activity) {
    private val main = Handler(Looper.getMainLooper())
    private val latest = AtomicLong(0)
    private val posted = AtomicBoolean(false)
    private val usedDevices = mutableSetOf<Int>()
    private var closed = false
    private var left = 0f
    private var top = 0f
    private var width = 0
    private var height = 0
    var pointer by mutableLongStateOf(0)
        private set
    private var remoteMouseState by mutableStateOf(false)
    var mouseMode: Boolean
        get() = remoteMouseState
        set(value) {
            remoteMouseState = value
            NativeCore.desktopViewport(width,height)
            NativeCore.remoteMouseMode(value, ViewConfiguration.getLongPressTimeout())
        }
    private var captureMouseState by mutableStateOf(true)
    var captureMouse: Boolean
        get() = captureMouseState
        set(value) { captureMouseState=value; updateCapture() }
    var enabled = false
        set(value) { field=value; updateCapture() }
    init {
        activity.window.decorView.setOnCapturedPointerListener { _, event -> motion(event,true) }
    }
    private fun hasMouse(): Boolean = InputDevice.getDeviceIds().any { id ->
        InputDevice.getDevice(id)?.let { !it.isVirtual && (it.supportsSource(InputDevice.SOURCE_MOUSE) || it.supportsSource(InputDevice.SOURCE_MOUSE_RELATIVE)) } == true
    }
    fun devicesChanged() = updateCapture()
    private fun updateCapture() {
        val view=activity.window.decorView
        if (enabled && captureMouse && hasMouse()) view.post { if (enabled && captureMouse && hasMouse() && view.hasWindowFocus()) view.requestPointerCapture() }
        else if (view.hasPointerCapture()) view.releasePointerCapture()
    }
    fun viewport(x:Float,y:Float,w:Int,h:Int) {
        if (left==x && top==y && width==w && height==h) return
        left=x;top=y;width=w;height=h
        NativeCore.desktopViewport(w,h)
    }
    /** 0 = local UI, 1 = consumed remotely, 2 = release/show local controls. */
    fun key(event:KeyEvent):Int {
        if (!enabled || (event.action!=KeyEvent.ACTION_DOWN && event.action!=KeyEvent.ACTION_UP)) return 0
        val device=event.device ?: return 0
        val keyboard=device.keyboardType==InputDevice.KEYBOARD_TYPE_ALPHABETIC && !device.isVirtual
        if (keyboard && !event.isFromSource(InputDevice.SOURCE_DPAD)) {
            usedDevices.add(device.id)
            return NativeCore.desktopKey(event.scanCode,event.keyCode,event.metaState,
                event.action==KeyEvent.ACTION_DOWN,event.isCanceled,event.repeatCount)
        }
        if (event.keyCode==KeyEvent.KEYCODE_BACK || event.keyCode==KeyEvent.KEYCODE_HOME) return 0
        return if (NativeCore.remoteMouseKey(event.keyCode,event.action==KeyEvent.ACTION_DOWN,event.isCanceled)) 1 else 0
    }
    fun motion(event:MotionEvent,captured:Boolean=false):Boolean {
        if (!enabled || (!event.isFromSource(InputDevice.SOURCE_MOUSE) && !event.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE))) return false
        usedDevices.add(event.deviceId)
        if (event.actionMasked==MotionEvent.ACTION_CANCEL) {NativeCore.desktopRelease();return true}
        val action=event.actionMasked
        val moved=action==MotionEvent.ACTION_MOVE || action==MotionEvent.ACTION_HOVER_MOVE || (!captured && (action==MotionEvent.ACTION_DOWN || action==MotionEvent.ACTION_UP || action==MotionEvent.ACTION_BUTTON_PRESS || action==MotionEvent.ACTION_BUTTON_RELEASE))
        NativeCore.desktopPointer(if(captured) event.x.toDouble() else (event.x-left).toDouble(),
            if(captured) event.y.toDouble() else (event.y-top).toDouble(),captured,moved,width,height,event.buttonState,
            (event.getAxisValue(MotionEvent.AXIS_HSCROLL)*120f).toInt(),(event.getAxisValue(MotionEvent.AXIS_VSCROLL)*120f).toInt())
        return true
    }
    fun deviceRemoved(id:Int) {
        if (usedDevices.remove(id)) NativeCore.desktopRelease()
    }
    /** Native cursor scalars only. At most one pending UI update, never a frame buffer. */
    fun pointerChanged(value:Long) {
        latest.set(value)
        if (posted.compareAndSet(false,true)) main.post {
            if (!closed) pointer=latest.get()
            posted.set(false)
            if (!closed && latest.get()!=pointer) pointerChanged(latest.get())
        }
    }
    fun close() {
        enabled=false
        mouseMode=false
        closed=true
        pointer=0
        activity.window.decorView.setOnCapturedPointerListener(null)
    }
}
