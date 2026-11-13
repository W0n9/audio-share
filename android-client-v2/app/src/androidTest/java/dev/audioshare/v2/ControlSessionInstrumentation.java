package dev.audioshare.v2;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Intent;
import android.content.ComponentName;
import android.content.Context;
import android.content.ServiceConnection;
import android.os.IBinder;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import kotlin.jvm.functions.Function1;
import android.os.Bundle;
import android.app.ActivityManager;
import android.view.View;
import android.view.ViewGroup;
import android.widget.TextView;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicReference;

/** 只经 Android 的公开 UI 接口验证 minified release 中的真实 native 会话。 */
public final class ControlSessionInstrumentation extends Instrumentation {
    private Bundle arguments;
    private final AtomicReference<Activity> currentActivity = new AtomicReference<>();
    private final CountDownLatch screenStopped = new CountDownLatch(1);
    private volatile boolean awaitingScreenStop;

    @Override public void callActivityOnStop(Activity activity) {
        super.callActivityOnStop(activity);
        if (awaitingScreenStop) screenStopped.countDown();
    }

    @Override public void callActivityOnResume(Activity activity) {
        super.callActivityOnResume(activity);
        currentActivity.set(activity);
    }

    @Override public void onCreate(Bundle args) {
        super.onCreate(args);
        arguments = args;
        start();
    }

    @Override public void onStart() {
        Activity activity = null;
        Bundle result = new Bundle();
        int code = Activity.RESULT_CANCELED;
        try {
            // MIUI 拒绝后台自行弹出 Activity；由宿主 adb 显式打开真实界面。
            Bundle progress = new Bundle();
            progress.putString("stream", "AWAITING_ACTIVITY\n");
            sendStatus(0, progress);
            long launchedBy = System.nanoTime() + 10_000_000_000L;
            while (currentActivity.get() == null && System.nanoTime() < launchedBy) Thread.sleep(100);
            activity = currentActivity.get();
            if (activity == null) throw new AssertionError("Activity 未进入 RESUMED");
            assertReady(activity);
            sendProgress("NATIVE_READY");
            String testCase = arguments.getString("case", "ready");
            if ("rebind".equals(testCase)) {
                assertBoundServiceReplaysReady();
            } else if ("recreate".equals(testCase)) {
                activity = recreateAndAssertReady(activity);
                assertBoundServiceReplaysReady();
            } else if ("lock".equals(testCase)) {
                ControlSessionService owner = assertBoundServiceReplaysReady();
                currentActivity.set(null);
                awaitingScreenStop = true;
                sendProgress("AWAIT_SCREEN_SLEEP");
                if (!screenStopped.await(10, TimeUnit.SECONDS)) throw new AssertionError("锁屏未触发 Activity.onStop");
                Thread.sleep(2_000);
                if (!"READY".equals(owner.stateSnapshot()) || !serviceIsForeground()) {
                    throw new AssertionError("锁屏期间会话或前台 Service 丢失：" + owner.stateSnapshot());
                }
                sendProgress("AWAIT_SCREEN_WAKE");
                long wakeBy = System.nanoTime() + 10_000_000_000L;
                while (currentActivity.get() == null && System.nanoTime() < wakeBy) Thread.sleep(100);
                Activity resumed = currentActivity.get();
                if (resumed == null) throw new AssertionError("唤醒后 Activity 未恢复");
                activity = resumed;
                assertReady(activity);
                assertBoundServiceReplaysReady();
            } else if ("stop".equals(testCase)) {
                ControlSessionService owner = assertBoundServiceReplaysReady();
                getTargetContext().startService(new Intent().setClassName(getTargetContext(),
                    "dev.audioshare.v2.ControlSessionService").setAction("dev.audioshare.v2.CONTROL_STOP"));
                long stoppedBy = System.nanoTime() + 5_000_000_000L;
                while ((!"CLOSED".equals(owner.stateSnapshot()) || serviceIsForeground()) &&
                    System.nanoTime() < stoppedBy) Thread.sleep(100);
                if (!"CLOSED".equals(owner.stateSnapshot()) || serviceIsForeground()) {
                    throw new AssertionError("ACTION_STOP 未清理：snapshot=" + owner.stateSnapshot() +
                        ", foreground=" + serviceIsForeground());
                }
            } else if (!"ready".equals(testCase)) {
                throw new AssertionError("未知测试：" + testCase);
            }
            result.putString("stream", "\nOK (1 test)\n");
            code = Activity.RESULT_OK;
        } catch (Throwable failure) {
            result.putString("stream", "\nFAIL: " + failure + "\n");
        } finally {
            if (activity != null) {
                final Activity closing = activity;
                runOnMainSync(new Runnable() {
                    @Override public void run() { closing.finish(); }
                });
                waitForIdleSync();
            }
            finish(code, result);
        }
    }

