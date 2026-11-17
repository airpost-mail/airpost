# frozen_string_literal: true

# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Airpost contributors
#
# The airpost-mail/homebrew-tap cask for the desktop app: the zips desktop-build-macos.yml builds
# (Airpost-AppleSilicon.zip, Airpost.zip for Intel), from a `desktop-v<version>` GitHub release.
# scripts/bump.sh --cask sets the version and checksums.
cask "airpost" do
  arch arm: "-AppleSilicon"

  version "0.1.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000",
         intel: "1111111111111111111111111111111111111111111111111111111111111111"

  url "https://github.com/airpost-mail/airpost/releases/download/desktop-v#{version}/Airpost#{arch}.zip"
  name "Airpost"
  desc "Email client with a local sync engine"
  homepage "https://github.com/airpost-mail/airpost"

  livecheck do
    url :homepage
    regex(/^desktop-v(\d+(?:\.\d+)+)$/i)
  end

  depends_on macos: :monterey

  app "Airpost.app"

  zap trash: [
    "~/Library/Application Support/Airpost",
    "~/Library/Caches/com.airpost.airpost",
    "~/Library/Caches/com.airpost.airpost.ShipIt",
    "~/Library/HTTPStorages/com.airpost.airpost",
    "~/Library/Preferences/com.airpost.airpost.plist",
    "~/Library/Saved Application State/com.airpost.airpost.savedState",
  ]

  # The app bundles the airpost command and links it from Preferences > General; the `airpost`
  # formula installs the same programs standalone.
  caveats <<~EOS
    To use the airpost command, turn it on in Airpost's Preferences > General,
    or install it on its own with:
      brew install airpost-mail/tap/airpost
  EOS
end
