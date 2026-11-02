class Townjump < Formula
  desc "Unofficial Rust port of Holesail: peer-to-peer TCP/UDP tunnels over HyperDHT"
  homepage "https://github.com/mesh-town/townjump"
  # The checksum list of the release pins the archives below.
  url "https://github.com/mesh-town/townjump/releases/download/v0.1.7/SHA256SUMS"
  # placeholder hash
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "AGPL-3.0-only"

  depends_on :macos

  resource "binary" do
    on_arm do
      url "https://github.com/mesh-town/townjump/releases/download/v0.1.7/townjump-0.1.7-aarch64-apple-darwin.tar.gz"
      # placeholder hash
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end

    on_intel do
      url "https://github.com/mesh-town/townjump/releases/download/v0.1.7/townjump-0.1.7-x86_64-apple-darwin.tar.gz"
      # placeholder hash
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    resource("binary").stage { bin.install "townjump" }
  end

  def caveats
    <<~EOS
      This package installs a command named `townjump`, so it does not
      collide with the npm package `holesail` (`npm i -g holesail`). The help
      and messages it prints still say `holesail`, as in the original.
      Unofficial Rust port, not affiliated with Holesail / LIVEPORT.
    EOS
  end

  test do
    assert_match "Holesail CLI", shell_output("#{bin}/townjump --help")
  end
end
