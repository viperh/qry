# Homebrew formula for the viperh/tap tap: `brew install viperh/tap/qry`.
#
# The version, URLs and checksums here are placeholders: CI
# (scripts/update-packaging.py) rewrites them for each release and pushes
# the result to the tap. Do not edit them by hand.
class Qry < Formula
  desc "Terminal SQL client for PostgreSQL, MySQL, MariaDB, SQLite and Oracle"
  homepage "https://github.com/viperh/qry"
  version "0.0.0"
  license "MIT"

  on_macos do
    # One universal binary for x86_64 and arm64.
    url "https://github.com/viperh/qry/releases/download/v0.0.0/qry-0.0.0-macos-universal.tar.gz"
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  end

  on_linux do
    on_intel do
      url "https://github.com/viperh/qry/releases/download/v0.0.0/qry-0.0.0-linux-x86_64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_arm do
      url "https://github.com/viperh/qry/releases/download/v0.0.0/qry-0.0.0-linux-aarch64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "qry"
    man1.install "man/qry.1"
    bash_completion.install "completions/qry.bash" => "qry"
    zsh_completion.install "completions/_qry"
    fish_completion.install "completions/qry.fish"
    doc.install "README.md", "CHANGELOG.md"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/qry --version")
    assert_match "QRY_CONFIG", shell_output("#{bin}/qry generate man")
  end
end
