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
          # the mail app's HTML view (WebKitGTK for GTK 4)
          webkit6-sys = _: {
            nativeBuildInputs = [pkgs.pkg-config];
            buildInputs = [pkgs.webkitgtk_6_0 pkgs.gtk4 pkgs.libsoup_3];
          };
          javascriptcore6-sys = _: {
            nativeBuildInputs = [pkgs.pkg-config];
            buildInputs = [pkgs.webkitgtk_6_0];
          };
          soup3-sys = _: {
            nativeBuildInputs = [pkgs.pkg-config];
            buildInputs = [pkgs.libsoup_3];
          };
        };
    };

    # rical (CLI), rical-mail-queue (send queue), rical-calendar, rical-contacts and rical-mail (GUIs)
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
      buildInputs = [pkgs.gtk4 pkgs.libadwaita pkgs.webkitgtk_6_0];
      installPhase = ''
        mkdir -p $out/bin
        cp ${cargoNix.workspaceMembers.rical.build}/bin/rical $out/bin/
        cp ${cargoNix.workspaceMembers.rical-calendar.build}/bin/rical-calendar $out/bin/
        cp ${cargoNix.workspaceMembers.rical-contacts.build}/bin/rical-contacts $out/bin/
        cp ${cargoNix.workspaceMembers.rical-mail-queue.build}/bin/rical-mail-queue $out/bin/
        cp ${cargoNix.workspaceMembers.rical-mail.build}/bin/rical-mail $out/bin/
        install -Dm644 ${./data/net.jeffas.Rical.Mail.desktop} $out/share/applications/net.jeffas.Rical.Mail.desktop
        install -Dm644 ${./data/icons/hicolor/scalable/apps/net.jeffas.Rical.Mail.svg} $out/share/icons/hicolor/scalable/apps/net.jeffas.Rical.Mail.svg
        install -Dm644 ${./data/icons/hicolor/symbolic/apps/net.jeffas.Rical.Mail-symbolic.svg} $out/share/icons/hicolor/symbolic/apps/net.jeffas.Rical.Mail-symbolic.svg
        install -Dm644 ${./data/net.jeffas.Rical.Contacts.desktop} $out/share/applications/net.jeffas.Rical.Contacts.desktop
        install -Dm644 ${./data/icons/hicolor/scalable/apps/net.jeffas.Rical.Contacts.svg} $out/share/icons/hicolor/scalable/apps/net.jeffas.Rical.Contacts.svg
        install -Dm644 ${./data/icons/hicolor/symbolic/apps/net.jeffas.Rical.Contacts-symbolic.svg} $out/share/icons/hicolor/symbolic/apps/net.jeffas.Rical.Contacts-symbolic.svg
        install -Dm644 ${./data/net.jeffas.Rical.desktop} $out/share/applications/net.jeffas.Rical.desktop
        install -Dm644 ${./data/icons/hicolor/scalable/apps/net.jeffas.Rical.svg} $out/share/icons/hicolor/scalable/apps/net.jeffas.Rical.svg
        install -Dm644 ${./data/icons/hicolor/symbolic/apps/net.jeffas.Rical-symbolic.svg} $out/share/icons/hicolor/symbolic/apps/net.jeffas.Rical-symbolic.svg
      '';
      meta = {
        description = "Calendar and tasks over a vdir of .ics files, and contacts over vCards";
        mainProgram = "rical-calendar";
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
      # ~/.config/rical-mail-queue/config; empty leaves a hand-written one be
      mailQueueConfig = let
        q = cfg.mailQueue;
        set = lib.filterAttrs (_: v: v != null) {
          inherit (q) queue sent send timeout poll;
          max-attempts = q.maxAttempts;
          retry-delay = q.retryDelay;
        };
      in
        lib.concatStrings (
          lib.mapAttrsToList (k: v: "${k} = ${toString v}\n") set
          ++ map (r: "send-from ${r.from} = ${r.command}\n") q.sendFrom
        )
        + q.extraConfig;
      settingValue = v:
        if builtins.isBool v
        then lib.boolToString v
        else if builtins.isList v
        then lib.concatStringsSep ", " v
        else toString v;
      # ~/.config/rical-mail/config: what the rest of the module knows (the
      # vdir, the queue, the contacts), the named options, then `settings`
      mailConfig = let
        m = cfg.mail;
        shared = lib.filterAttrs (_: v: v != null) {
          vdir = vdirPath;
          inherit (cfg.mailQueue) queue;
          contacts = cfg.settings.contacts or null;
        };
        named = lib.filterAttrs (_: v: v != null && v != []) {
          inherit (m) mail me;
        };
        all = shared // named // m.settings;
      in
        lib.concatStrings (
          lib.mapAttrsToList (k: v: "${k} = ${settingValue v}\n") all
          ++ lib.mapAttrsToList (a: v: "account ${a} = ${settingValue v}\n") m.accounts
        )
        + m.extraConfig;
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
          description = "Root of the vdir (a folder of calendar folders) rical-calendar opens.";
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
          description = "Run `rical remind` as a user service so reminders arrive without rical-calendar open (rical-calendar then leaves reminders to it).";
        };
        # `mailQueue = true` still works: it is `mailQueue.enable = true`
        mailQueue = lib.mkOption {
          default = {};
          description = "The send queue's worker, `rical-mail-queue run`, as a user service: it watches the queue and sends each message as it arrives.";
          type = with lib.types;
            coercedTo bool (enable: {inherit enable;}) (submodule {
              options = {
                enable = lib.mkEnableOption "the rical-mail-queue worker as a user service";
                send = lib.mkOption {
                  type = nullOr str;
                  default = null;
                  example = "msmtp --read-envelope-from -- $RICAL_MAIL_QUEUE_RECIPIENTS";
                  description = "The command that sends: the message on stdin, the envelope in RICAL_MAIL_QUEUE_*. Without it (or sendFrom), the service stops at once and nothing is sent.";
                };
                sendFrom = lib.mkOption {
                  type = listOf (submodule {
                    options = {
                      from = lib.mkOption {
                        type = str;
                        example = "*@work.example";
                        description = "Sender pattern: `*` is any run of characters, case ignored.";
                      };
                      command = lib.mkOption {
                        type = str;
                        example = "msmtp -a work -- $RICAL_MAIL_QUEUE_RECIPIENTS";
                        description = "The command for senders that match.";
                      };
                    };
                  });
                  default = [];
                  description = "Commands by sender, the first match winning; `send` is the fallback.";
                };
                queue = lib.mkOption {
                  type = nullOr str;
                  default = null;
                  example = "~/.local/share/rical-mail-queue/outbox";
                  description = "The queue folder (default ~/.local/share/rical-mail-queue/outbox). Keep it out of any folder a mail sync touches; set rical-mail's `queue` to the same.";
                };
                sent = lib.mkOption {
                  type = nullOr str;
                  default = null;
                  example = "~/mail/Sent";
                  description = "A Maildir for copies of what was sent (default sent/ in the queue).";
                };
                timeout = lib.mkOption {
                  type = nullOr str;
                  default = null;
                  example = "2m";
                  description = "How long the send command has (default 60s).";
                };
                maxAttempts = lib.mkOption {
                  type = nullOr ints.positive;
                  default = null;
                  description = "Attempts before a message moves to failed/ (default 8).";
                };
                retryDelay = lib.mkOption {
                  type = nullOr str;
                  default = null;
                  example = "1m";
                  description = "Wait after the first failure, doubling each time up to an hour (default 30s).";
                };
                poll = lib.mkOption {
                  type = nullOr str;
                  default = null;
                  description = "The longest it waits before looking at the queue again (default 1m); arrivals wake it at once.";
                };
                packages = lib.mkOption {
                  type = listOf package;
                  default = [];
                  example = lib.literalExpression "[ pkgs.msmtp pkgs.pass ]";
                  description = "Put on the service's PATH for the send command, ahead of the profile's.";
                };
                verbose = lib.mkOption {
                  type = bool;
                  default = true;
                  description = "Log what happens to each message to the journal.";
                };
                extraConfig = lib.mkOption {
                  type = lines;
                  default = "";
                  description = "Lines added to ~/.config/rical-mail-queue/config after the options above.";
                };
              };
            });
        };
        # rical-mail: its own config file, sharing what the module knows
        mail = {
          enable = lib.mkEnableOption "managing rical-mail's settings (~/.config/rical-mail/config)";
          mail = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "~/mail";
            description = "The folder of Maildir folders (default ~/mail or ~/Maildir, whichever exists).";
          };
          me = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
            example = ["Me <me@example.com>"];
            description = "Your addresses, `Name <address>`; the first is what new mail is from.";
          };
          accounts = lib.mkOption {
            type = with lib.types; attrsOf (listOf str);
            default = {};
            example = {work = ["Me <me@work.example>"];};
            description = "Addresses by account: mail from one keeps its draft and sent copy in that account.";
          };
          settings = lib.mkOption {
            type = with lib.types; attrsOf (oneOf [str bool int (listOf str)]);
            default = {};
            example = {
              markdown = true;
              read-after = "2s";
            };
            description = "Any other rical-mail setting by name. `vdir`, `queue` and `contacts` default to programs.rical.vdir, mailQueue.queue and settings.contacts.";
          };
          binds = lib.mkOption {
            type = lib.types.lines;
            default = "";
            description = "Extra key bindings, written to ~/.config/rical-mail/binds.conf.";
          };
          extraConfig = lib.mkOption {
            type = lib.types.lines;
            default = "";
            example = "include-if-exists = private.conf";
            description = "Lines added to ~/.config/rical-mail/config after the rest, e.g. includes.";
          };
          openMailto = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Make rical-mail the handler for mailto: links.";
          };
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
          description = "Settings for ~/.config/rical-calendar/config: any rical-calendar flag by its long name.";
        };
        openCalendarFiles = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Make rical-calendar the default for .ics files (text/calendar), so mail clients open invitations in it.";
        };
        openContactFiles = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Make rical-contacts the default for .vcf files (text/vcard), so a contact opened elsewhere is shown, or added, in it.";
        };
        extraConfig = lib.mkOption {
          type = lib.types.lines;
          default = "";
          example = "include-if-exists = local.conf";
          description = "Lines added to ~/.config/rical-calendar/config after `settings`, e.g. includes.";
        };
      };

      config = lib.mkIf cfg.enable {
        # settings go in the config file rather than the environment, so
        # rical-calendar's Preferences can override them (in local.conf, as
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
        xdg.configFile."rical-mail-queue/config" = lib.mkIf (cfg.mailQueue.enable && mailQueueConfig != "") {
          text = mailQueueConfig;
        };
        systemd.user.services.rical-mail-queue = lib.mkIf cfg.mailQueue.enable {
          Unit = {
            Description = "rical mail queue: sends what is queued as it arrives";
            Documentation = ["https://github.com/jeffa5/rical/blob/main/docs/mail-queue.md"];
            # picks up changed settings on a switch
            X-Restart-Triggers = [(builtins.hashString "sha256" mailQueueConfig)];
          };
          Service = {
            ExecStart = lib.escapeShellArgs (
              ["${cfg.package}/bin/rical-mail-queue"]
              ++ lib.optional cfg.mailQueue.verbose "-v"
              ++ ["run"]
            );
            # the send command runs under sh, finding what it names here:
            # user services otherwise get only systemd's own PATH
            Environment = [
              "PATH=${lib.makeBinPath (cfg.mailQueue.packages ++ [pkgs.bash pkgs.coreutils])}:${config.home.profileDirectory}/bin:/run/current-system/sw/bin:/usr/bin:/bin"
            ];
            Restart = "on-failure";
            RestartSec = "30s";
            # 78 (EX_CONFIG): no send command; retrying would not help
            RestartPreventExitStatus = "78";
          };
          Install.WantedBy = ["default.target"];
        };
        xdg.mimeApps.defaultApplications = lib.mkMerge [
          (lib.mkIf cfg.openCalendarFiles {
            "text/calendar" = ["net.jeffas.Rical.desktop"];
            "application/ics" = ["net.jeffas.Rical.desktop"];
          })
          (lib.mkIf cfg.openContactFiles {
            "text/vcard" = ["net.jeffas.Rical.Contacts.desktop"];
            "text/x-vcard" = ["net.jeffas.Rical.Contacts.desktop"];
          })
          (lib.mkIf cfg.mail.openMailto {
            "x-scheme-handler/mailto" = ["net.jeffas.Rical.Mail.desktop"];
          })
        ];
        xdg.configFile."rical-mail/binds.conf" = lib.mkIf (cfg.mail.enable && cfg.mail.binds != "") {
          text = cfg.mail.binds;
        };
        xdg.configFile."rical-mail/config" = lib.mkIf (cfg.mail.enable && mailConfig != "") {
          text = mailConfig;
        };
        xdg.configFile."rical-calendar/binds.conf" = lib.mkIf (cfg.binds != "") {
          text = cfg.binds;
        };
        xdg.configFile."rical-calendar/config" = let
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
            text =
              lib.concatStrings (lib.mapAttrsToList (k: v: "${k} = ${settingValue v}\n") all)
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

        # rical-calendar
        pkgs.pkg-config
        pkgs.gtk4
        pkgs.libadwaita
        # rical-mail's HTML view
        pkgs.webkitgtk_6_0
        pkgs.libsoup_3

        # headless GUI screenshots (scripts/screenshot.sh)
        pkgs.sway
        pkgs.grim
        pkgs.wtype
      ];
      # icons and GSettings schemas for running rical-calendar from the shell
      shellHook = ''
        export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}:${pkgs.adwaita-icon-theme}/share:${pkgs.hicolor-icon-theme}/share:$XDG_DATA_DIRS
      '';
    };
  };
}
