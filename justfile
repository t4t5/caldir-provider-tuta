format:
  cargo fmt --package caldir-provider-tuta

check:
  cargo check -p caldir-provider-tuta
  cargo clippy -p caldir-provider-tuta --no-deps -- -D warnings

test:
  cargo test -p caldir-provider-tuta

install:
  cargo install --path .

vendor checkout:
  scripts/vendor.sh {{checkout}}

