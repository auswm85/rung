class Rung < Formula
  desc "Git workflow tool for managing stacked PRs"
  homepage "https://github.com/auswm85/rung"
  url "https://github.com/auswm85/rung/archive/refs/tags/v0.1.3.tar.gz"
  sha256 "6d3c6d5f220ba6021a046011ecd82085dfd385ca2fa89a525843c88079e331c7"
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
