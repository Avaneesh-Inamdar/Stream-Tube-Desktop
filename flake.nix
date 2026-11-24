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
          mpv
          glibc
          noto-fonts
        ];

        # Package the binary directly
        package = pkgs.stdenvNoCC.mkDerivation {
          pname = "streamtube";
          inherit version;

          src = extracted;

          nativeBuildInputs = with pkgs; [ patchelf ];

          installPhase = ''
            mkdir -p $out/bin
            cp StreamTube-${version}-linux-x86_64/streamtube $out/bin/streamtube-bin
            chmod +x $out/bin/streamtube-bin

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

            # Create fontconfig configuration that includes noto-fonts
            mkdir -p $out/etc/fonts/conf.d
            cat > $out/etc/fonts/fonts.conf << 'FONTCONF_EOF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <description>Stream Tube font configuration</description>
  <dir>${pkgs.noto-fonts}/share/fonts</dir>
  <include ignore_missing="yes">${pkgs.fontconfig}/etc/fonts/conf.d</include>
  <cachedir prefix="xdg">fontconfig</cachedir>
</fontconfig>
FONTCONF_EOF

            # Create wrapper script with fontconfig environment
            cat > $out/bin/streamtube << 'WRAPPER_EOF'
#!/bin/sh
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
export FONTCONFIG_FILE="$SCRIPT_DIR/../etc/fonts/fonts.conf"
export XDG_DATA_DIRS="@fontconfig_share@:@noto_fonts_share@"
if [ -n "$XDG_DATA_DIRS" ]; then
  export XDG_DATA_DIRS="$XDG_DATA_DIRS:$XDG_DATA_DIRS"
fi
exec "$SCRIPT_DIR/streamtube-bin" "$@"
WRAPPER_EOF
            # Substitute store paths
            sed -i "s|@fontconfig_share@|${pkgs.fontconfig}/share|g" $out/bin/streamtube
            sed -i "s|@noto_fonts_share@|${pkgs.noto-fonts}/share|g" $out/bin/streamtube
            chmod +x $out/bin/streamtube
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
            # Verify wrapper script exists and binary is present
            test -x ${package}/bin/streamtube
            test -f ${package}/bin/streamtube-bin
            # Verify it's a text wrapper (shebang present, not a raw ELF)
            head -1 ${package}/bin/streamtube | grep -q "#!\(/bin/sh\|/nix/store.*bin/sh\)"
            # Verify store paths are substituted (not placeholder strings)
            ! grep -q "@fontconfig_share@" ${package}/bin/streamtube
            ! grep -q "@noto_fonts_share@" ${package}/bin/streamtube
            # Verify fonts.conf includes noto-fonts
            test -f ${package}/etc/fonts/fonts.conf
            grep -q "noto-fonts" ${package}/etc/fonts/fonts.conf
            echo "All checks passed" > "$out"
          '';
        };
      }
    );
}