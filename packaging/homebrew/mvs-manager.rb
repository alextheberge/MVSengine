# TEMPLATE — not yet submitted to any tap. See ../README.md before using.
class MvsManager < Formula
  desc "Multidimensional (ARCH.FEAT.PROT.FIX-CONT) post-SemVer versioning CLI"
  homepage "https://github.com/alextheberge/MVSengine"
  version "0.0.0" # fill in from the release tag, e.g. "2.5.1"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/alextheberge/MVSengine/releases/download/v#{version}/mvs-manager-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/alextheberge/MVSengine/releases/download/v#{version}/mvs-manager-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/alextheberge/MVSengine/releases/download/v#{version}/mvs-manager-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/alextheberge/MVSengine/releases/download/v#{version}/mvs-manager-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  depends_on "git" => :recommended # `migrate backfill`/`detect` shell out to git when present

  def install
    bin.install "mvs-manager"
  end

  test do
    assert_match "mvs-manager", shell_output("#{bin}/mvs-manager --version")
  end
end
