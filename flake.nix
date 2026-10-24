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
  in {
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
