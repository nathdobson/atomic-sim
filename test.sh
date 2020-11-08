#!/bin/sh
set -e
cd tests
BUILD="cargo +nightly-2020-07-01-x86_64-apple-darwin rustc -Zbuild-std=core --target=x86_64-apple-darwin -- \
         -C opt-level=0 -C panic=abort -C overflow-checks=no"
$BUILD --emit=llvm-ir
$BUILD --emit=llvm-bc
cd ..
RUST_BACKTRACE=1 cargo run