#!/bin/sh
# Stages a standalone release copy of the R package for CRAN-style source
# builds: copies the two unpublished engine crates into src/rust/local,
# rewrites the staged manifests to point at them, drops dev-dependencies,
# seeds the release lockfile from the workspace lock, and vendors the
# registry sources into src/rust/vendor.tar.xz for offline builds.
#
# Run from the repository root after committing the packaging files:
#   tools/stage-r.sh
# It prints the staged directory; build it with e.g.
#   R CMD build "$STAGE/tabellion"
set -eu

export STAGE
STAGE=$(mktemp -d)
mkdir "$STAGE/tabellion"
# Stage tracked package files only, not rustlib/target or other build output.
git archive HEAD r/tabellion |
    tar -xf - -C "$STAGE/tabellion" --strip-components=2

python3 -I - <<'PY'
import os
import re
import shutil
from pathlib import Path

repo = Path.cwd()
src = Path(os.environ["STAGE"]) / "tabellion/src"
root_manifest = (repo / "Cargo.toml").read_text()
values = dict(re.findall(r"^([\w-]+) = (.+)$", root_manifest, re.M))
values["nebu-docx"] = '{ path = "../nebu-docx" }'

for name in ("nebu-docx", "tabellion-typst"):
    original = repo / "crates" / name
    dest = src / "rust/local" / name
    shutil.copytree(original / "src", dest / "src")
    manifest = (original / "Cargo.toml").read_text().split("[dev-dependencies]")[0]
    manifest = re.sub(
        r"^([\w-]+)\.workspace = true$",
        lambda m: f"{m[1]} = {values[m[1]]}",
        manifest,
        flags=re.M,
    )
    (dest / "Cargo.toml").write_text(manifest)

manifest = src / "rustlib/Cargo.toml"
manifest.write_text(
    manifest.read_text().split("[dev-dependencies]")[0]
    .replace("../../../../crates/", "../rust/local/")
)
shutil.copyfile(repo / "Cargo.lock", src / "rustlib/Cargo.lock")
PY

(
    cd "$STAGE/tabellion/src"
    # Reconcile the copied workspace lock against the standalone release graph.
    cargo metadata --format-version 1 --manifest-path rustlib/Cargo.toml >/dev/null
    cargo vendor --locked --manifest-path rustlib/Cargo.toml rust/vendor \
        > rust/vendor-config.toml
    tar -C rust -cJf rust/vendor.tar.xz vendor
    rm -rf rust/vendor
)
printf '%s\n' "$STAGE/tabellion"
