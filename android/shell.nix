{ pkgs ? import <nixpkgs> {
    config = {
      android_sdk.accept_license = true;
      allowUnfree = true;
    };
  }
}:
let
  androidComposition = pkgs.androidenv.composeAndroidPackages {
    platformVersions = [ "34" "35" ];
    buildToolsVersions = [ "34.0.0" "35.0.0" ];
    includeEmulator = false;
    includeNDK = false;
  };
  androidSdk = androidComposition.androidsdk;
in
pkgs.mkShell {
  buildInputs = [
    androidSdk
    pkgs.gradle
    pkgs.jdk17
  ];

  ANDROID_HOME = "${androidSdk}/libexec/android-sdk";
  ANDROID_SDK_ROOT = "${androidSdk}/libexec/android-sdk";
  JAVA_HOME = "${pkgs.jdk17}";
  GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidSdk}/libexec/android-sdk/build-tools/35.0.0/aapt2";
  LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath [pkgs.stdenv.cc.cc.lib]}";
}
