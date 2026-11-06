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

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane, ... }:
    {
      homeManagerModules.default = import ./nix/hm-module.nix self;
    }
    // flake-utils.lib.eachDefaultSystem (system:
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

        # The emulator and its system image are most of a gigabyte, and only the
        # instrumented suite needs them, so they live in their own shell. `just
        # test-emulator` enters it; every other job keeps the small SDK.
        androidEmulatorComposition = pkgs.androidenv.composeAndroidPackages {
          buildToolsVersions = [ "35.0.0" "34.0.0" ];
          platformVersions = [ "35" "34" ];
          includeEmulator = true;
          includeSystemImages = true;
          systemImageTypes = [ "google_apis" ];
          abiVersions = [ "x86_64" ];
          includeNDK = false;
        };
        androidEmulatorSdk = androidEmulatorComposition.androidsdk;

        # Both Android shells need the same two pieces of local setup; only the SDK and
        # the packages differ. The emulator shell leaves out the built CLI: it needs a
        # compiler for the test server, not the server's own package (which would be
        # built again on every CI run just to enter a shell).
        androidShell =
          { sdkPath, extraPackages }:
          craneLib.devShell {
            checks = { inherit cargoArtifacts; };
            ANDROID_HOME = sdkPath;
            ANDROID_SDK_ROOT = sdkPath;
            packages = [
              pkgs.rust-analyzer
              pkgs.just
              pkgs.cargo-watch
              pkgs.cargo-llvm-cov
              pkgs.cargo-nextest
              pkgs.jdk17
              pkgs.curl
              # Pinned, not ambient: the watcher and channel tests run Node with
              # `--experimental-strip-types`, which a runner's older Node does not have.
              pkgs.nodejs_22
            ] ++ extraPackages;
            shellHook = let
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

        commonArgs = {
          src = pkgs.lib.cleanSourceWith {
            src = ./server;
            filter = path: type:
              (craneLib.filterCargoSources path type) ||
              (pkgs.lib.hasInfix "/assets" path) ||
              (pkgs.lib.hasInfix "/golden" path) ||
              (pkgs.lib.hasInfix "/fixtures" path) ||
              # Read by a test that keeps the metric names honest.
              (pkgs.lib.hasInfix "METRICS.md" path);
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
          postFixup = ''
            mkdir -p $out/share/eink-bridge
            cp -r $src/assets $out/share/eink-bridge/assets
          '';
        });

        einkChannel = pkgs.buildNpmPackage {
          pname = "eink-channel";
          version = "0.1.0";
          src = ./harness/channels/eink-channel;
          npmDepsHash = "sha256-F9jXvaYG9b7ZvqYSGGI8xouFt8yIYcM4aIVUdy22E/s=";
          dontBuild = true;
          nativeBuildInputs = [ pkgs.makeWrapper ];
          installPhase = ''
            mkdir -p $out/libexec/eink-channel $out/bin
            cp channel.ts $out/libexec/eink-channel/
            cp -r node_modules $out/libexec/eink-channel/
            makeWrapper ${pkgs.nodejs}/bin/node $out/bin/eink-channel-server \
              --add-flags "--experimental-strip-types $out/libexec/eink-channel/channel.ts"
          '';
        };

        # The commands the skills run. The unit tests that need nothing but Python run
        # here; the ones that start a server or Neovim stay in `just harness-test`.
        harnessTools = pkgs.python3Packages.buildPythonApplication {
          pname = "eink-harness";
          version = "0.1.0";
          pyproject = true;
          src = ./harness;
          build-system = [ pkgs.python3Packages.setuptools ];
          pythonImportsCheck = [ "eink_harness.review" "eink_harness.watch_feedback" ];
          checkPhase = ''
            runHook preCheck
            python -m unittest tests.test_eink_pr_review tests.test_eink_pr_registry \
              tests.test_check_marks tests.test_pr_comments tests.test_harness_assets
            runHook postCheck
          '';
        };

        harness = pkgs.stdenvNoCC.mkDerivation {
          name = "eink-bridge-harness";
          src = ./harness;
          phases = [ "installPhase" ];
          installPhase = ''
            mkdir -p $out/skills $out/output-styles $out/channels $out/agents
            cp -r $src/skills/* $out/skills/
            cp -r $src/output-styles/* $out/output-styles/
            cp -r $src/agents/* $out/agents/
          '';
        };
      in {
        packages = {
          default = einkBridge;
          eink-bridge = einkBridge;
          inherit harness einkChannel;
          harness-tools = harnessTools;
        };

        devShells.default = androidShell {
          sdkPath = "${androidSdk}/libexec/android-sdk";
          extraPackages = [ androidSdk einkBridge einkChannel harnessTools ];
        };

        # The instrumented suite's shell: the same tools, plus an emulator and a
        # `system-images;android-34;google_apis;x86_64` image to boot it from.
        devShells.emulator = androidShell {
          sdkPath = "${androidEmulatorSdk}/libexec/android-sdk";
          extraPackages = [ androidEmulatorSdk ];
        };
      }
    );
}
