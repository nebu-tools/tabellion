# Picks the Rust target for the Rtools toolchain: x64, x64 with Clang,
# or ARM64. Called from src/Makevars.win; see rextendr's config.R and
# CRAN hellorust's tools/rustarch.R.
target <- if (grepl("aarch", R.version$platform)) {
  "aarch64-pc-windows-gnullvm"
} else if (grepl("clang", Sys.getenv("R_COMPILED_BY"))) {
  "x86_64-pc-windows-gnullvm"
} else {
  "x86_64-pc-windows-gnu"
}
cat(target)
