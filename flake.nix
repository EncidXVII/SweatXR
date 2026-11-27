# Copyright (C) 2026 EncidXVII
# SPDX-License-Identifier: GPL-3.0-only

# SweatXR for Nix and NixOS, from the published Linux release (the same files as the tarball on GitHub Releases).
#   nix run github:EncidXVII/SweatXR
# SweatXR registers its own path with SteamVR so it starts with SteamVR, and a Nix store path changes with every
# update. So the `sweatxr` command keeps a per-user copy in ~/.local/share/sweatxr (the folder install.sh uses),
# refreshes it when the package changes, and runs that copy.
{
  description = "SweatXR: calories, active minutes and heart rate on your wrist in every SteamVR game";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { self, nixpkgs }:
    let
      version = "0.8.1";
      # The release file for each system and its SHA-256 (the .sha256 file beside it, in Nix's SRI form).
      assets = {
        x86_64-linux = {
          arch = "x64";
          hash = "sha256-FuLiK7P649VIohC4zBZv9gB4de+somvjHrY19pdFJpY=";
        };
        aarch64-linux = {
          arch = "arm64";
          hash = "sha256-sxW7W85hxk8DxLS718fOSBJwuokc1Hsm4Oz/Bu2ZmD4=";
        };
      };
      forEach = f: nixpkgs.lib.genAttrs (builtins.attrNames assets) (system: f nixpkgs.legacyPackages.${system} assets.${system});
    in
    {
      packages = forEach (
        pkgs: asset:
        let
          meta = {
            description = "Calories, active minutes and heart rate on your wrist in every SteamVR game";
            homepage = "https://everyvr.net/sweatxr";
            license = pkgs.lib.licenses.gpl3Only;
            sourceProvenance = [ pkgs.lib.sourceTypes.binaryNativeCode ];
            platforms = builtins.attrNames assets;
            mainProgram = "sweatxr";
          };
          # The release as published, with its libraries pointed at the Nix store. The window system, Vulkan and
          # OpenXR (Monado, WiVRn) libraries are loaded at run time, so they go on the library path too.
          app = pkgs.stdenv.mkDerivation {
            pname = "sweatxr-app";
            inherit version meta;
            src = pkgs.fetchurl {
              url = "https://github.com/EncidXVII/SweatXR/releases/download/v${version}/sweatxr-linux-${asset.arch}.tar.gz";
              inherit (asset) hash;
            };
            nativeBuildInputs = [ pkgs.autoPatchelfHook ];
            buildInputs = [
              pkgs.stdenv.cc.cc.lib
              pkgs.fontconfig
              pkgs.dbus
            ];
            runtimeDependencies = [
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.vulkan-loader
              pkgs.openxr-loader
              pkgs.libGL
              (pkgs.libx11 or pkgs.xorg.libX11)
              (pkgs.libxcursor or pkgs.xorg.libXcursor)
              (pkgs.libxi or pkgs.xorg.libXi)
              (pkgs.libxcb or pkgs.xorg.libxcb)
            ];
            dontConfigure = true;
            dontBuild = true;
            installPhase = ''
              runHook preInstall
              mkdir -p $out/share/sweatxr
              cp -r sweatxr README.md LICENSE NOTICE THIRD_PARTY_NOTICES.md uninstall.sh $out/share/sweatxr/
              runHook postInstall
            '';
          };
          launcher = pkgs.writeShellApplication {
            name = "sweatxr";
            runtimeInputs = [ pkgs.coreutils ];
            inherit meta;
            text = ''
              dest="''${XDG_DATA_HOME:-$HOME/.local/share}/sweatxr"
              if [ "$(cat "$dest/.nix-package" 2>/dev/null || true)" != "${app}" ]; then
                mkdir -p "$dest"
                cp -R --remove-destination "${app}/share/sweatxr/." "$dest/"
                chmod -R u+w "$dest"
                echo "${app}" > "$dest/.nix-package"
              fi
              # The copy's libraries are in the Nix store: a GC root keeps them while the copy is there, or a
              # garbage collection after `nix run` would break the start with SteamVR.
              if command -v nix-store >/dev/null; then
                nix-store --add-root "$dest/.nix-gcroot" --realise "${app}" >/dev/null 2>&1 || true
              fi
              exec "$dest/sweatxr" "$@"
            '';
          };
          # The command plus the menu entry and icon from this repository.
          sweatxr = pkgs.runCommand "sweatxr-${version}" { inherit meta; } ''
            mkdir -p $out/share/applications $out/share/icons/hicolor/256x256/apps
            ln -s ${launcher}/bin $out/bin
            cp ${self}/packaging/sweatxr.desktop $out/share/applications/
            cp ${self}/packaging/sweatxr.png $out/share/icons/hicolor/256x256/apps/sweatxr.png
          '';
        in
        {
          inherit app sweatxr;
          default = sweatxr;
        }
      );
    };
}
