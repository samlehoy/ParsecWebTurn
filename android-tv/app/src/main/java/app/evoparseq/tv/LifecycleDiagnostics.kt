package app.evoparseq.tv

import android.app.ActivityManager
import android.app.Application
import android.app.ApplicationExitInfo
import android.os.Build

/** Own-process history only, once on the existing account worker; no log polling. */
internal fun recentAppFailure(application: Application): String? {
    if (Build.VERSION.SDK_INT < 30) return null
    return runCatching {
        val manager = application.getSystemService(ActivityManager::class.java) ?: return null
        val exit = manager.getHistoricalProcessExitReasons(application.packageName,0,1).firstOrNull()
            ?: return null
        if (System.currentTimeMillis() - exit.timestamp !in 0L..300_000L) return null
        val reason = when(exit.reason) {
            ApplicationExitInfo.REASON_CRASH -> "Java/Kotlin crash"
            ApplicationExitInfo.REASON_CRASH_NATIVE -> "native crash"
            ApplicationExitInfo.REASON_ANR -> "unresponsive app"
            else -> return null
        }
        "Android reports a recent $reason (status ${exit.status})."
    }.getOrNull()
}
