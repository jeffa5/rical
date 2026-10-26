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

    # Every crate is its own derivation (crate2nix), so a change to rical
    # rebuilds only rical's crates; dependencies come from the store.
    # Regenerate Cargo.nix after changing dependencies: `crate2nix generate`
    # (the pre-commit hook checks it is current).
    cargoNix = import ./Cargo.nix {
      inherit pkgs;
      defaultCrateOverrides =
        pkgs.defaultCrateOverrides
        // {
          # not in nixpkgs' overrides
          libadwaita-sys = _: {
            nativeBuildInputs = [pkgs.pkg-config];
            buildInputs = [pkgs.libadwaita pkgs.gtk4];
          };
        };
    };

    # rical (CLI) and rical-gtk (GUI)
    rical = pkgs.stdenv.mkDerivation {
      pname = "rical";
      version = "0.1.0";
      dontUnpack = true;
      nativeBuildInputs = [pkgs.wrapGAppsHook4];
      # the icon theme too: without it, a session that provides none (sway)
      # shows broken-image icons for those GTK does not build in. After the
      # session's own, so a theme chosen there still wins.
      preFixup = ''
        gappsWrapperArgs+=(--suffix XDG_DATA_DIRS : "${pkgs.adwaita-icon-theme}/share:${pkgs.hicolor-icon-theme}/share")
      '';
      buildInputs = [pkgs.gtk4 pkgs.libadwaita];
      installPhase = ''
        mkdir -p $out/bin
        cp ${cargoNix.workspaceMembers.rical.build}/bin/rical $out/bin/
        cp ${cargoNix.workspaceMembers.rical-gtk.build}/bin/rical-gtk $out/bin/
        install -Dm644 ${./data/net.jeffas.Rical.desktop} $out/share/applications/net.jeffas.Rical.desktop
        install -Dm644 ${./data/icons/hicolor/scalable/apps/net.jeffas.Rical.svg} $out/share/icons/hicolor/scalable/apps/net.jeffas.Rical.svg
        install -Dm644 ${./data/icons/hicolor/symbolic/apps/net.jeffas.Rical-symbolic.svg} $out/share/icons/hicolor/symbolic/apps/net.jeffas.Rical-symbolic.svg
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
      vdirPath =
        if cfg.vdir == null
        then null
        else builtins.replaceStrings ["~/"] ["${config.home.homeDirectory}/"] cfg.vdir;
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
        week = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "workweek";
          description = "What the week view shows: fullweek, workweek or Ndays (e.g. 3days).";
        };
        remind = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "10m";
          description = "Remind this long before timed events without alarms of their own.";
        };
        reminders = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Run `rical remind` as a user service so reminders arrive without rical-gtk open (rical-gtk then leaves reminders to it).";
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
        settings = lib.mkOption {
          type = with lib.types; attrsOf (oneOf [str bool int (listOf str)]);
          default = {};
          example = {
            me = ["me@example.com"];
            contacts = "~/contacts/nextcloud";
            view = "3days";
          };
          description = "Settings for ~/.config/rical/config: any rical-gtk flag by its long name.";
        };
        openCalendarFiles = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Make rical-gtk the default for .ics files (text/calendar), so mail clients open invitations in it.";
        };
        extraConfig = lib.mkOption {
          type = lib.types.lines;
          default = "";
          example = "include-if-exists = local.conf";
          description = "Lines added to ~/.config/rical/config after `settings`, e.g. includes.";
        };
      };

      config = lib.mkIf cfg.enable {
        # settings go in the config file rather than the environment, so
        # rical-gtk's Preferences can override them (in local.conf, as
        # this file is read-only)
        home.packages = [cfg.package];
        assertions = [
          {
            assertion = cfg.reminders -> cfg.vdir != null;
            message = "programs.rical.reminders needs programs.rical.vdir";
          }
        ];
        systemd.user.services.rical-remind = lib.mkIf cfg.reminders {
          Unit = {
            Description = "rical reminders";
            PartOf = ["graphical-session.target"];
            After = ["graphical-session.target"];
          };
          Service = {
            ExecStart = lib.escapeShellArgs (
              ["${cfg.package}/bin/rical" "remind" vdirPath]
              ++ lib.optionals (cfg.remind != null) ["--remind" cfg.remind]
            );
            Restart = "on-failure";
          };
          Install.WantedBy = ["graphical-session.target"];
        };
        xdg.mimeApps.defaultApplications = lib.mkIf cfg.openCalendarFiles {
          "text/calendar" = ["net.jeffas.Rical.desktop"];
          "application/ics" = ["net.jeffas.Rical.desktop"];
        };
        xdg.configFile."rical/binds.conf" = lib.mkIf (cfg.binds != "") {
          text = cfg.binds;
        };
        xdg.configFile."rical/config" = let
          # the named options, then `settings` (which can override them)
          named = lib.filterAttrs (_: v: v != null) {
            vdir = vdirPath;
            inherit (cfg) calendar remind week;
            no-notifications =
              if cfg.reminders
              then true
              else null;
          };
          all = named // cfg.settings;
        in
          lib.mkIf (all != {} || cfg.extraConfig != "") {
            text = let
              value = v:
                if builtins.isBool v
                then lib.boolToString v
                else if builtins.isList v
                then lib.concatStringsSep ", " v
                else toString v;
            in
              lib.concatStrings (lib.mapAttrsToList (k: v: "${k} = ${value v}\n") all)
              + cfg.extraConfig;
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
        # regenerate Cargo.nix after changing dependencies
        pkgs.crate2nix

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
