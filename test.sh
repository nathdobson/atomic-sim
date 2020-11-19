#!/bin/sh
set -e
cd tests
function build {
  RUSTFLAGS="--emit $1" \
  cargo +nightly-2020-07-01-x86_64-apple-darwin \
       rustc -Zbuild-std=std,core,alloc \
             --target-dir=target-$1 \
             --target=x86_64-apple-darwin -- \
             -C opt-level=0 \
             -C overflow-checks=no
}
build llvm-ir
build llvm-bc
cd ..
RUST_BACKTRACE=1 cargo run