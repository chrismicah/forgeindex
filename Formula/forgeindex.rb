class Forgeindex < Formula
  desc "AST-driven codebase intelligence MCP server for agentic workflows"
  homepage "https://github.com/chrismicah/forgeindex"
  url "https://github.com/chrismicah/forgeindex.git", tag: "v0.1.0"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "forgeindex", shell_output("#{bin}/forgeindex --version")
  end
end
