# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Tern contributors
#
# tern, tern-engine, tern-mcp and tern-gateway built from source, with the web client the
# gateway serves in $out/share/tern-gateway/web (the release archives' layout). Used by the
# repo's flake.nix. The crates and npm packages come pinned from Cargo.lock and
# apps/web/package-lock.json, whose licence files are in the build's vendored sources; the
# THIRD-PARTY-NOTICES files of the release archives (cargo-about) aren't generated here.
{
  lib,
  rustPlatform,
  buildNpmPackage,
  importNpmLock,
  nodejs_22,
}:

let
  root = ../..;
  version = (lib.importTOML (root + "/Cargo.toml")).workspace.package.version;

  web = buildNpmPackage {
    pname = "tern-web";
    inherit version;
    src = lib.fileset.toSource {
      root = root + "/apps/web";
      fileset = lib.fileset.difference (root + "/apps/web") (
        lib.fileset.maybeMissing (root + "/apps/web/node_modules")
      );
    };
    nodejs = nodejs_22;
    npmDeps = importNpmLock { npmRoot = root + "/apps/web"; };
    npmConfigHook = importNpmLock.npmConfigHook;
    env.VITE_TERN_ENGINE = "gateway";
    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  };
in
rustPlatform.buildRustPackage {
  pname = "tern";
  inherit version;

  # Every workspace member must be present for cargo to read the workspace.
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/apps/backend")
      (root + "/apps/cli")
      (root + "/apps/engine/Cargo.toml")
      (root + "/apps/engine/src")
      (root + "/apps/gateway")
      (root + "/apps/mcp")
      (root + "/crates")
      (root + "/examples/service-plugin")
      # Built into `tern` (`tern skill install`).
      (root + "/skills/tern")
    ];
  };

  cargoLock.lockFile = root + "/Cargo.lock";
  cargoBuildFlags = [
    "-p"
    "tern"
    "-p"
    "tern-engine"
    "-p"
    "tern-mcp"
    "-p"
    "tern-gateway"
  ];
  # The workspace tests need the Python harness and network servers (apps/engine/test).
  doCheck = false;

  postInstall = ''
    mkdir -p $out/share/tern-gateway $out/share/doc/tern
    cp -r ${web} $out/share/tern-gateway/web
    cp ${root + "/LICENSE.md"} ${root + "/NOTICE.md"} $out/share/doc/tern/
    cp ${root + "/assets/brand/LICENSE.md"} $out/share/doc/tern/TRADEMARKS.md
  '';

  passthru = { inherit web; };

  meta = {
    description = "Mail engine, command-line client, MCP server and web gateway for Tern";
    homepage = "https://github.com/ternmail/ternmail";
    license = lib.licenses.gpl3Only;
    mainProgram = "tern";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
