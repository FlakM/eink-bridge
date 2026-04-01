{
  description = "E-Ink Bridge: synchronous review sessions on e-ink";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-25.11";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, flake-utils, rust-overlay, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
          config.allowUnfree = true;
          config.android_sdk.accept_license = true;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "llvm-tools-preview" ];
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        androidComposition = pkgs.androidenv.composeAndroidPackages {
          buildToolsVersions = [ "35.0.0" "34.0.0" ];
          platformVersions = [ "35" "34" ];
          includeEmulator = false;
          includeNDK = false;
        };
        androidSdk = androidComposition.androidsdk;

        commonArgs = {
          src = pkgs.lib.cleanSourceWith {
            src = ./server;
            filter = path: type:
              (craneLib.filterCargoSources path type) ||
              (pkgs.lib.hasInfix "/assets" path) ||
              (pkgs.lib.hasInfix "/golden" path) ||
              (pkgs.lib.hasInfix "/fixtures" path);
          };
          strictDeps = true;
          buildInputs = with pkgs; [
            openssl
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
            pkgs.darwin.apple_sdk.frameworks.Security
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        einkBridge = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.makeWrapper ];
          postInstall = ''
            for bin in $out/bin/*; do
              wrapProgram "$bin" \
                --set TESSDATA_PREFIX "${pkgs.tesseract}/share/tessdata"
            done
          '';
          postFixup = ''
            mkdir -p $out/share/eink-bridge
            cp -r $src/assets $out/share/eink-bridge/assets
          '';
        });

        harness = pkgs.stdenvNoCC.mkDerivation {
          name = "eink-bridge-harness";
          src = ./harness;
          phases = [ "installPhase" ];
          installPhase = ''
            mkdir -p $out/skills $out/output-styles
            cp -r $src/skills/* $out/skills/
            cp -r $src/output-styles/* $out/output-styles/
          '';
        };
      in {
        packages = {
          default = einkBridge;
          eink-bridge = einkBridge;
          inherit harness;
        };

        devShells.default = craneLib.devShell {
          checks = { inherit cargoArtifacts; };
          ANDROID_HOME = "${androidSdk}/libexec/android-sdk";
          ANDROID_SDK_ROOT = "${androidSdk}/libexec/android-sdk";
          packages = with pkgs; [
            rust-analyzer
            just
            cargo-watch
            cargo-llvm-cov
            androidSdk
            jdk17
            einkBridge
          ];
          shellHook = let
            sdkPath = "${androidSdk}/libexec/android-sdk";
            aapt2Path = "${sdkPath}/build-tools/35.0.0/aapt2";
          in ''
            # Generate local.properties pointing at the nix Android SDK
            if [ -d android ]; then
              printf 'sdk.dir=${sdkPath}\n' > android/local.properties
            fi
            # Tell AGP to use the nix-patched aapt2 instead of the Maven one
            # (the Maven binary has /lib64/ld-linux-x86-64.so.2 as interpreter
            # which doesn't exist on NixOS). We write this to ~/.gradle/gradle.properties
            # because it's a Gradle project property that must be set before transforms run.
            mkdir -p "''${HOME}/.gradle"
            if ! grep -q 'aapt2FromMavenOverride' "''${HOME}/.gradle/gradle.properties" 2>/dev/null; then
              printf '\nandroid.aapt2FromMavenOverride=${aapt2Path}\n' >> "''${HOME}/.gradle/gradle.properties"
            else
              sed -i 's|android.aapt2FromMavenOverride=.*|android.aapt2FromMavenOverride=${aapt2Path}|' "''${HOME}/.gradle/gradle.properties"
            fi
          '';
        };
      }
    );
}
