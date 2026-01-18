class Rung < Formula
  desc "Git workflow tool for managing stacked PRs"
  homepage "https://github.com/auswm85/rung"
  url "https://github.com/auswm85/rung/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "db26cb7fcfc954cdf5821c799f730e1b435d109005a6b9549a4c293be37e3028"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/rung-cli")
  end

  test do
    # Test version output
    assert_match "rung #{version}", shell_output("#{bin}/rung --version")

    # Test that it recognizes a non-rung repo gracefully
    system "git", "init", "test-repo"
    cd "test-repo" do
      output = shell_output("#{bin}/rung status 2>&1", 1)
      assert_match(/not initialized/i, output)
    end
  end
end
