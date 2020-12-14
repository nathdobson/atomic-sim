#!/bin/sh
set -e
cd tests
function build {
  #OPT_LEVEL=-C opt-level=0
  OPT_LEVEL=
  RUSTFLAGS="--emit $1" \
  cargo +nightly-2020-07-01-x86_64-apple-darwin \
       rustc --release \
             -Zbuild-std=std,core,alloc \
             --target-dir=target-$1 \
             --target=x86_64-apple-darwin -- \
             $OPT_LEVEL \
             -C overflow-checks=yes
}
cargo +nightly-2020-07-01-x86_64-apple-darwin \
     rustc --release \
           -Zbuild-std=std,core,alloc \
           --target-dir=target-bin \
           --target=x86_64-apple-darwin -- \
           -C overflow-checks=yes

build llvm-ir
build llvm-bc
cd ..
RUST_BACKTRACE=1 cargo run "$@"
