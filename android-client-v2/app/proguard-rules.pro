# R8 rules for the wire-protocol-v2 client (ticket 08).
#
# JNI entry points are looked up by symbol name at System.loadLibrary time;
# R8 must not rename or remove them.
-keep class dev.audioshare.v2.ClientNative { *; }

# JNI upcall seam: native code calls ClientSigner.publicKeySec1()/sign() by
# name on the Kotlin object, so R8 must not rename or strip them.
-keep interface dev.audioshare.v2.ClientSigner { *; }
-keep class * implements dev.audioshare.v2.ClientSigner { *; }
-keepclassmembers class * implements dev.audioshare.v2.ClientSigner {
    public java.security.PublicKey getPublicKey();
    public byte[] publicKeySec1();
    public byte[] sign(byte[]);
}

-keepattributes SourceFile,LineNumberTable
