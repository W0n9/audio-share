# R8 rules for the wire-protocol-v2 client (ticket 08).
#
# JNI entry points are looked up by symbol name at System.loadLibrary time;
# R8 must not rename or remove them.
-keep class dev.audioshare.v2.ClientNative { *; }

-keepattributes SourceFile,LineNumberTable
