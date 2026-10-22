class Townjump < Formula
  desc "Unofficial Rust port of Holesail: peer-to-peer TCP/UDP tunnels over HyperDHT"
  homepage "https://github.com/mesh-town/townjump"
  url "https://github.com/mesh-town/townjump/archive/refs/tags/v0.1.0.tar.gz"
  # Fill in after the release: shasum -a 256 <tarball>
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "AGPL-3.0-only"
  head "https://github.com/mesh-town/townjump.git", branch: "main"

  depends_on "rust" => :build

  def install
    # std_cargo_args already passes --locked, --root and --path.
    system "cargo", "install", *std_cargo_args(path: "crates/townjump")
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
    output = shell_output("#{bin}/townjump --live abc", 2)
    assert_match "Given port is not a valid number", output
  end
end
