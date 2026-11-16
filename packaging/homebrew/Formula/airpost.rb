# frozen_string_literal: true

# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Tern contributors
#
# The ternmail/homebrew-tap formula, installing the release archives (packaging/archive.sh).
# packaging/bump.sh sets the version and checksums from a release's SHA256SUMS.
class Tern < Formula
  desc "Mail engine, command-line client, MCP server and web gateway for Tern"
  homepage "https://github.com/ternmail/ternmail"
  license "GPL-3.0-only"

  livecheck do
    url :homepage
    regex(/^v(\d+(?:\.\d+)+)$/i)
  end

  on_macos do
    on_arm do
      url "https://github.com/ternmail/ternmail/releases/download/v0.1.0/tern-0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/ternmail/ternmail/releases/download/v0.1.0/tern-0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/ternmail/ternmail/releases/download/v0.1.0/tern-0.1.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/ternmail/ternmail/releases/download/v0.1.0/tern-0.1.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "bin/tern", "bin/tern-engine", "bin/tern-mcp", "bin/tern-gateway"
    (share/"tern-gateway").install "share/tern-gateway/web"
    doc.install Dir["share/doc/tern/*"]
  end

  # No `service do`: `tern engine install` writes and owns the engine's LaunchAgent or systemd
  # user unit (and the gateway's and service plugins' units, which start after it). A brew
  # services unit as well would be a second owner starting a second engine on the same profile.
  def caveats
    <<~EOS
      Run the engine in the background, started at login:
        tern engine install
      then add an account:
        tern account add you@example.com

      tern manages the engine's unit itself; don't use `brew services` for Tern.
      After `brew upgrade tern`, run `tern engine install` and `tern engine restart`
      so the unit runs the new engine.

      The web client for tern-gateway is in:
        #{opt_share}/tern-gateway/web
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/tern --version")
    assert_match version.to_s, shell_output("#{bin}/tern-mcp --version")
    assert_match version.to_s, shell_output("#{bin}/tern-gateway --version")
    (testpath/"profile").mkpath
    with_env(CONFIG_DIR_PATH: testpath/"profile") do
      system bin/"tern-engine", "--mode", "migrate"
    end
    assert_path_exists testpath/"profile/tern.db"
    assert_path_exists share/"tern-gateway/web/index.html"
  end
end
