# Homebrew formula for Vertion.
#
# This file is a template. Homebrew reads formulae from a tap repository, not
# from this one, so to use it:
#
#   1. Create a repo named `homebrew-tap` under your GitHub account.
#   2. Copy this file to `Formula/vertion.rb` in that repo.
#   3. Fill in the four `sha256` values from the release's `.sha256` files.
#   4. `brew install vertX-dev/tap/vertion`
#
# The `version` and every `sha256` must be updated on each release. `brew
# bump-formula-pr` automates that once the formula is live.

class Vertion < Formula
  desc "Filter source files by version markers and emit a per-version build tree"
  homepage "https://github.com/vertX-dev/vertion"
  version "1.0.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/vertX-dev/vertion/releases/download/v1.0.0/vertion-v1.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_APPLE_DARWIN_SHA256"
    end
    on_intel do
      url "https://github.com/vertX-dev/vertion/releases/download/v1.0.0/vertion-v1.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_APPLE_DARWIN_SHA256"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/vertX-dev/vertion/releases/download/v1.0.0/vertion-v1.0.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_X86_64_LINUX_GNU_SHA256"
    end
  end

  def install
    bin.install "vertion"

    # Generated from the binary rather than shipped in the archive, so they can
    # never describe a different version than the one being installed.
    generate_completions_from_executable(bin/"vertion", "completions")
    (man1/"vertion.1").write Utils.safe_popen_read(bin/"vertion", "man")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/vertion --version")

    # A real build, so the test covers more than argument parsing.
    (testpath/"vertion.cfg").write <<~CFG
      [project]
      version = "1.0.0"
      input = "./src"
      output = "./build"
      ignore = ["./build"]
    CFG
    (testpath/"src").mkpath
    (testpath/"src/app.js").write <<~JS
      const base = 1;
      //version 2.0 *
      const later = 2;
      //version 2.0 *
    JS

    system bin/"vertion", "build", "-v", "1.0"
    built = (testpath/"build/1.0.0/app.js").read
    assert_match "const base = 1;", built
    refute_match "const later = 2;", built
  end
end