    private void sendProgress(String message) {
        Bundle status = new Bundle();
        status.putString("stream", message + "\n");
        sendStatus(0, status);
    }

    @SuppressWarnings("deprecation")
    private boolean serviceIsForeground() {
        ActivityManager manager = (ActivityManager) getTargetContext().getSystemService(Context.ACTIVITY_SERVICE);
        for (ActivityManager.RunningServiceInfo info : manager.getRunningServices(16)) {
            if ("dev.audioshare.v2.ControlSessionService".equals(info.service.getClassName())) return info.foreground;
        }
        return false;
    }

    private Activity recreateAndAssertReady(final Activity oldActivity) throws InterruptedException {
        currentActivity.set(null);
        runOnMainSync(new Runnable() {
            @Override public void run() { oldActivity.recreate(); }
        });
        long resumedBy = System.nanoTime() + 5_000_000_000L;
        while (currentActivity.get() == null && System.nanoTime() < resumedBy) Thread.sleep(100);
        Activity recreated = currentActivity.get();
        if (recreated == null || recreated == oldActivity) throw new AssertionError("Activity 没有完成重建");
        assertReady(recreated);
        return recreated;
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    private ControlSessionService assertBoundServiceReplaysReady() throws InterruptedException {
        final CountDownLatch bound = new CountDownLatch(1);
        final AtomicReference<String> immediate = new AtomicReference<>();
        final AtomicReference<ControlSessionService> owner = new AtomicReference<>();
        ServiceConnection connection = new ServiceConnection() {
            @Override public void onServiceConnected(ComponentName name, IBinder binder) {
                final AtomicReference<String> seen = new AtomicReference<>();
                ControlSessionService service = ((ControlBinder) binder).getService();
                owner.set(service);
                if (!"READY".equals(service.stateSnapshot())) {
                    immediate.set("snapshot=" + service.stateSnapshot());
                    bound.countDown();
                    return;
                }
                service.setStateListener(new Function1() {
                    @Override public Object invoke(Object state) { seen.set((String) state); return null; }
                });
                // 必须同步重放；后续轮询不能把这个 RED 变成假通过。
                immediate.set(seen.get());
                bound.countDown();
            }
            @Override public void onServiceDisconnected(ComponentName name) {}
        };
        Intent intent = new Intent().setClassName(getTargetContext(),
            "dev.audioshare.v2.ControlSessionService");
        if (!getTargetContext().bindService(intent, connection, 0)) throw new AssertionError("无法绑定活动会话");
        try {
            if (!bound.await(3, TimeUnit.SECONDS)) throw new AssertionError("未收到 Service binding");
            if (!"READY".equals(immediate.get())) {
                throw new AssertionError("READY 会话重绑定后必须立即重放 READY；实际：" + immediate.get());
            }
            return owner.get();
        } finally {
            getTargetContext().unbindService(connection);
        }
    }

    private void assertReady(final Activity activity) throws InterruptedException {
        final AtomicReference<List<String>> text = new AtomicReference<>(new ArrayList<String>());
        long deadline = System.nanoTime() + 5_000_000_000L;
        do {
            runOnMainSync(new Runnable() {
                @Override public void run() { text.set(viewText(activity.getWindow().getDecorView())); }
            });
            if (text.get().contains("READY")) return;
            Thread.sleep(100);
        } while (System.nanoTime() < deadline);
        throw new AssertionError("原生会话完成认证后，Activity 必须显示 READY；实际文本：" + text.get());
    }

    private List<String> viewText(View view) {
        List<String> text = new ArrayList<>();
        if (view instanceof TextView) text.add(((TextView) view).getText().toString());
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int index = 0; index < group.getChildCount(); index++) {
                text.addAll(viewText(group.getChildAt(index)));
            }
        }
        return text;
    }
}
