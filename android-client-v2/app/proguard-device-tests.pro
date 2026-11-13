# 仅 -PdeviceTests=true：保留由独立测试 APK 调用的公开 Binder seam。
-keep class dev.audioshare.v2.ControlBinder {
    public dev.audioshare.v2.ControlSessionService getService();
}
-keepclassmembers class dev.audioshare.v2.ControlSessionService {
    public void setStateListener(kotlin.jvm.functions.Function1);
    public java.lang.String stateSnapshot();
}
-keep interface kotlin.jvm.functions.Function1 { *; }
