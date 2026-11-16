# frozen_string_literal: true

# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Tern contributors
#
# The ternmail/homebrew-tap cask for the desktop app: the zips desktop-build-macos.yml builds
# (Tern-AppleSilicon.zip, Tern.zip for Intel), from a `desktop-v<version>` GitHub release.
# packaging/bump.sh --cask sets the version and checksums.
cask "tern" do
  arch arm: "-AppleSilicon"

  version "1.26.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000",
         intel: "1111111111111111111111111111111111111111111111111111111111111111"

  url "https://github.com/ternmail/ternmail/releases/download/desktop-v#{version}/Tern#{arch}.zip"
  name "Tern"
  desc "Email client with a local sync engine"
  homepage "https://github.com/ternmail/ternmail"

  livecheck do
    url :homepage
    regex(/^desktop-v(\d+(?:\.\d+)+)$/i)
  end

  depends_on macos: :monterey

  app "Tern.app"

  zap trash: [
    "~/Library/Application Support/Tern",
    "~/Library/Caches/com.ternmail.tern",
    "~/Library/Caches/com.ternmail.tern.ShipIt",
    "~/Library/HTTPStorages/com.ternmail.tern",
    "~/Library/Preferences/com.ternmail.tern.plist",
    "~/Library/Saved Application State/com.ternmail.tern.savedState",
  ]

  # The app bundles the tern command and links it from Preferences > General; the `tern`
  # formula installs the same programs standalone.
  caveats <<~EOS
    To use the tern command, turn it on in Tern's Preferences > General,
    or install it on its own with:
      brew install ternmail/tap/tern
  EOS
end
