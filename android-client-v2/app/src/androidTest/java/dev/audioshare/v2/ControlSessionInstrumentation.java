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
import java.util.Arrays;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.concurrent.atomic.AtomicReference;

/** 只经 Android 的公开 UI 接口验证 minified release 中的真实 native 会话。 */
public final class ControlSessionInstrumentation extends Instrumentation {
    /** M4+: fake capture can legitimately leave the session in PLAYING. */
    private static final Set<String> LIVE_STATES =
        Collections.unmodifiableSet(new HashSet<>(Arrays.asList("READY", "PLAYING")));
    /** A reconnect/recreate would surface one of these during the observed window. */
    private static final Set<String> REBUILD_STATES =
        Collections.unmodifiableSet(new HashSet<>(Arrays.asList("IDLE", "AUTHENTICATING", "CLOSING", "CLOSED")));

    private Bundle arguments;
    private final AtomicReference<Activity> currentActivity = new AtomicReference<>();
    private final CountDownLatch screenStopped = new CountDownLatch(1);
    private volatile boolean awaitingScreenStop;
    private final CountDownLatch earlyBound = new CountDownLatch(1);
    private ControlSessionService earlyOwner;
    private ServiceConnection earlyConnection;

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
            String testCase = arguments.getString("case", "ready");
            if ("ready".equals(testCase)) {
                // READY 在 M4 是 AUTHENTICATING→PLAYING 之间的短暂状态。先绑定
                // Service 再拉起 Activity，直接轮询公开的 stateSnapshot，
                // 避免 UI 回调注册太晚而漏掉 READY。
                bindEarlyService();
            }
            // MIUI 拒绝后台自行弹出 Activity；由宿主 adb 显式打开真实界面。
            Bundle progress = new Bundle();
            progress.putString("stream", "AWAITING_ACTIVITY\n");
            sendStatus(0, progress);
            activity = awaitResumedActivity(10_000, "Activity 未进入 RESUMED");
            if ("ready".equals(testCase)) {
                // ready 用例本身保持严格口径：启动完成必须观察到 READY。
                assertReady(activity, earlyOwner);
            } else {
                assertLiveState(activity, "启动");
            }
            sendProgress("NATIVE_READY");
            if ("rebind".equals(testCase)) {
                assertBoundServiceReplaysReady();
            } else if ("recreate".equals(testCase)) {
                ControlSessionService owner = assertBoundServiceReplaysReady();
                List<String> observed = observeStates(owner);
                activity = recreateAndAssertLive(activity, observed);
                ControlSessionService rebound = assertBoundServiceReplaysReady();
                if (rebound != owner) throw new AssertionError("Activity 重建后 Service/session 被重建");
                assertNoRebuildTransitions(observed, "Activity 重建");
                assertLiveSnapshot(owner, "Activity 重建后");
            } else if ("recreate-loop".equals(testCase)) {
                ControlSessionService owner = assertBoundServiceReplaysReady();
                List<String> observed = observeStates(owner);
                for (int i = 0; i < 10; i++) {
                    activity = recreateAndAssertLive(activity, observed);
                    ControlSessionService rebound = assertBoundServiceReplaysReady();
                    if (rebound != owner) {
                        throw new AssertionError("第 " + (i + 1) + " 次重建后 Service/session 被重建");
                    }
                    // 重新绑定替换了监听器；继续把后续状态写进同一份记录。
                    observeStates(owner, observed);
                    assertNoRebuildTransitions(observed, "第 " + (i + 1) + " 次 Activity 重建");
                    assertLiveSnapshot(owner, "第 " + (i + 1) + " 次 Activity 重建后");
                }
            } else if ("lock".equals(testCase)) {
                ControlSessionService owner = assertBoundServiceReplaysReady();
                List<String> observed = observeStates(owner);
                int pidBefore = servicePid();
                currentActivity.set(null);
                awaitingScreenStop = true;
                sendProgress("AWAIT_SCREEN_SLEEP");
                if (!screenStopped.await(10, TimeUnit.SECONDS)) throw new AssertionError("锁屏未触发 Activity.onStop");
                Thread.sleep(2_000);
                assertLiveSnapshot(owner, "锁屏期间");
                if (!serviceIsForeground()) throw new AssertionError("锁屏期间前台媒体 Service 丢失");
                assertSameServiceProcess(pidBefore, "锁屏期间");
                assertNoRebuildTransitions(observed, "锁屏期间");
                sendProgress("AWAIT_SCREEN_WAKE");
                activity = awaitResumedActivity(10_000, "唤醒后 Activity 未恢复");
                assertLiveState(activity, "唤醒后");
                assertNoRebuildTransitions(observed, "锁屏与唤醒");
                ControlSessionService rebound = assertBoundServiceReplaysReady();
                if (rebound != owner) throw new AssertionError("唤醒后 Service/session 被重建");
                if (!serviceIsForeground()) throw new AssertionError("唤醒后前台媒体 Service 丢失");
                assertSameServiceProcess(pidBefore, "唤醒后");
                assertLiveSnapshot(owner, "唤醒后");
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
            // Activity.finish 已发送 ACTION_STOP；释放提前绑定，避免把 Service 留在进程里。
            if (earlyConnection != null) {
                getTargetContext().unbindService(earlyConnection);
                earlyConnection = null;
            }
            finish(code, result);
        }
    }

    private Activity awaitResumedActivity(long timeoutMillis, String failureMessage) throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis);
        while (currentActivity.get() == null && System.nanoTime() < deadline) Thread.sleep(100);
        Activity activity = currentActivity.get();
        if (activity == null) throw new AssertionError(failureMessage);
        return activity;
    }

    private void sendProgress(String message) {
        Bundle status = new Bundle();
        status.putString("stream", message + "\n");
        sendStatus(0, status);
    }

    @SuppressWarnings("deprecation")
    private boolean serviceIsForeground() {
        ActivityManager.RunningServiceInfo info = runningService();
        return info != null && info.foreground;
    }

    /** PID of the process hosting the foreground session Service, or -1. */
    @SuppressWarnings("deprecation")
    private int servicePid() {
        ActivityManager.RunningServiceInfo info = runningService();
        return info == null ? -1 : info.pid;
    }

    @SuppressWarnings("deprecation")
    private ActivityManager.RunningServiceInfo runningService() {
        ActivityManager manager = (ActivityManager) getTargetContext().getSystemService(Context.ACTIVITY_SERVICE);
        for (ActivityManager.RunningServiceInfo info : manager.getRunningServices(16)) {
            if ("dev.audioshare.v2.ControlSessionService".equals(info.service.getClassName())) return info;
        }
        return null;
    }

    /** The session process must be the same one observed before the lifecycle event. */
    private void assertSameServiceProcess(int expectedPid, String context) {
        int pid = servicePid();
        if (pid < 0 || pid != expectedPid) {
            throw new AssertionError(context + " Service 进程被重建：before=" + expectedPid + ", after=" + pid);
        }
    }

    /** 观察 Service 发布的状态；用于证明锁屏/重建期间没有出现重连窗口。 */
    private List<String> observeStates(ControlSessionService owner) {
        List<String> observed = Collections.synchronizedList(new ArrayList<String>());
        observeStates(owner, observed);
        return observed;
    }

    private void observeStates(ControlSessionService owner, final List<String> observed) {
        owner.setStateListener(new Function1() {
            @Override public Object invoke(Object state) {
                observed.add((String) state);
                return null;
            }
        });
    }

    private void assertNoRebuildTransitions(List<String> observed, String context) {
        synchronized (observed) {
            for (String state : observed) {
                if (REBUILD_STATES.contains(state)) {
                    throw new AssertionError(context + " 出现会话重建状态：" + state + "（完整序列：" + observed + "）");
                }
            }
        }
    }

    private void assertLiveSnapshot(ControlSessionService owner, String context) {
        String state = owner.stateSnapshot();
        if (!LIVE_STATES.contains(state)) {
            throw new AssertionError(context + " 会话未存活：" + state);
        }
    }

    private Activity recreateAndAssertLive(final Activity oldActivity, List<String> observed)
        throws InterruptedException {
        currentActivity.set(null);
        runOnMainSync(new Runnable() {
            @Override public void run() { oldActivity.recreate(); }
        });
        Activity recreated = awaitResumedActivity(5_000, "Activity 没有完成重建");
        if (recreated == oldActivity) throw new AssertionError("Activity 没有完成重建");
        assertLiveState(recreated, "Activity 重建");
        // 覆盖一次最小重连退避窗口；若重建触发了重连，会发布 IDLE/AUTHENTICATING。
        Thread.sleep(1_200);
        assertNoRebuildTransitions(observed, "Activity 重建窗口");
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
                if (!LIVE_STATES.contains(service.stateSnapshot())) {
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
            String replay = immediate.get();
            if (replay == null || !LIVE_STATES.contains(replay)) {
                throw new AssertionError("会话重绑定后必须立即重放 READY/PLAYING；实际：" + replay);
            }
            return owner.get();
        } finally {
            getTargetContext().unbindService(connection);
        }
    }

    /** 在 Activity 之前绑定 Service，用 stateSnapshot 捕捉短暂的 READY。 */
    private void bindEarlyService() throws InterruptedException {
        earlyConnection = new ServiceConnection() {
            @Override public void onServiceConnected(ComponentName name, IBinder binder) {
                earlyOwner = ((ControlBinder) binder).getService();
                earlyBound.countDown();
            }
            @Override public void onServiceDisconnected(ComponentName name) {}
        };
        Intent intent = new Intent().setClassName(getTargetContext(),
            "dev.audioshare.v2.ControlSessionService");
        if (!getTargetContext().bindService(intent, earlyConnection, Context.BIND_AUTO_CREATE)) {
            throw new AssertionError("无法提前绑定 ControlSessionService");
        }
        if (!earlyBound.await(5, TimeUnit.SECONDS)) throw new AssertionError("提前绑定未收到 Service binding");
    }

    /** ready 用例的严格断言：必须在公开的 Service 快照上观察到 READY（或 UI 已渲染 READY）。 */
    private void assertReady(final Activity activity, final ControlSessionService owner)
        throws InterruptedException {
        final AtomicReference<List<String>> text = new AtomicReference<>(new ArrayList<String>());
        long deadline = System.nanoTime() + 5_000_000_000L;
        do {
            if (owner != null && "READY".equals(owner.stateSnapshot())) return;
            runOnMainSync(new Runnable() {
                @Override public void run() { text.set(viewText(activity.getWindow().getDecorView())); }
            });
            if (text.get().contains("READY")) return;
            Thread.sleep(50);
        } while (System.nanoTime() < deadline);
        throw new AssertionError("原生会话完成认证后，必须观察到 READY；实际 snapshot="
            + (owner == null ? "<none>" : owner.stateSnapshot()) + "，文本：" + text.get());
    }

    /** M4+：媒体活跃时会话可以合法停在 READY 或 PLAYING。 */
    private void assertLiveState(final Activity activity, String context) throws InterruptedException {
        assertStateText(activity, LIVE_STATES, context + " Activity 必须显示 READY 或 PLAYING");
    }

    private void assertStateText(final Activity activity, final Set<String> accepted,
        String failureMessage) throws InterruptedException {
        final AtomicReference<List<String>> text = new AtomicReference<>(new ArrayList<String>());
        long deadline = System.nanoTime() + 5_000_000_000L;
        do {
            runOnMainSync(new Runnable() {
                @Override public void run() { text.set(viewText(activity.getWindow().getDecorView())); }
            });
            for (String value : text.get()) {
                if (accepted.contains(value)) return;
            }
            Thread.sleep(100);
        } while (System.nanoTime() < deadline);
        throw new AssertionError(failureMessage + "；实际文本：" + text.get());
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
