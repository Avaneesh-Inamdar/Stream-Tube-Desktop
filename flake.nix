{
  description = "Stream Tube - A native YouTube client for Windows and Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
          };
        };

        # Version from Cargo.toml
        version = "0.7.2-alpha";

        # Fetch the Linux release binary
        streamtube-linux = pkgs.fetchurl {
          url = "https://github.com/Avaneesh-Inamdar/Stream-Tube-Desktop/releases/download/v${version}/StreamTube-${version}-linux-x86_64.tar.gz";
          sha256 = "sha256-YAFFiZKD26E2gEJpRd1aR1jQ0dgo42TPc6uTX60Y04w=";
        };

        # Extract the binary
        extracted = pkgs.runCommand "streamtube-extracted" {
          buildInputs = [ pkgs.gnutar ];
        } ''
          mkdir -p $out
          tar -xzf ${streamtube-linux} -C $out
        '';

        # Runtime dependencies
        runtimeDeps = with pkgs; [
          fontconfig
          freetype
          wayland
          libxkbcommon
          libx11
          libxcb
          vulkan-loader
          openssl
          zstd
          sqlite
          glibc
        ];

        # Package the binary directly
        package = pkgs.stdenvNoCC.mkDerivation {
          pname = "streamtube";
          inherit version;

          src = extracted;

          installPhase = ''
            mkdir -p $out/bin
            cp StreamTube-${version}-linux-x86_64/streamtube $out/bin/streamtube
            chmod +x $out/bin/streamtube

            # Desktop file
            mkdir -p $out/share/applications
            cp StreamTube-${version}-linux-x86_64/streamtube.desktop $out/share/applications/

            # Icon
            mkdir -p $out/share/icons/hicolor/256x256/apps
            cp StreamTube-${version}-linux-x86_64/streamtube.png $out/share/icons/hicolor/256x256/apps/streamtube.png

            # License and readme
            mkdir -p $out/share/doc/streamtube
            cp StreamTube-${version}-linux-x86_64/LICENSE $out/share/doc/streamtube/
            cp StreamTube-${version}-linux-x86_64/README.md $out/share/doc/streamtube/
          '';
        };

      in {
        devShells.default = pkgs.mkShell {
          name = "streamtube-dev";
          buildInputs = with pkgs; [
            rustup
            pkg-config
            fontconfig
            freetype
            wayland
            libxkbcommon
            libx11
            libxcb
            vulkan-loader
            openssl
            zstd
            sqlite
          ];
          shellHook = ''
            export RUST_BACKTRACE=1
            echo "Stream Tube development environment"
            echo "Run 'cargo build --release' to build"
            echo "Run 'cargo run --release' to run"
          '';
        };

        packages.default = package;

        # App package for use in other flakes
        apps.default = {
          type = "app";
          program = "${package}/bin/streamtube";
        };

        # Checks for CI
        checks = {
          build = package;
          check = pkgs.runCommand "check-binary" {
            buildInputs = [ package ];
          } ''
            # Verify binary exists and has correct permissions
            test -x ${package}/bin/streamtube
            echo "Binary installed correctly at ${package}/bin/streamtube" > "$out"
          '';
        };
      }
    );
}