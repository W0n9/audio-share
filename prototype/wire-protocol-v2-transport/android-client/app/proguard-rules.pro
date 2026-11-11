# Prototype R8 rules (ticket 01).
#
# JNI entry points are looked up by symbol name at System.loadLibrary time;
# R8 must not rename or remove them.
-keep class dev.prototype.wireproto.ProtoNative { *; }

# Keep the crash-visible class names of the prototype (evidence readability)
-keepattributes SourceFile,LineNumberTable
