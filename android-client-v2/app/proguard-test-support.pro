# -PtestSupport=true only: the adb-extras lifecycle harness is reached by
# Class.forName, so R8 must keep its original name and methods.
-keep class dev.audioshare.v2.TestSupportHarness { *; }
