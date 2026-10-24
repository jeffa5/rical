{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    system = "x86_64-linux";
    pkgs = nixpkgs.legacyPackages.${system};
    lib = pkgs.lib;

    # rical (CLI) and rical-gtk (GUI)
    rical = pkgs.rustPlatform.buildRustPackage {
      pname = "rical";
      version = "0.1.0";
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./data
        ];
      };
      cargoLock = {
        lockFile = ./Cargo.lock;
        outputHashes = {
          "icalendar-0.17.5" = "sha256-ARg8hJNMCdUGIQHfxbCgJj/06wF1cQNg9BvDl+33lQc=";
        };
      };
      nativeBuildInputs = [pkgs.pkg-config pkgs.wrapGAppsHook4];
      buildInputs = [pkgs.gtk4 pkgs.libadwaita];
      # the CLI tests format times in a fixed zone
      nativeCheckInputs = [pkgs.tzdata];
      preCheck = "export TZDIR=${pkgs.tzdata}/share/zoneinfo";
      postInstall = ''
        install -Dm644 data/net.jeffas.Rical.desktop -t $out/share/applications
      '';
      meta = {
        description = "Calendar and tasks over a vdir of .ics files";
        mainProgram = "rical-gtk";
        license = lib.licenses.mit;
        platforms = lib.platforms.linux;
      };
    };
  in {
    packages.${system} = {
      inherit rical;
      default = rical;
    };

    # pkgs.rical, as nix-home consumes papers and tasknet
    overlays.default = final: prev: {
      rical = self.packages.${final.stdenv.hostPlatform.system}.default;
    };

    # programs.rical: install rical, point it at a vdir, set key bindings.
    homeManagerModules.default = {
      config,
      lib,
      pkgs,
      ...
    }: let
      cfg = config.programs.rical;
    in {
      options.programs.rical = {
        enable = lib.mkEnableOption "rical, a calendar for a vdir";
        package = lib.mkOption {
          type = lib.types.package;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          description = "The rical package.";
        };
        vdir = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "~/calendar/nextcloud";
          description = "Root of the vdir (a folder of calendar folders) rical-gtk opens.";
        };
        calendar = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "personal";
          description = "Calendar (folder name) new events go in by default.";
        };
        binds = lib.mkOption {
          type = lib.types.lines;
          default = "";
          example = ''
            [global]
            <C-t> = :today<Enter>
          '';
          description = "Extra key bindings, in binds.conf format, layered over the defaults.";
        };
      };

      config = lib.mkIf cfg.enable {
        home.packages = [
          (pkgs.symlinkJoin {
            name = "rical-configured";
            paths = [cfg.package];
            nativeBuildInputs = [pkgs.makeWrapper];
            postBuild = ''
              wrapProgram $out/bin/rical-gtk \
                ${lib.optionalString (cfg.vdir != null) "--set-default RICAL_VDIR ${lib.escapeShellArg (builtins.replaceStrings ["~/"] ["${config.home.homeDirectory}/"] cfg.vdir)}"} \
                ${lib.optionalString (cfg.calendar != null) "--set-default RICAL_CALENDAR ${lib.escapeShellArg cfg.calendar}"}
            '';
          })
        ];
        xdg.configFile."rical/binds.conf" = lib.mkIf (cfg.binds != "") {
          text = cfg.binds;
        };
      };
    };

    devShells.${system}.default = pkgs.mkShell {
      packages = [
        pkgs.rust-analyzer
        pkgs.rustc
        pkgs.cargo
        pkgs.rustfmt
        pkgs.clippy

        # rical-gtk
        pkgs.pkg-config
        pkgs.gtk4
        pkgs.libadwaita

        # headless GUI screenshots (scripts/screenshot.sh)
        pkgs.sway
        pkgs.grim
        pkgs.wtype
      ];
      # icons and GSettings schemas for running rical-gtk from the shell
      shellHook = ''
        export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}:${pkgs.adwaita-icon-theme}/share:${pkgs.hicolor-icon-theme}/share:$XDG_DATA_DIRS
      '';
    };
  };
}
