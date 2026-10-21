# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Tern contributors
#
#   nix run github:ternmail/ternmail -- status
#   nix profile install github:ternmail/ternmail
#
# The package is packaging/nix/package.nix.
{
  description = "Tern: mail engine, command-line client, MCP server and web gateway";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        tern = pkgs.callPackage ./packaging/nix/package.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.tern;
      });
    };
}
