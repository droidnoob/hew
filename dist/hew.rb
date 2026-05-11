class Hew < Formula
  desc "Beads-powered methodology for AI coding agents"
  homepage "https://github.com/droidnoob/hew"
  license "MIT"

  # Real version, sha256, and url are filled in by cargo-dist (`dist`)
  # when a release tag is pushed. This stub committed for visibility;
  # don't edit by hand.
  version "0.0.0"
  url "https://github.com/droidnoob/hew/archive/refs/tags/v0.0.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  depends_on "rust" => :build
  # Beads is the runtime requirement; `hew init` warns if it is missing.
  depends_on "beads" => :recommended

  def install
    system "cargo", "install", *std_cargo_args(path: "hew")
  end

  test do
    assert_match "hew #{version}", shell_output("#{bin}/hew --version")
  end
end
