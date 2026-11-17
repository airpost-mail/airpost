# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Airpost contributors
#
#   nix run github:airpost-mail/airpost -- status
#   nix profile install github:airpost-mail/airpost
#
# The package is distribution/nix/package.nix.
{
  description = "Airpost: mail engine, command-line client, MCP server and web gateway";

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
        airpost = pkgs.callPackage ./distribution/nix/package.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.airpost;
      });
    };
}
